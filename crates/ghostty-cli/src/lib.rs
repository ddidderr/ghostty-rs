//! ghostty-cli: `ghostty +action` CLI actions (ZIG/src/cli).
//!
//! Every action lives in its own module and exposes `run(args)`, which takes
//! the full process argument vector (including `argv[0]`, like Zig's
//! `args.argsIterator`) and returns the process exit code. Actions write to
//! stdout/stderr exactly like the Zig implementation.

#![allow(
    clippy::wildcard_imports,
    clippy::struct_field_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::doc_overindented_list_items
)]

#[macro_use]
mod options;

pub mod action;
pub mod boo;
pub mod build_info;
pub mod crash_report;
pub mod edit_config;
pub mod error;
pub mod ghostty;
pub mod help;
pub mod list_actions;
pub mod list_colors;
pub mod list_fonts;
pub mod list_keybinds;
pub mod list_themes;
pub mod new_window;
pub mod show_config;
pub mod show_face;
pub mod ssh_cache;
pub mod tui;
pub mod validate_config;
pub mod version;

pub use error::Error;
pub use ghostty::{Action, run_from_args};
pub use options::{NoOptions, parse_options};
