//! Kitty's clipboard protocol (OSC 5522)
//! Specification: https://sw.kovidgoyal.net/kitty/clipboard/
//! https://rockorager.dev/misc/bracketed-paste-mime/
//! (ZIG/src/terminal/osc/parsers/kitty_clipboard_protocol.zig)

use crate::osc::{Command, Parser, State, Terminator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OSC {
    /// The raw metadata that was received. It can be parsed by using the
    /// `read_option` method (or the typed helpers).
    pub metadata: Vec<u8>,
    /// The raw payload. It may be Base64 encoded, check the `e` option.
    pub payload: Option<Vec<u8>>,
    /// The terminator that was used in case we need to send a response.
    pub terminator: Terminator,
}

/// Value of a decoded option (Zig `Option.Type()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionValue<'a> {
    Str(&'a [u8]),
    Loc(Location),
    Status(Status),
    Type(Operation),
}

impl OSC {
    /// Decode an option from the metadata.
    pub fn read_option(&self, key: Opt) -> Option<OptionValue<'_>> {
        key.read(&self.metadata)
    }

    pub fn id(&self) -> Option<&[u8]> {
        match self.read_option(Opt::Id)? {
            OptionValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn loc(&self) -> Option<Location> {
        match self.read_option(Opt::Loc)? {
            OptionValue::Loc(v) => Some(v),
            _ => None,
        }
    }

    pub fn mime(&self) -> Option<&[u8]> {
        match self.read_option(Opt::Mime)? {
            OptionValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn name(&self) -> Option<&[u8]> {
        match self.read_option(Opt::Name)? {
            OptionValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn password(&self) -> Option<&[u8]> {
        match self.read_option(Opt::Password)? {
            OptionValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn pw(&self) -> Option<&[u8]> {
        match self.read_option(Opt::Pw)? {
            OptionValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn status(&self) -> Option<Status> {
        match self.read_option(Opt::Status)? {
            OptionValue::Status(v) => Some(v),
            _ => None,
        }
    }

    pub fn type_(&self) -> Option<Operation> {
        match self.read_option(Opt::Type)? {
            OptionValue::Type(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Primary,
}

impl Location {
    pub fn init(s: &[u8]) -> Option<Location> {
        match s {
            b"primary" => Some(Location::Primary),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Read,
    Walias,
    Wdata,
    Write,
}

impl Operation {
    pub fn init(s: &[u8]) -> Option<Operation> {
        match s {
            b"read" => Some(Operation::Read),
            b"walias" => Some(Operation::Walias),
            b"wdata" => Some(Operation::Wdata),
            b"write" => Some(Operation::Write),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum Status {
    DATA,
    DONE,
    EBUSY,
    EINVAL,
    EIO,
    ENOSYS,
    EPERM,
    OK,
}

impl Status {
    pub fn init(s: &[u8]) -> Option<Status> {
        match s {
            b"DATA" => Some(Status::DATA),
            b"DONE" => Some(Status::DONE),
            b"EBUSY" => Some(Status::EBUSY),
            b"EINVAL" => Some(Status::EINVAL),
            b"EIO" => Some(Status::EIO),
            b"ENOSYS" => Some(Status::ENOSYS),
            b"EPERM" => Some(Status::EPERM),
            b"OK" => Some(Status::OK),
            _ => None,
        }
    }
}

/// Option keys (Zig `Option`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opt {
    Id,
    Loc,
    Mime,
    Name,
    Password,
    Pw,
    Status,
    Type,
}

/// Zig `std.ascii.isWhitespace`.
fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

fn trim_ws(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if is_ws(*first) {
            s = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = s {
        if is_ws(*last) {
            s = rest;
        } else {
            break;
        }
    }
    s
}

fn index_of_from(hay: &[u8], from: usize, needle: u8) -> Option<usize> {
    hay[from..]
        .iter()
        .position(|&c| c == needle)
        .map(|i| i + from)
}

impl Opt {
    pub fn tag_name(self) -> &'static str {
        match self {
            Opt::Id => "id",
            Opt::Loc => "loc",
            Opt::Mime => "mime",
            Opt::Name => "name",
            Opt::Password => "password",
            Opt::Pw => "pw",
            Opt::Status => "status",
            Opt::Type => "type",
        }
    }

    /// Read the option value from the raw metadata string.
    pub fn read(self, metadata: &[u8]) -> Option<OptionValue<'_>> {
        let tag = self.tag_name().as_bytes();
        let value: &[u8] = 'value: {
            let mut pos: usize = 0;
            while pos < metadata.len() {
                // skip any whitespace
                while pos < metadata.len() && is_ws(metadata[pos]) {
                    pos += 1;
                }
                // bail if we are out of metadata
                if pos >= metadata.len() {
                    return None;
                }
                if !metadata[pos..].starts_with(tag) {
                    // this isn't the key we are looking for, skip to the next option, or bail if
                    // there is no next option
                    pos = index_of_from(metadata, pos, b':')?;
                    pos += 1;
                    continue;
                }
                // skip past the key
                pos += tag.len();
                // skip any whitespace
                while pos < metadata.len() && is_ws(metadata[pos]) {
                    pos += 1;
                }
                // bail if we are out of metadata
                if pos >= metadata.len() {
                    return None;
                }
                // a valid option has an '='
                if metadata[pos] != b'=' {
                    return None;
                }
                // the end of the value is bounded by a ':' or the end of the metadata
                let end = index_of_from(metadata, pos, b':').unwrap_or(metadata.len());
                let start = pos + 1;
                // strip any leading or trailing whitespace
                break 'value trim_ws(&metadata[start..end]);
            }
            // the key was not found
            return None;
        };

        // return the parsed value
        match self {
            Opt::Id => parse_identifier(value).map(OptionValue::Str),
            Opt::Loc => Location::init(value).map(OptionValue::Loc),
            Opt::Mime | Opt::Name | Opt::Password | Opt::Pw => Some(OptionValue::Str(value)),
            Opt::Status => Status::init(value).map(OptionValue::Status),
            Opt::Type => Operation::init(value).map(OptionValue::Type),
        }
    }
}

/// Characters that are valid in identifiers.
const VALID_IDENTIFIER_CHARACTERS: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_+.";

fn is_valid_identifier(s: &[u8]) -> bool {
    if s.is_empty() {
        return false;
    }
    s.iter().all(|c| VALID_IDENTIFIER_CHARACTERS.contains(c))
}

fn parse_identifier(s: &[u8]) -> Option<&[u8]> {
    if is_valid_identifier(s) {
        Some(s)
    } else {
        None
    }
}

pub fn parse(parser: &mut Parser, terminator_ch: Option<u8>) -> Option<Command> {
    debug_assert_eq!(parser.state, State::S5522);

    let data = parser.buffered()?;

    let (metadata, payload) = match data.iter().position(|&c| c == b';') {
        None => (data, None),
        Some(start) => (data[..start].to_vec(), Some(data[start + 1..].to_vec())),
    };

    Some(Command::KittyClipboardProtocol(OSC {
        metadata,
        payload,
        terminator: Terminator::init(terminator_ch),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(input: &str) -> OSC {
        let mut p = Parser::init(true);
        for &ch in input.as_bytes() {
            p.next(ch);
        }
        match p.end(Some(0x1b)) {
            Some(Command::KittyClipboardProtocol(o)) => o,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[allow(clippy::option_option)]
    /// Expected values for every option; `None` means "must be null".
    #[derive(Default)]
    struct Exp<'a> {
        metadata: Option<&'a str>,
        payload: Option<Option<&'a str>>,
        id: Option<&'a str>,
        loc: Option<Location>,
        mime: Option<&'a str>,
        name: Option<&'a str>,
        password: Option<&'a str>,
        pw: Option<&'a str>,
        status: Option<Status>,
        type_: Option<Operation>,
    }

    fn check(c: &OSC, e: &Exp) {
        if let Some(m) = e.metadata {
            assert_eq!(m.as_bytes(), &c.metadata[..]);
        }
        if let Some(p) = e.payload {
            assert_eq!(p.map(|s| s.as_bytes().to_vec()), c.payload);
        }
        assert_eq!(e.id.map(str::as_bytes), c.id());
        assert_eq!(e.loc, c.loc());
        assert_eq!(e.mime.map(str::as_bytes), c.mime());
        assert_eq!(e.name.map(str::as_bytes), c.name());
        assert_eq!(e.password.map(str::as_bytes), c.password());
        assert_eq!(e.pw.map(str::as_bytes), c.pw());
        assert_eq!(e.status, c.status());
        assert_eq!(e.type_, c.type_());
    }

    #[test]
    fn osc_5522_empty_metadata_and_missing_payload() {
        let c = run("5522;");
        check(
            &c,
            &Exp {
                metadata: Some(""),
                payload: Some(None),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_empty_metadata_and_empty_payload() {
        let c = run("5522;;");
        check(
            &c,
            &Exp {
                metadata: Some(""),
                payload: Some(Some("")),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_non_empty_metadata_and_payload() {
        let c = run("5522;type=read;dGV4dC9wbGFpbg==");
        check(
            &c,
            &Exp {
                metadata: Some("type=read"),
                payload: Some(Some("dGV4dC9wbGFpbg==")),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_empty_id() {
        let c = run("5522;id=");
        assert!(c.id().is_none());
    }

    #[test]
    fn osc_5522_valid_id() {
        let c = run("5522;id=5c076ad9-d36f-4705-847b-d4dbf356cc0d");
        assert_eq!(b"5c076ad9-d36f-4705-847b-d4dbf356cc0d", c.id().unwrap());
    }

    #[test]
    fn osc_5522_invalid_id() {
        let c = run("5522;id=*42*");
        assert!(c.id().is_none());
    }

    #[test]
    fn osc_5522_invalid_status() {
        let c = run("5522;status=BOBR");
        assert!(c.status().is_none());
    }

    #[test]
    fn osc_5522_valid_status() {
        let c = run("5522;status=DONE");
        assert_eq!(Some(Status::DONE), c.status());
    }

    #[test]
    fn osc_5522_invalid_location() {
        let c = run("5522;loc=bobr");
        assert!(c.loc().is_none());
    }

    #[test]
    fn osc_5522_valid_location() {
        let c = run("5522;loc=primary");
        assert_eq!(Some(Location::Primary), c.loc());
    }

    #[test]
    fn osc_5522_password_1() {
        let c = run("5522;pw=R2hvc3R0eQ==:name=Qk9CUiBLVVJXQQ==");
        assert_eq!(b"R2hvc3R0eQ==", c.pw().unwrap());
        assert_eq!(b"Qk9CUiBLVVJXQQ==", c.name().unwrap());
    }

    #[test]
    fn osc_5522_password_2() {
        let c = run("5522;password=R2hvc3R0eQ==");
        assert_eq!(b"R2hvc3R0eQ==", c.password().unwrap());
    }

    #[test]
    fn osc_5522_example_1() {
        let c = run("5522;type=read:status=OK");
        check(
            &c,
            &Exp {
                payload: Some(None),
                status: Some(Status::OK),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_2() {
        let c = run("5522;type=read:mime=dGV4dC9wbGFpbg==;R2hvc3R0eQ==");
        check(
            &c,
            &Exp {
                payload: Some(Some("R2hvc3R0eQ==")),
                mime: Some("dGV4dC9wbGFpbg=="),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_3() {
        let c = run("5522;type=read:status=OK");
        check(
            &c,
            &Exp {
                payload: Some(None),
                status: Some(Status::OK),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_4() {
        let c = run("5522;type=write");
        check(
            &c,
            &Exp {
                payload: Some(None),
                type_: Some(Operation::Write),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_5() {
        let c = run("5522;type=wdata:mime=dGV4dC9wbGFpbg==;R2hvc3R0eQ==");
        check(
            &c,
            &Exp {
                payload: Some(Some("R2hvc3R0eQ==")),
                mime: Some("dGV4dC9wbGFpbg=="),
                type_: Some(Operation::Wdata),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_6() {
        let c = run("5522;type=wdata");
        check(
            &c,
            &Exp {
                payload: Some(None),
                type_: Some(Operation::Wdata),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_7() {
        let c = run("5522;type=write:status=DONE");
        check(
            &c,
            &Exp {
                payload: Some(None),
                status: Some(Status::DONE),
                type_: Some(Operation::Write),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_8() {
        let c = run("5522;type=write:status=EPERM");
        check(
            &c,
            &Exp {
                payload: Some(None),
                status: Some(Status::EPERM),
                type_: Some(Operation::Write),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_9() {
        let c = run("5522;type=walias:mime=dGV4dC9wbGFpbg==;dGV4dC9odG1sIGFwcGxpY2F0aW9uL2pzb24=");
        check(
            &c,
            &Exp {
                payload: Some(Some("dGV4dC9odG1sIGFwcGxpY2F0aW9uL2pzb24=")),
                mime: Some("dGV4dC9wbGFpbg=="),
                type_: Some(Operation::Walias),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_10() {
        let c = run("5522;type=read:status=OK:password=Qk9CUiBLVVJXQQ==");
        check(
            &c,
            &Exp {
                payload: Some(None),
                password: Some("Qk9CUiBLVVJXQQ=="),
                status: Some(Status::OK),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_11() {
        let c = run("5522;type=read:status=DATA:mime=dGV4dC9wbGFpbg==");
        check(
            &c,
            &Exp {
                payload: Some(None),
                mime: Some("dGV4dC9wbGFpbg=="),
                status: Some(Status::DATA),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_12() {
        let c = run("5522;type=read:mime=dGV4dC9wbGFpbg==:password=Qk9CUiBLVVJXQQ==");
        check(
            &c,
            &Exp {
                payload: Some(None),
                mime: Some("dGV4dC9wbGFpbg=="),
                password: Some("Qk9CUiBLVVJXQQ=="),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_13() {
        let c = run("5522;type=read:status=OK");
        check(
            &c,
            &Exp {
                payload: Some(None),
                status: Some(Status::OK),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_14() {
        let c = run("5522;type=read:status=DATA:mime=dGV4dC9wbGFpbg==;Qk9CUiBLVVJXQQ==");
        check(
            &c,
            &Exp {
                payload: Some(Some("Qk9CUiBLVVJXQQ==")),
                mime: Some("dGV4dC9wbGFpbg=="),
                status: Some(Status::DATA),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }

    #[test]
    fn osc_5522_example_15() {
        let c = run("5522;type=read:status=OK");
        check(
            &c,
            &Exp {
                payload: Some(None),
                status: Some(Status::OK),
                type_: Some(Operation::Read),
                ..Default::default()
            },
        );
    }
}
