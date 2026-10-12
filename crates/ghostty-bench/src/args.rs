//! Minimal `--key=value` option parsing (the subset of `cli/args.zig` used by
//! the benchmark and synthetic CLIs).
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::str::FromStr;

/// Parsed `--key[=value]` arguments.
#[derive(Debug, Default)]
pub struct Args {
    map: HashMap<String, Option<String>>,
}

impl Args {
    /// Parse arguments (skipping `+action` words and the program name).
    pub fn parse<I: IntoIterator<Item = String>>(iter: I) -> Result<Args, String> {
        let mut map = HashMap::new();
        for arg in iter {
            if arg.starts_with('+') {
                continue;
            }
            let Some(rest) = arg.strip_prefix("--") else {
                return Err(format!("unexpected argument: {arg}"));
            };
            match rest.split_once('=') {
                Some((k, v)) => map.insert(k.to_owned(), Some(v.to_owned())),
                None => map.insert(rest.to_owned(), None),
            };
        }
        Ok(Args { map })
    }

    /// Take a typed value, falling back to `default`.
    pub fn take<T: FromStr>(&mut self, key: &str, default: T) -> Result<T, String> {
        match self.map.remove(key) {
            None => Ok(default),
            Some(None) => Err(format!("option --{key} requires a value")),
            Some(Some(v)) => v
                .parse()
                .map_err(|_| format!("invalid value for --{key}: {v}")),
        }
    }

    pub fn take_opt_string(&mut self, key: &str) -> Result<Option<String>, String> {
        match self.map.remove(key) {
            None => Ok(None),
            Some(None) => Err(format!("option --{key} requires a value")),
            Some(Some(v)) => Ok(Some(v)),
        }
    }

    /// Take a boolean flag (`--key`, `--key=true`, `--key=false`).
    pub fn take_flag(&mut self, key: &str) -> Result<bool, String> {
        match self.map.remove(key) {
            None => Ok(false),
            Some(None) => Ok(true),
            Some(Some(v)) => match v.as_str() {
                "true" | "t" | "1" | "yes" => Ok(true),
                "false" | "f" | "0" | "no" => Ok(false),
                _ => Err(format!("invalid value for --{key}: {v}")),
            },
        }
    }

    /// Error on any unconsumed option.
    pub fn finish(self) -> Result<(), String> {
        match self.map.into_keys().next() {
            Some(k) => Err(format!("unknown option --{k}")),
            None => Ok(()),
        }
    }
}

/// Returns the data source for the path: `None` if unset, stdin for `-`,
/// otherwise the opened file (mirrors `benchmark/options.zig`).
pub fn data_file(path: Option<&str>) -> io::Result<Option<Box<dyn Read>>> {
    match path {
        None => Ok(None),
        Some("-") => Ok(Some(Box::new(io::stdin()))),
        Some(p) => Ok(Some(Box::new(File::open(p)?))),
    }
}

/// Read until `buf` is full or EOF (Zig `readSliceShort`).
pub fn read_short(r: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(m) => n += m,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}
