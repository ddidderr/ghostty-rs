//! Helpers shared by the action `Options` structs.

use ghostty_config::cli_args::{self, ArgsIterator, CliTarget};

use crate::error::Error;

/// Implements [`CliTarget`] for a struct declared with `config_struct!` and
/// gives it a `Default`. Pass `help` to enable `-h`/`--help` (the Zig
/// `pub fn help` decl); structs without it treat `--help` as an ordinary
/// (unknown or declared) field.
macro_rules! cli_options {
    ($name:ident, help, diagnostics) => {
        cli_options!(@base $name);
        impl ::ghostty_config::cli_args::CliTarget for $name {
            fn parse_into_field(
                &mut self,
                key: &str,
                value: ::core::option::Option<&str>,
            ) -> ::core::result::Result<(), ::ghostty_config::cli_args::ArgError> {
                self.parse_field(key, value)
            }
            fn valid_values(&self, key: &str) -> ::core::option::Option<&'static [&'static str]> {
                Self::valid_values_for(key)
            }
            fn diagnostics_mut(
                &mut self,
            ) -> ::core::option::Option<&mut ::ghostty_config::diagnostics::DiagnosticList> {
                ::core::option::Option::Some(&mut self.diagnostics)
            }
            fn supports_help(&self) -> bool {
                true
            }
            fn help(&mut self) -> ::core::result::Result<(), ::ghostty_config::cli_args::ArgError> {
                ::core::result::Result::Err(::ghostty_config::cli_args::ArgError::other(
                    $crate::error::HELP_ERROR_NAME,
                ))
            }
        }
    };
    ($name:ident, help) => {
        cli_options!(@base $name);
        impl ::ghostty_config::cli_args::CliTarget for $name {
            fn parse_into_field(
                &mut self,
                key: &str,
                value: ::core::option::Option<&str>,
            ) -> ::core::result::Result<(), ::ghostty_config::cli_args::ArgError> {
                self.parse_field(key, value)
            }
            fn valid_values(&self, key: &str) -> ::core::option::Option<&'static [&'static str]> {
                Self::valid_values_for(key)
            }
            fn supports_help(&self) -> bool {
                true
            }
            fn help(&mut self) -> ::core::result::Result<(), ::ghostty_config::cli_args::ArgError> {
                ::core::result::Result::Err(::ghostty_config::cli_args::ArgError::other(
                    $crate::error::HELP_ERROR_NAME,
                ))
            }
        }
    };
    ($name:ident) => {
        cli_options!(@base $name);
        impl ::ghostty_config::cli_args::CliTarget for $name {
            fn parse_into_field(
                &mut self,
                key: &str,
                value: ::core::option::Option<&str>,
            ) -> ::core::result::Result<(), ::ghostty_config::cli_args::ArgError> {
                self.parse_field(key, value)
            }
            fn valid_values(&self, key: &str) -> ::core::option::Option<&'static [&'static str]> {
                Self::valid_values_for(key)
            }
        }
    };
    (@base $name:ident) => {
        impl ::core::default::Default for $name {
            fn default() -> Self {
                Self::raw_default()
            }
        }
    };
}

/// Parse the CLI arguments `args` (the full argv, including `argv[0]`,
/// which is skipped; `+action` arguments are ignored) into `opts`
/// (Zig: `args.argsIterator` + `args.parse`).
pub fn parse_options<T: CliTarget>(opts: &mut T, args: &[String]) -> Result<(), Error> {
    let mut iter = ArgsIterator::new(args.iter().skip(1).cloned());
    cli_args::parse(opts, &mut iter)?;
    Ok(())
}

/// The options of an action that has no flags of its own (only `-h`/`--help`).
#[derive(Debug, Clone, Default)]
pub struct NoOptions;

impl CliTarget for NoOptions {
    fn parse_into_field(
        &mut self,
        _key: &str,
        _value: Option<&str>,
    ) -> Result<(), cli_args::ArgError> {
        Err(cli_args::ArgError::InvalidField)
    }

    fn supports_help(&self) -> bool {
        true
    }

    fn help(&mut self) -> Result<(), cli_args::ArgError> {
        Err(cli_args::ArgError::other(crate::error::HELP_ERROR_NAME))
    }
}
