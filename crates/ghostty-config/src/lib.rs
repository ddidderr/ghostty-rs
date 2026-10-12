//! ghostty-config: configuration parsing, loading and formatting
//! (ZIG/src/config/*, cli/args.zig, cli/CommaSplitter.zig,
//! cli/diagnostics.zig). See docs/rewrite/ARCHITECTURE.md.

#![allow(
    clippy::wildcard_imports,
    clippy::should_implement_trait,
    clippy::float_cmp,
    clippy::collapsible_if,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::manual_let_else
)]

// Generic CLI parsing infrastructure.
pub mod c_value;
pub mod cli_args;
pub mod comma_splitter;
pub mod diagnostics;
pub mod macros;

// Config support modules.
pub mod clipboard_codepoint_map;
pub mod command;
pub mod conditional;
pub mod edit;
pub mod error_list;
pub mod file_load;
pub mod formatter;
pub mod io;
pub mod key;
pub mod path;
pub mod repeatable_string_map;
pub mod string;
pub mod theme;
pub mod url;

// Types that were nested in Config.zig.
pub mod font_types;
pub mod input_fields;
pub mod keybinds;
pub mod metric_modifier;
pub mod types_a;
pub mod types_b;

// The Config struct itself.
pub mod config;
pub mod file_formatter;
pub mod keys;
