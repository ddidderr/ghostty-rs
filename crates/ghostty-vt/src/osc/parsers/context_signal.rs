//! OSC 3008: Hierarchical Context Signalling (UAPI spec)
//! Specification: https://uapi-group.org/specifications/specs/osc_context/
//! (ZIG/src/terminal/osc/parsers/context_signal.zig)
//!
//! OSC 3008 allows programs to signal context changes to the terminal emulator.
//! Each context has an identifier and metadata fields. Contexts are hierarchical
//! and form a stack.

use crate::osc::{ByteStorage, Command as OscCommand, Parser, State};

/// Maximum length of a context identifier (per spec).
const MAX_CONTEXT_ID_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// OSC 3008;start=<id> - initiates, updates, or returns to a context.
    Start,
    /// OSC 3008;end=<id> - terminates a context.
    End,
}

/// A single OSC 3008 context signal command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command<'a> {
    pub action: Action,
    /// The context identifier. Must be 1-64 characters in the 32..126 byte range.
    pub id: crate::osc::Bytes<'a>,
    /// Raw unparsed metadata fields after the context ID.
    /// Fields are semicolon-separated key=value pairs.
    /// Parsed lazily via `read_option`.
    pub metadata: crate::osc::Bytes<'a>,
}

/// Value of a decoded metadata field (Zig `Field.Type()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldValue<'a> {
    Type(ContextType),
    Exit(ExitStatus),
    U64(u64),
    Str(&'a [u8]),
}

impl Command<'_> {
    pub fn into_owned(self) -> Command<'static> {
        Command {
            action: self.action,
            id: super::super::own(self.id),
            metadata: super::super::own(self.metadata),
        }
    }

    /// Read a metadata field value from the raw fields string.
    /// Returns None if the field is not present or malformed.
    pub fn read_option(&self, option: Field) -> Option<FieldValue<'_>> {
        option.read(&self.metadata)
    }

    pub fn context_type(&self) -> Option<ContextType> {
        match self.read_option(Field::Type)? {
            FieldValue::Type(v) => Some(v),
            _ => None,
        }
    }

    pub fn exit(&self) -> Option<ExitStatus> {
        match self.read_option(Field::Exit)? {
            FieldValue::Exit(v) => Some(v),
            _ => None,
        }
    }

    /// For the numeric fields (`pid`, `pidfdid`, `status`).
    pub fn read_u64(&self, option: Field) -> Option<u64> {
        match self.read_option(option)? {
            FieldValue::U64(v) => Some(v),
            _ => None,
        }
    }

    /// For the string fields.
    pub fn read_str(&self, option: Field) -> Option<&[u8]> {
        match self.read_option(option)? {
            FieldValue::Str(v) => Some(v),
            _ => None,
        }
    }
}

/// Context types defined by the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextType {
    Boot,
    Container,
    Vm,
    Elevate,
    Chpriv,
    Subcontext,
    Remote,
    Shell,
    Command,
    App,
    Service,
    Session,
}

impl ContextType {
    pub fn parse(value: &[u8]) -> Option<ContextType> {
        Some(match value {
            b"boot" => ContextType::Boot,
            b"container" => ContextType::Container,
            b"vm" => ContextType::Vm,
            b"elevate" => ContextType::Elevate,
            b"chpriv" => ContextType::Chpriv,
            b"subcontext" => ContextType::Subcontext,
            b"remote" => ContextType::Remote,
            b"shell" => ContextType::Shell,
            b"command" => ContextType::Command,
            b"app" => ContextType::App,
            b"service" => ContextType::Service,
            b"session" => ContextType::Session,
            _ => return None,
        })
    }
}

/// Exit status for the `exit` end-sequence field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Success,
    Failure,
    Crash,
    Interrupt,
}

impl ExitStatus {
    pub fn parse(value: &[u8]) -> Option<ExitStatus> {
        Some(match value {
            b"success" => ExitStatus::Success,
            b"failure" => ExitStatus::Failure,
            b"crash" => ExitStatus::Crash,
            b"interrupt" => ExitStatus::Interrupt,
            _ => return None,
        })
    }
}

/// Metadata fields that can appear in OSC 3008 sequences.
/// Fields are read lazily from the raw string using the `read` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    // Start sequence fields
    Type,
    User,
    Hostname,
    Machineid,
    Bootid,
    Pid,
    Pidfdid,
    Comm,
    Cwd,
    Cmdline,
    Vm,
    Container,
    Targetuser,
    Targethost,
    Sessionid,

    // End sequence fields
    Exit,
    Status,
    Signal,
}

impl Field {
    fn key(self) -> &'static str {
        match self {
            Field::Type => "type",
            Field::User => "user",
            Field::Hostname => "hostname",
            Field::Machineid => "machineid",
            Field::Bootid => "bootid",
            Field::Pid => "pid",
            Field::Pidfdid => "pidfdid",
            Field::Comm => "comm",
            Field::Cwd => "cwd",
            Field::Cmdline => "cmdline",
            Field::Vm => "vm",
            Field::Container => "container",
            Field::Targetuser => "targetuser",
            Field::Targethost => "targethost",
            Field::Sessionid => "sessionid",
            Field::Exit => "exit",
            Field::Status => "status",
            Field::Signal => "signal",
        }
    }

    /// Read the field value from the raw fields string.
    ///
    /// The raw fields string contains semicolon-separated key=value pairs
    /// e.g. "type=container;user=lennart;hostname=zeta".
    ///
    /// Unknown or malformed fields are ignored per the specification.
    pub fn read(self, raw: &[u8]) -> Option<FieldValue<'_>> {
        let key = self.key().as_bytes();
        for full in raw.split(|&c| c == b';') {
            let value = match full.iter().position(|&c| c == b'=') {
                Some(eql) if &full[..eql] == key => &full[eql + 1..],
                _ => continue,
            };

            return match self {
                Field::Type => ContextType::parse(value).map(FieldValue::Type),
                Field::Exit => ExitStatus::parse(value).map(FieldValue::Exit),
                Field::Pid | Field::Pidfdid | Field::Status => {
                    if value.iter().any(|c| !c.is_ascii_digit()) {
                        return None;
                    }
                    std::str::from_utf8(value)
                        .ok()
                        .and_then(|s| s.parse::<u64>().ok())
                        .map(FieldValue::U64)
                }
                // String fields
                _ => {
                    if value.is_empty() {
                        None
                    } else {
                        Some(FieldValue::Str(value))
                    }
                }
            };
        }

        // Not found
        None
    }
}

/// Parse OSC 3008: hierarchical context signalling.
///
/// Expected data format (after "3008;" prefix has been consumed by the state machine):
///   start=<id>[;<field>=<value>]*
///   end=<id>[;<field>=<value>]*
pub fn parse<S: ByteStorage>(
    parser: &mut Parser<S>,
    _terminator_ch: Option<u8>,
) -> Option<OscCommand<'_>> {
    parser.buffered()?;

    // Validate with a short-lived borrow; the command borrows the payload
    // again below (so marking the parser invalid does not overlap with the
    // returned borrow).
    let (action, id_range, meta_start) = {
        let data = parser.payload();
        if data.is_empty() {
            parser.state = State::Invalid;
            return None;
        }

        // Determine the action (start= or end=)
        let (action, prefix_len) = if data.starts_with(b"start=") {
            (Action::Start, b"start=".len())
        } else if data.starts_with(b"end=") {
            (Action::End, b"end=".len())
        } else {
            log::warn!(
                "OSC 3008: expected 'start=' or 'end=' prefix, got: {}",
                String::from_utf8_lossy(&data[..data.len().min(10)])
            );
            parser.state = State::Invalid;
            return None;
        };

        // Skip past the "start=" or "end=" prefix
        let rest = &data[prefix_len..];

        if rest.is_empty() {
            log::warn!("OSC 3008: missing context ID");
            parser.state = State::Invalid;
            return None;
        }

        // Extract the context ID (up to the first semicolon or end of data)
        let id_end = rest.iter().position(|&c| c == b';').unwrap_or(rest.len());
        let id = &rest[..id_end];

        // Validate context ID length (1-64 chars per spec)
        if id.is_empty() || id.len() > MAX_CONTEXT_ID_LEN {
            log::warn!(
                "OSC 3008: context ID length {} out of range (1-{})",
                id.len(),
                MAX_CONTEXT_ID_LEN
            );
            parser.state = State::Invalid;
            return None;
        }

        // Validate context ID characters (32..126 byte range per spec)
        for &c in id {
            if !(0x20..=0x7e).contains(&c) {
                log::warn!("OSC 3008: invalid character 0x{c:02x} in context ID");
                parser.state = State::Invalid;
                return None;
            }
        }

        // Metadata fields are everything after the ID.
        let meta_start = if id_end < rest.len() {
            prefix_len + id_end + 1
        } else {
            data.len()
        };
        (action, prefix_len..prefix_len + id_end, meta_start)
    };

    let data = parser.payload();
    Some(OscCommand::ContextSignal(Command {
        action,
        id: data[id_range].into(),
        metadata: data[meta_start..].into(),
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn run(input: &str) -> Option<OscCommand<'static>> {
        let mut p = Parser::init(false);
        for &ch in input.as_bytes() {
            p.next(ch);
        }
        p.end(None).map(OscCommand::into_owned)
    }

    fn cs(input: &str) -> Command<'static> {
        match run(input) {
            Some(OscCommand::ContextSignal(c)) => c,
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn osc_3008_basic_start_command() {
        let c = cs("3008;start=abc123");
        assert_eq!(Action::Start, c.action);
        assert_eq!(b"abc123", &c.id[..]);
        assert_eq!(b"", &c.metadata[..]);
    }

    #[test]
    fn osc_3008_basic_end_command() {
        let c = cs("3008;end=abc123");
        assert_eq!(Action::End, c.action);
        assert_eq!(b"abc123", &c.id[..]);
        assert_eq!(b"", &c.metadata[..]);
    }

    #[test]
    fn osc_3008_start_with_metadata_fields() {
        let c = cs(
            "3008;start=bed86fab93af4328bbed0a1224af6d40;type=container;user=lennart;hostname=zeta",
        );
        assert_eq!(Action::Start, c.action);
        assert_eq!(b"bed86fab93af4328bbed0a1224af6d40", &c.id[..]);
        assert_eq!(Some(ContextType::Container), c.context_type());
        assert_eq!(b"lennart", c.read_str(Field::User).unwrap());
        assert_eq!(b"zeta", c.read_str(Field::Hostname).unwrap());
    }

    #[test]
    fn osc_3008_start_with_all_common_fields() {
        let c = cs(
            "3008;start=myctx;type=shell;user=root;hostname=myhost;machineid=3deb5353d3ba43d08201c136a47ead7b;bootid=d4a3d0fdf2e24fdea6d971ce73f4fbf2;pid=1062862;pidfdid=1063162;comm=bash",
        );
        assert_eq!(Some(ContextType::Shell), c.context_type());
        assert_eq!(b"root", c.read_str(Field::User).unwrap());
        assert_eq!(b"myhost", c.read_str(Field::Hostname).unwrap());
        assert_eq!(
            b"3deb5353d3ba43d08201c136a47ead7b",
            c.read_str(Field::Machineid).unwrap()
        );
        assert_eq!(
            b"d4a3d0fdf2e24fdea6d971ce73f4fbf2",
            c.read_str(Field::Bootid).unwrap()
        );
        assert_eq!(Some(1062862), c.read_u64(Field::Pid));
        assert_eq!(Some(1063162), c.read_u64(Field::Pidfdid));
        assert_eq!(b"bash", c.read_str(Field::Comm).unwrap());
    }

    #[test]
    fn osc_3008_end_with_exit_metadata() {
        let c = cs("3008;end=myctx;exit=success;status=0");
        assert_eq!(Action::End, c.action);
        assert_eq!(b"myctx", &c.id[..]);
        assert_eq!(Some(ExitStatus::Success), c.exit());
        assert_eq!(Some(0), c.read_u64(Field::Status));
    }

    #[test]
    fn osc_3008_end_with_failure_exit() {
        let c = cs("3008;end=myctx;exit=failure;status=1;signal=SIGKILL");
        assert_eq!(Some(ExitStatus::Failure), c.exit());
        assert_eq!(Some(1), c.read_u64(Field::Status));
        assert_eq!(b"SIGKILL", c.read_str(Field::Signal).unwrap());
    }

    #[test]
    fn osc_3008_unknown_fields_are_ignored() {
        let c = cs("3008;start=myctx;type=shell;unknownfield=value;user=root");
        assert_eq!(Some(ContextType::Shell), c.context_type());
        assert_eq!(b"root", c.read_str(Field::User).unwrap());
    }

    #[test]
    fn osc_3008_missing_field_returns_null() {
        let c = cs("3008;start=myctx;user=lennart");
        assert!(c.context_type().is_none());
        assert!(c.read_str(Field::Hostname).is_none());
        assert!(c.read_u64(Field::Pid).is_none());
    }

    #[test]
    fn osc_3008_invalid_prefix() {
        assert!(run("3008;bogus=abc123").is_none());
    }

    #[test]
    fn osc_3008_empty_data() {
        assert!(run("3008;start=").is_none());
    }

    #[test]
    fn osc_3008_max_length_context_id() {
        let id = "a".repeat(64);
        let c = cs(&format!("3008;start={id}"));
        assert_eq!(id.as_bytes(), &c.id[..]);
    }

    #[test]
    fn osc_3008_over_length_context_id() {
        let id = "a".repeat(65);
        assert!(run(&format!("3008;start={id}")).is_none());
    }

    #[test]
    fn osc_3008_context_type_enum_coverage() {
        let types = [
            ("boot", ContextType::Boot),
            ("container", ContextType::Container),
            ("vm", ContextType::Vm),
            ("elevate", ContextType::Elevate),
            ("chpriv", ContextType::Chpriv),
            ("subcontext", ContextType::Subcontext),
            ("remote", ContextType::Remote),
            ("shell", ContextType::Shell),
            ("command", ContextType::Command),
            ("app", ContextType::App),
            ("service", ContextType::Service),
            ("session", ContextType::Session),
        ];
        for (s, e) in types {
            assert_eq!(Some(e), ContextType::parse(s.as_bytes()));
        }
        assert!(ContextType::parse(b"invalid").is_none());
    }

    #[test]
    fn osc_3008_exit_status_enum_coverage() {
        assert_eq!(Some(ExitStatus::Success), ExitStatus::parse(b"success"));
        assert_eq!(Some(ExitStatus::Failure), ExitStatus::parse(b"failure"));
        assert_eq!(Some(ExitStatus::Crash), ExitStatus::parse(b"crash"));
        assert_eq!(Some(ExitStatus::Interrupt), ExitStatus::parse(b"interrupt"));
        assert!(ExitStatus::parse(b"invalid").is_none());
    }

    #[test]
    fn osc_3008_spec_example_container_start() {
        let c = cs(
            "3008;start=bed86fab93af4328bbed0a1224af6d40;type=container;user=lennart;hostname=zeta;machineid=3deb5353d3ba43d08201c136a47ead7b;bootid=d4a3d0fdf2e24fdea6d971ce73f4fbf2;pid=1062862;pidfdid=1063162;comm=systemd-nspawn;container=foobar",
        );
        assert_eq!(Action::Start, c.action);
        assert_eq!(b"bed86fab93af4328bbed0a1224af6d40", &c.id[..]);
        assert_eq!(Some(ContextType::Container), c.context_type());
        assert_eq!(b"lennart", c.read_str(Field::User).unwrap());
        assert_eq!(b"zeta", c.read_str(Field::Hostname).unwrap());
        assert_eq!(b"systemd-nspawn", c.read_str(Field::Comm).unwrap());
        assert_eq!(b"foobar", c.read_str(Field::Container).unwrap());
        assert_eq!(Some(1062862), c.read_u64(Field::Pid));
    }

    #[test]
    fn osc_3008_spec_example_context_end() {
        let c = cs("3008;end=bed86fab93af4328bbed0a1224af6d40");
        assert_eq!(Action::End, c.action);
        assert_eq!(b"bed86fab93af4328bbed0a1224af6d40", &c.id[..]);
    }

    #[test]
    fn osc_3008_cwd_and_cmdline_fields() {
        let c = cs("3008;start=myctx;type=command;cwd=/home/user;cmdline=ls -la");
        assert_eq!(b"/home/user", c.read_str(Field::Cwd).unwrap());
        assert_eq!(b"ls -la", c.read_str(Field::Cmdline).unwrap());
    }

    #[test]
    fn osc_3008_start_command_with_no_fields() {
        let c = cs("3008;start=simpleid");
        assert_eq!(Action::Start, c.action);
        assert_eq!(b"simpleid", &c.id[..]);
        assert!(c.context_type().is_none());
        assert!(c.read_str(Field::User).is_none());
        assert!(c.exit().is_none());
    }
}
