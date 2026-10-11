//! Search functionality for the terminal.
#![allow(clippy::should_implement_trait)]

pub mod active;
pub mod pagelist;
pub mod screen;
pub mod sliding_window;
#[cfg(test)]
pub(crate) mod test_util;
pub mod thread;
pub mod viewport;
