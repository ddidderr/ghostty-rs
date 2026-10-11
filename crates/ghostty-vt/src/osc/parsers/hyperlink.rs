//! OSC 8 hyperlinks (ZIG/src/terminal/osc/parsers/hyperlink.zig).
use crate::osc::{Command, Parser, State};

/// Parse OSC 8 hyperlinks
pub fn parse(parser: &mut Parser, _terminator_ch: Option<u8>) -> Option<Command> {
    let data = parser.data()?;
    let Some(s) = data.iter().position(|&b| b == b';') else {
        parser.state = State::Invalid;
        return None;
    };

    let uri = data[s + 1..].to_vec();
    let mut id: Option<Vec<u8>> = None;

    // The params section is `key=value` pairs separated by ':'. This follows
    // the Zig implementation byte for byte: ':' and the ';' terminator are
    // turned into NULs and the segments are walked by searching for the next
    // NUL starting one past the segment start.
    let mut kvs = data[..s].to_vec();
    kvs.push(0);
    for b in &mut kvs {
        if *b == b':' {
            *b = 0;
        }
    }
    let mut kv_start = 0;
    while kv_start < kvs.len() {
        let Some(kv_end) = kvs
            .iter()
            .skip(kv_start + 1)
            .position(|&b| b == 0)
            .map(|i| i + kv_start + 1)
        else {
            break;
        };
        let kv = &kvs[kv_start..kv_end];
        let Some(v) = kv.iter().position(|&b| b == b'=') else {
            break;
        };
        let key = &kv[..v];
        let value = &kv[v + 1..];
        if key == b"id" {
            if !value.is_empty() {
                id = Some(value.to_vec());
            }
        } else {
            log::warn!(
                "unknown hyperlink option: '{}'",
                String::from_utf8_lossy(key)
            );
        }
        kv_start = kv_end + 1;
    }

    if uri.is_empty() {
        if id.is_some() {
            parser.state = State::Invalid;
            return None;
        }
        return Some(Command::HyperlinkEnd);
    }

    Some(Command::HyperlinkStart { id, uri })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn run(input: &[u8]) -> Option<Command> {
        let mut p = Parser::init(false);
        for c in input {
            p.next(*c);
        }
        p.end(Some(0x1b))
    }

    fn start(id: Option<&str>, uri: &str) -> Command {
        Command::HyperlinkStart {
            id: id.map(|s| s.as_bytes().to_vec()),
            uri: uri.as_bytes().to_vec(),
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
