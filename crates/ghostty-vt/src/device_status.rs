//! Device status requests (ZIG/src/terminal/device_status.zig).

/// The available device status requests. Each is identified by a numeric
/// value plus whether it uses the "?" prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Request {
    OperatingStatus,
    CursorPosition,
    ColorScheme,
}

impl Request {
    /// The numeric value of the request.
    pub fn value(self) -> u16 {
        ENTRIES
            .iter()
            .find(|e| e.request == self)
            .map_or(0, |e| e.value)
    }

    /// Whether the request uses the "?" prefix.
    pub fn question(self) -> bool {
        ENTRIES
            .iter()
            .find(|e| e.request == self)
            .is_some_and(|e| e.question)
    }
}

struct Entry {
    request: Request,
    value: u16,
    question: bool,
}

/// The full list of device status request entries.
const ENTRIES: &[Entry] = &[
    Entry {
        request: Request::OperatingStatus,
        value: 5,
        question: false,
    },
    Entry {
        request: Request::CursorPosition,
        value: 6,
        question: false,
    },
    Entry {
        request: Request::ColorScheme,
        value: 996,
        question: true,
    },
];

pub fn req_from_int(v: u16, question: bool) -> Option<Request> {
    ENTRIES
        .iter()
        .find(|e| e.value == v && e.question == question)
        .map(|e| e.request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_from_int_lookup() {
        assert_eq!(req_from_int(5, false), Some(Request::OperatingStatus));
        assert_eq!(req_from_int(6, false), Some(Request::CursorPosition));
        assert_eq!(req_from_int(996, true), Some(Request::ColorScheme));
        assert_eq!(req_from_int(996, false), None);
        assert_eq!(req_from_int(5, true), None);
        assert_eq!(Request::ColorScheme.value(), 996);
        assert!(Request::ColorScheme.question());
    }
}
