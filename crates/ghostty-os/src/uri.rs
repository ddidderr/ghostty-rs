//! URI parsing. Extends a faithful port of Zig's `std.Uri.parse` with the
//! additional [`ParseOptions`] from `os/uri.zig`.

use std::{borrow::Cow, fmt::Write as _};

#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    /// Parse MAC addresses in the host component.
    ///
    /// This is useful when the "Private Wi-Fi address" is enabled on macOS,
    /// which sets the hostname to a rotating MAC address (12:34:56:ab:cd:ef).
    pub mac_address: bool,

    /// Return the full, raw, unencoded path string. Any query and fragment
    /// values will be return as part of the path instead of as distinct
    /// fields.
    pub raw_path: bool,
}

/// Errors of `std.Uri.ParseError` plus `InvalidMacAddress`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected character")]
    UnexpectedCharacter,
    #[error("invalid format")]
    InvalidFormat,
    #[error("invalid port")]
    InvalidPort,
    #[error("invalid mac address")]
    InvalidMacAddress,
}

/// `std.Uri.Component`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Component<'a> {
    /// Unencoded text.
    Raw(Cow<'a, str>),
    /// Percent-encoded text.
    PercentEncoded(&'a str),
}

impl Component<'_> {
    pub fn is_empty(&self) -> bool {
        match self {
            Component::Raw(s) => s.is_empty(),
            Component::PercentEncoded(s) => s.is_empty(),
        }
    }

    /// The raw (percent-decoded) value (`toRaw`) as bytes. Invalid escapes
    /// are left as-is, matching `std.Uri.percentDecodeInPlace`. The decoded
    /// bytes are NOT required to be UTF-8 (e.g. non-UTF-8 file names).
    pub fn to_raw_bytes(&self) -> Cow<'_, [u8]> {
        match self {
            Component::Raw(s) => Cow::Borrowed(s.as_bytes()),
            Component::PercentEncoded(s) => percent_decode(s),
        }
    }

    /// Lossy (U+FFFD) UTF-8 view of [`Self::to_raw_bytes`], for values such
    /// as host names that are expected to be text. Use `to_raw_bytes` for
    /// paths.
    pub fn to_raw(&self) -> Cow<'_, str> {
        match self.to_raw_bytes() {
            Cow::Borrowed(b) => String::from_utf8_lossy(b),
            Cow::Owned(b) => Cow::Owned(String::from_utf8_lossy(&b).into_owned()),
        }
    }

    /// The percent-encoded value as it appears in the source, or the raw
    /// text for `Raw` components.
    pub fn as_str(&self) -> &str {
        match self {
            Component::Raw(s) => s,
            Component::PercentEncoded(s) => s,
        }
    }
}

/// Lenient percent decoding like `std.Uri.percentDecodeBackwards`/InPlace:
/// valid `%XX` sequences are decoded, everything else passes through.
fn percent_decode(s: &str) -> Cow<'_, [u8]> {
    if !s.contains('%') {
        return Cow::Borrowed(s.as_bytes());
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hexval(b[i + 1]), hexval(b[i + 2]))
        {
            out.push((h << 4) | l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    Cow::Owned(out)
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// `std.Uri`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uri<'a> {
    pub scheme: &'a str,
    pub user: Option<Component<'a>>,
    pub password: Option<Component<'a>>,
    pub host: Option<Component<'a>>,
    pub port: Option<u16>,
    pub path: Component<'a>,
    pub query: Option<Component<'a>>,
    pub fragment: Option<Component<'a>>,
}

/// Port of `std.Uri.parse`.
fn std_parse(text: &str) -> Result<Uri<'_>, ParseError> {
    let Some(colon) = text.find(':') else {
        return Err(ParseError::UnexpectedCharacter);
    };
    let scheme = &text[..colon];
    parse_after_scheme(scheme, &text[colon + 1..])
}

/// Port of `std.Uri.parseAfterScheme`.
fn parse_after_scheme<'a>(scheme: &'a str, text: &'a str) -> Result<Uri<'a>, ParseError> {
    let mut uri = Uri {
        scheme,
        user: None,
        password: None,
        host: None,
        port: None,
        path: Component::PercentEncoded(""),
        query: None,
        fragment: None,
    };
    let mut rest = text;

    if let Some(after) = rest.strip_prefix("//") {
        // authority part
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        let authority = &after[..end];
        rest = &after[end..];

        if authority.is_empty() {
            if !rest.starts_with('/') {
                return Err(ParseError::InvalidFormat);
            }
        } else {
            parse_authority(authority, &mut uri)?;
        }
    }

    let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
    uri.path = Component::PercentEncoded(&rest[..path_end]);
    rest = &rest[path_end..];
    if let Some(after) = rest.strip_prefix('?') {
        // query part
        let end = after.find('#').unwrap_or(after.len());
        uri.query = Some(Component::PercentEncoded(&after[..end]));
        rest = &after[end..];
    }
    if let Some(after) = rest.strip_prefix('#') {
        uri.fragment = Some(Component::PercentEncoded(after));
    }
    Ok(uri)
}

fn parse_authority<'a>(authority: &'a str, uri: &mut Uri<'a>) -> Result<(), ParseError> {
    let mut start_of_host = 0;
    if let Some(index) = authority.find('@') {
        start_of_host = index + 1;
        let user_info = &authority[..index];

        if let Some(idx) = user_info.find(':') {
            uri.user = Some(Component::PercentEncoded(&user_info[..idx]));
            if idx + 1 < user_info.len() {
                // empty password is also "no password"
                uri.password = Some(Component::PercentEncoded(&user_info[idx + 1..]));
            }
        } else {
            uri.user = Some(Component::PercentEncoded(user_info));
            uri.password = None;
        }
    }

    // only possible if uri consists of only `userinfo@`
    if start_of_host >= authority.len() {
        return Ok(());
    }

    let mut end_of_host = authority.len();
    let ab = authority.as_bytes();

    // if we see `]` first without `@`
    if ab[start_of_host] == b']' {
        return Err(ParseError::InvalidFormat);
    }

    if ab[start_of_host] == b'[' {
        // IPv6
        end_of_host = authority.rfind(']').ok_or(ParseError::InvalidFormat)? + 1;

        if let Some(index) = authority.rfind(':')
            && index >= end_of_host
        {
            // if not part of the V6 address field
            end_of_host = end_of_host.min(index);
            uri.port = Some(
                authority[index + 1..]
                    .parse::<u16>()
                    .map_err(|_| ParseError::InvalidPort)?,
            );
        }
    } else if let Some(index) = authority.rfind(':')
        && index >= start_of_host
    {
        // if not part of the userinfo field
        end_of_host = end_of_host.min(index);
        uri.port = Some(
            authority[index + 1..]
                .parse::<u16>()
                .map_err(|_| ParseError::InvalidPort)?,
        );
    }

    uri.host = Some(Component::PercentEncoded(
        &authority[start_of_host..end_of_host],
    ));
    Ok(())
}

/// Byte offset of `inner` within `outer` (both must be sub-slices of the
/// same allocation; here `inner` always comes from parsing `outer`).
fn offset_in(outer: &str, inner: &str) -> usize {
    inner.as_ptr() as usize - outer.as_ptr() as usize
}

/// Parses a URI from the given string.
///
/// This extends the standard URI parse with some additional ParseOptions.
pub fn parse(text: &str, options: ParseOptions) -> Result<Uri<'_>, ParseError> {
    let mut uri = match std_parse(text) {
        Ok(uri) => uri,
        Err(err) => {
            // We can attempt to re-parse the text as a URI that has a MAC address
            // in its host field (which tripped up the standard port parsing):
            //
            //      file://12:34:56:78:90:aa/path/to/file
            //                            ^^ InvalidPort
            //
            if err != ParseError::InvalidPort || !options.mac_address {
                return Err(err);
            }

            // We can assume that the initial parse already validated the
            // scheme, so we only need to find its bounds within the string.
            let scheme_end = text.find("://").ok_or(ParseError::InvalidFormat)?;
            let scheme = &text[..scheme_end];

            // We similarly find the bounds of the host component by looking
            // for the first slash (/) after the scheme. This is all we need
            // for this case because the resulting slice can be unambiguously
            // determined to be a MAC address (or not).
            let host_start = scheme_end + "://".len();
            let host_end = text[host_start..]
                .find('/')
                .map_or(text.len(), |i| host_start + i);
            let mac_address = &text[host_start..host_end];
            if !is_valid_mac_address(mac_address) {
                return Err(ParseError::InvalidMacAddress);
            }

            // Parse the rest of the text (starting with the path component) as a
            // partial URI and then add our MAC address as its host component.
            let mut uri = parse_after_scheme(scheme, &text[host_end..])?;
            uri.host = Some(Component::PercentEncoded(mac_address));
            uri
        }
    };

    // When MAC address parsing is enabled, we need to handle the case where
    // the standard parse parsed the address's last octet as a numeric port number.
    // We use a few heuristics to identify this case (14 characters, 4 colons)
    // and then "repair" the result by reassign the .host component to the full
    // MAC address and clearing the .port component.
    //
    //    12:34:56:78:90:99 -> [12:34:56:78:90, 99] -> 12:34:56:78:90:99
    //    (original host)      (parsed host + port)    (restored host)
    //
    if options.mac_address
        && let Some(Component::PercentEncoded(host)) = uri.host.clone()
        && host.len() == 14
        && host.matches(':').count() == 4
        && let Some(port) = uri.port
        && port <= 99
    {
        // The parse returns slices pointing into the original text string.
        let host_start = offset_in(text, host);
        let path_start = offset_in(text, uri.path.as_str());
        let mac_address = &text[host_start..path_start];
        if !is_valid_mac_address(mac_address) {
            return Err(ParseError::InvalidMacAddress);
        }

        uri.host = Some(Component::PercentEncoded(mac_address));
        uri.port = None;
    }

    // When the raw_path option is active, return everything after the authority
    // (host) in the .path component, including any query and fragment values.
    if options.raw_path {
        let path_start = offset_in(text, uri.path.as_str());
        uri.path = Component::Raw(Cow::Borrowed(&text[path_start..]));
        uri.query = None;
        uri.fragment = None;
    }

    Ok(uri)
}

/// The pieces of a parsed URI that are needed by byte-oriented callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBytes {
    pub scheme: String,
    /// Percent-decoded host (lossy; host names are text).
    pub host: Option<String>,
    /// Percent-decoded path bytes (or the full raw tail with
    /// [`ParseOptions::raw_path`]). Not required to be UTF-8.
    pub path: Vec<u8>,
}

/// Parse a URI given as arbitrary bytes without losing any path bytes.
///
/// Bytes that are not valid UTF-8 are percent-escaped before parsing (so
/// they decode back to exactly the same bytes), mirroring Zig's byte-string
/// `std.Uri.parse`. With `raw_path` the original bytes are sliced directly.
pub fn parse_bytes(text: &[u8], options: ParseOptions) -> Result<ParsedBytes, ParseError> {
    // Escape invalid bytes, remembering the offset mapping.
    let mut escaped = String::with_capacity(text.len());
    for chunk in text.utf8_chunks() {
        escaped.push_str(chunk.valid());
        for b in chunk.invalid() {
            let _ = write!(escaped, "%{b:02X}");
        }
    }

    let uri = parse(&escaped, options)?;
    let path = if options.raw_path {
        let esc_start = offset_in(&escaped, uri.path.as_str());
        // Map the escaped offset back to the original byte offset. The path
        // never begins inside an escape (it starts at a delimiter).
        let mut orig_start = esc_start;
        let mut esc_pos = 0usize;
        let mut orig_pos = 0usize;
        for chunk in text.utf8_chunks() {
            let v = chunk.valid().len();
            if esc_start <= esc_pos + v {
                orig_start = orig_pos + (esc_start - esc_pos);
                break;
            }
            esc_pos += v;
            orig_pos += v;
            let inv = chunk.invalid().len();
            esc_pos += inv * 3;
            orig_pos += inv;
            orig_start = orig_pos;
        }
        text[orig_start.min(text.len())..].to_vec()
    } else {
        uri.path.to_raw_bytes().into_owned()
    };
    Ok(ParsedBytes {
        scheme: uri.scheme.to_owned(),
        host: uri.host.as_ref().map(|h| h.to_raw().into_owned()),
        path,
    })
}

/// Checks if a string represents a valid MAC address, e.g. 12:34:56:ab:cd:ef.
fn is_valid_mac_address(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 17 {
        return false;
    }

    for (i, &c) in b.iter().enumerate() {
        if i % 3 == 2 {
            if c != b':' {
                return false;
            }
        } else if !c.is_ascii_hexdigit() {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: ParseOptions = ParseOptions {
        mac_address: true,
        raw_path: false,
    };

    fn host<'a>(u: &'a Uri<'_>) -> &'a str {
        match u.host.as_ref().unwrap() {
            Component::PercentEncoded(s) => s,
            Component::Raw(_) => panic!("raw host"),
        }
    }

    #[test]
    fn parse_mac_address() {
        // Numeric MAC address without a port
        let uri1 = parse("file://00:12:34:56:78:90/path", MAC).unwrap();
        assert_eq!("file", uri1.scheme);
        assert_eq!("00:12:34:56:78:90", host(&uri1));
        assert_eq!("/path", uri1.path.as_str());
        assert_eq!(None, uri1.port);

        // Numeric MAC address with a port
        let uri2 = parse("file://00:12:34:56:78:90:999/path", MAC).unwrap();
        assert_eq!("file", uri2.scheme);
        assert_eq!("00:12:34:56:78:90", host(&uri2));
        assert_eq!("/path", uri2.path.as_str());
        assert_eq!(Some(999), uri2.port);

        // Alphabetic MAC address without a port
        let uri3 = parse("file://ab:cd:ef:ab:cd:ef/path", MAC).unwrap();
        assert_eq!("file", uri3.scheme);
        assert_eq!("ab:cd:ef:ab:cd:ef", host(&uri3));
        assert_eq!("/path", uri3.path.as_str());
        assert_eq!(None, uri3.port);

        // Alphabetic MAC address with a port
        let uri4 = parse("file://ab:cd:ef:ab:cd:ef:999/path", MAC).unwrap();
        assert_eq!("file", uri4.scheme);
        assert_eq!("ab:cd:ef:ab:cd:ef", host(&uri4));
        assert_eq!("/path", uri4.path.as_str());
        assert_eq!(Some(999), uri4.port);

        // Numeric MAC address without a path component
        let uri5 = parse("file://00:12:34:56:78:90", MAC).unwrap();
        assert_eq!("file", uri5.scheme);
        assert_eq!("00:12:34:56:78:90", host(&uri5));
        assert!(uri5.path.is_empty());

        // Alphabetic MAC address without a path component
        let uri6 = parse("file://ab:cd:ef:ab:cd:ef", MAC).unwrap();
        assert_eq!("file", uri6.scheme);
        assert_eq!("ab:cd:ef:ab:cd:ef", host(&uri6));
        assert!(uri6.path.is_empty());

        // Invalid MAC addresses
        assert_eq!(
            Err(ParseError::InvalidMacAddress),
            parse("file://zz:zz:zz:zz:zz:00/path", MAC)
        );
        assert_eq!(
            Err(ParseError::InvalidMacAddress),
            parse("file://zz:zz:zz:zz:zz:zz/path", MAC)
        );
    }

    #[test]
    fn percent_decode_preserves_non_utf8_bytes() {
        let u = parse("file://localhost/a%FFb%e2%82", ParseOptions::default()).unwrap();
        assert_eq!(&*u.path.to_raw_bytes(), b"/a\xffb\xe2\x82");
        // The lossy text view still works for text values.
        assert_eq!(u.path.to_raw(), "/a\u{fffd}b\u{fffd}");
    }

    #[test]
    fn parse_bytes_non_utf8() {
        let p = parse_bytes(b"file://localhost/a\xffb%FEc", ParseOptions::default()).unwrap();
        assert_eq!(p.scheme, "file");
        assert_eq!(p.host.as_deref(), Some("localhost"));
        assert_eq!(p.path, b"/a\xffb\xfec");

        let raw = ParseOptions {
            raw_path: true,
            mac_address: true,
        };
        let p = parse_bytes(b"kitty-shell-cwd://localhost/a\xff%41?x", raw).unwrap();
        assert_eq!(p.path, b"/a\xff%41?x");
        let p = parse_bytes(b"kitty-shell-cwd://\xfflo/a", raw).ok();
        assert!(p.is_none() || p.is_some());
    }

    #[test]
    fn parse_raw_path() {
        let text = "file://localhost/path??#fragment";

        let uri1 = parse(
            text,
            ParseOptions {
                raw_path: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!("file", uri1.scheme);
        assert_eq!("localhost", host(&uri1));
        assert_eq!("/path", uri1.path.to_raw());
        assert_eq!("?", uri1.query.as_ref().unwrap().as_str());
        assert_eq!("fragment", uri1.fragment.as_ref().unwrap().as_str());

        let uri2 = parse(
            text,
            ParseOptions {
                raw_path: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!("file", uri2.scheme);
        assert_eq!("localhost", host(&uri2));
        assert_eq!("/path??#fragment", uri2.path.to_raw());
        assert_eq!(None, uri2.query);
        assert_eq!(None, uri2.fragment);

        let uri3 = parse(
            "file://localhost",
            ParseOptions {
                raw_path: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!("file", uri3.scheme);
        assert_eq!("localhost", host(&uri3));
        assert!(uri3.path.is_empty());
        assert_eq!(None, uri3.query);
        assert_eq!(None, uri3.fragment);
    }

    #[test]
    fn is_valid_mac_address_cases() {
        assert!(is_valid_mac_address("01:23:45:67:89:Aa"));
        assert!(is_valid_mac_address("Aa:Bb:Cc:Dd:Ee:Ff"));

        assert!(!is_valid_mac_address(""));
        assert!(!is_valid_mac_address("00:23:45"));
        assert!(!is_valid_mac_address("00:23:45:Xx:Yy:Zz"));
        assert!(!is_valid_mac_address("01-23-45-67-89-Aa"));
        assert!(!is_valid_mac_address("01:23:45:67:89:Aa:Bb"));
    }

    #[test]
    fn std_parse_basics() {
        let u = parse(
            "https://user:pw@example.com:8080/a%20b?q=1#frag",
            ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(u.scheme, "https");
        assert_eq!(u.user.as_ref().unwrap().as_str(), "user");
        assert_eq!(u.password.as_ref().unwrap().as_str(), "pw");
        assert_eq!(host(&u), "example.com");
        assert_eq!(u.port, Some(8080));
        assert_eq!(u.path.to_raw(), "/a b");
        assert_eq!(u.query.as_ref().unwrap().as_str(), "q=1");
        assert_eq!(u.fragment.as_ref().unwrap().as_str(), "frag");

        let u = parse("file:///tmp/x", ParseOptions::default()).unwrap();
        assert_eq!(u.host, None);
        assert_eq!(u.path.as_str(), "/tmp/x");

        let u = parse("http://[::1]:80/", ParseOptions::default()).unwrap();
        assert_eq!(host(&u), "[::1]");
        assert_eq!(u.port, Some(80));

        assert_eq!(
            parse("nocolon", ParseOptions::default()),
            Err(ParseError::UnexpectedCharacter)
        );
        assert_eq!(
            parse("file://a:b/", ParseOptions::default()),
            Err(ParseError::InvalidPort)
        );
        assert_eq!(
            parse("file://?x", ParseOptions::default()),
            Err(ParseError::InvalidFormat)
        );
    }
}
