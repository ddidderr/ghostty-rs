//! Port of `config/ErrorList.zig`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub message: String,
}

/// The list of errors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ErrorList {
    pub list: Vec<Error>,
}

impl ErrorList {
    /// True if there are no errors.
    #[must_use]
    pub fn empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Add a new error to the list.
    pub fn add(&mut self, err: Error) {
        self.list.push(err);
    }
}
