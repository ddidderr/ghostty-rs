//! OSC 8 hyperlinks (ZIG/src/terminal/osc/parsers/hyperlink.zig).
use crate::osc::{ByteStorage, Command, Parser, State};

/// Parse OSC 8 hyperlinks
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<Command<'_>> {
    parser.data()?;

    // Work out the layout with a short-lived borrow of the payload; the
    // command is built from a fresh borrow at the end (so that marking the
    // parser invalid above does not conflict with the returned borrow).
    let (s, id) = {
        let data = parser.payload();
        let Some(s) = data.iter().position(|&b| b == b';') else {
            parser.state = State::Invalid;
            return None;
        };

        let mut id: Option<(usize, usize)> = None;

        // The params section is `key=value` pairs separated by ':'. This
        // follows the Zig implementation byte for byte: ':' and the ';'
        // terminator are turned into NULs and the segments are walked by
        // searching for the next NUL starting one past the segment start. The
        // NUL-converted copy Zig makes in place is represented by `is_nul`
        // instead (no allocation).
        let is_nul = |i: usize| i == s || data[i] == b':' || data[i] == 0;
        let mut kv_start = 0;
        while kv_start <= s {
            let Some(kv_end) = (kv_start + 1..=s).find(|&i| is_nul(i)) else {
                break;
            };
            let kv = &data[kv_start..kv_end];
            let Some(v) = kv.iter().position(|&b| b == b'=') else {
                break;
            };
            let key = &kv[..v];
            if key == b"id" {
                if v + 1 < kv.len() {
                    id = Some((kv_start + v + 1, kv_end));
                }
            } else {
                log::warn!(
                    "unknown hyperlink option: '{}'",
                    String::from_utf8_lossy(key)
                );
            }
            kv_start = kv_end + 1;
        }
        (s, id)
    };

    let data = parser.payload();
    let uri = &data[s + 1..];
    if uri.is_empty() {
        if id.is_some() {
            parser.state = State::Invalid;
            return None;
        }
        return Some(Command::HyperlinkEnd);
    }

    let data = parser.payload();
    Some(Command::HyperlinkStart {
        id: id.map(|(a, b)| data[a..b].into()),
        uri: data[s + 1..].into(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn run(input: &[u8]) -> Option<Command<'_>> {
        let mut p = Parser::init(false);
        for c in input {
            p.next(*c);
        }
        p.end(Some(0x1b)).map(Command::into_owned)
    }

    fn start(id: Option<&str>, uri: &str) -> Command<'static> {
        Command::HyperlinkStart {
            id: id.map(|s| s.as_bytes().to_vec().into()),
            uri: uri.as_bytes().to_vec().into(),
        }
    }

    #[test]
    fn osc_8_hyperlink() {
        assert_eq!(
            run(b"8;;http://example.com").unwrap(),
            start(None, "http://example.com")
        );
    }

    #[test]
    fn osc_8_hyperlink_with_id_set() {
        assert_eq!(
            run(b"8;id=foo;http://example.com").unwrap(),
            start(Some("foo"), "http://example.com")
        );
    }

    #[test]
    fn osc_8_hyperlink_with_empty_id() {
        assert_eq!(
            run(b"8;id=;http://example.com").unwrap(),
            start(None, "http://example.com")
        );
    }

    #[test]
    fn osc_8_hyperlink_with_incomplete_key() {
        assert_eq!(
            run(b"8;id;http://example.com").unwrap(),
            start(None, "http://example.com")
        );
    }

    #[test]
    fn osc_8_hyperlink_with_empty_key() {
        assert_eq!(
            run(b"8;=value;http://example.com").unwrap(),
            start(None, "http://example.com")
        );
    }

    #[test]
    fn osc_8_hyperlink_with_empty_key_and_id() {
        assert_eq!(
            run(b"8;=value:id=foo;http://example.com").unwrap(),
            start(Some("foo"), "http://example.com")
        );
    }

    #[test]
    fn osc_8_hyperlink_with_empty_uri() {
        assert!(run(b"8;id=foo;").is_none());
    }

    #[test]
    fn osc_8_hyperlink_end() {
        assert_eq!(run(b"8;;").unwrap(), Command::HyperlinkEnd);
    }
}
