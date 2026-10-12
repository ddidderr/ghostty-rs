//! Minimal `--key=value` option parsing (the subset of `cli/args.zig` used by
//! the benchmark and synthetic CLIs).
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use ghostty_os::osbytes::{os_bytes, os_from_bytes};

/// Typed option value parsed like `cli/args.zig` `parseIntoField`
/// (`std.fmt.parseInt(T, v, 0)` / `std.fmt.parseFloat`).
pub trait ZigArg: Sized {
    fn parse_zig(v: &[u8]) -> Option<Self>;
}

macro_rules! zig_arg_int {
    ($($t:ty),*) => {$(
        impl ZigArg for $t {
            fn parse_zig(v: &[u8]) -> Option<Self> {
                ghostty_os::zig_fmt::parse_int::<$t>(v, 0).ok()
            }
        }
    )*};
}
zig_arg_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl ZigArg for f64 {
    fn parse_zig(v: &[u8]) -> Option<Self> {
        ghostty_os::zig_fmt::parse_float::<f64>(v).ok()
    }
}
impl ZigArg for f32 {
    fn parse_zig(v: &[u8]) -> Option<Self> {
        ghostty_os::zig_fmt::parse_float::<f32>(v).ok()
    }
}

/// Parsed `--key[=value]` arguments.
#[derive(Debug, Default)]
pub struct Args {
    // Values are OS strings: file names (`--data=...`) need not be UTF-8.
    map: HashMap<String, Option<OsString>>,
}

/// The action name of a `+action` argument, if `arg` is one.
#[must_use]
pub fn action_name(arg: &OsStr) -> Option<String> {
    let bytes = os_bytes(arg);
    let name = bytes.strip_prefix(b"+")?;
    Some(String::from_utf8_lossy(name).into_owned())
}

impl Args {
    /// Parse arguments (skipping `+action` words and the program name).
    pub fn parse<I, S>(iter: I) -> Result<Args, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut map = HashMap::new();
        for arg in iter {
            let bytes = os_bytes(&arg.into());
            if bytes.starts_with(b"+") {
                continue;
            }
            let Some(rest) = bytes.strip_prefix(b"--") else {
                return Err(format!(
                    "unexpected argument: {}",
                    String::from_utf8_lossy(&bytes)
                ));
            };
            let (key, value) = match rest.iter().position(|b| *b == b'=') {
                Some(i) => (&rest[..i], Some(os_from_bytes(rest[i + 1..].to_vec()))),
                None => (rest, None),
            };
            let key = String::from_utf8(key.to_vec())
                .map_err(|_| "option names must be valid UTF-8".to_owned())?;
            map.insert(key, value);
        }
        Ok(Args { map })
    }

    fn take_os(&mut self, key: &str) -> Result<Option<OsString>, String> {
        match self.map.remove(key) {
            None => Ok(None),
            Some(None) => Err(format!("option --{key} requires a value")),
            Some(Some(v)) => Ok(Some(v)),
        }
    }

    /// Take a typed value, falling back to `default`.
    pub fn take<T: ZigArg>(&mut self, key: &str, default: T) -> Result<T, String> {
        match self.take_os(key)? {
            None => Ok(default),
            Some(v) => T::parse_zig(&os_bytes(&v))
                .ok_or_else(|| format!("invalid value for --{key}: {}", v.to_string_lossy())),
        }
    }

    pub fn take_opt_string(&mut self, key: &str) -> Result<Option<String>, String> {
        match self.take_os(key)? {
            None => Ok(None),
            Some(v) => v
                .into_string()
                .map(Some)
                .map_err(|v| format!("invalid value for --{key}: {}", v.to_string_lossy())),
        }
    }

    /// Take a file path value (any bytes are allowed).
    pub fn take_opt_path(&mut self, key: &str) -> Result<Option<PathBuf>, String> {
        Ok(self.take_os(key)?.map(PathBuf::from))
    }

    /// Take a boolean flag (`--key`, `--key=true`, `--key=false`).
    pub fn take_flag(&mut self, key: &str) -> Result<bool, String> {
        match self.map.remove(key) {
            None => Ok(false),
            Some(None) => Ok(true),
            Some(Some(v)) => match v.to_str().unwrap_or_default() {
                "true" | "t" | "1" | "yes" => Ok(true),
                "false" | "f" | "0" | "no" => Ok(false),
                _ => Err(format!(
                    "invalid value for --{key}: {}",
                    v.to_string_lossy()
                )),
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
pub fn data_file(path: Option<&Path>) -> io::Result<Option<Box<dyn Read>>> {
    match path {
        None => Ok(None),
        Some(p) if p == Path::new("-") => Ok(Some(Box::new(io::stdin()))),
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

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    use super::*;

    #[test]
    fn take_uses_zig_parse_int_base_0() {
        let args = ["--terminal-rows=0x10", "--terminal-cols=1_0", "--p=0x1p-1"];
        let mut a = Args::parse(args.map(OsString::from)).unwrap();
        assert_eq!(a.take("terminal-rows", 0u16).unwrap(), 16);
        assert_eq!(a.take("terminal-cols", 0u16).unwrap(), 10);
        assert_eq!(a.take("p", 0f64).unwrap(), 0.5);
        let mut a = Args::parse([OsString::from("--n=+5")]).unwrap();
        assert_eq!(a.take("n", 0u16).unwrap(), 5);
        let mut a = Args::parse([OsString::from("--n=-1")]).unwrap();
        assert!(a.take("n", 0u16).is_err());
    }

    #[test]
    fn data_path_keeps_non_utf8_bytes() {
        let arg = OsString::from_vec(b"--data=/tmp/a\xffb".to_vec());
        let mut args = Args::parse([arg]).unwrap();
        let p = args.take_opt_path("data").unwrap().unwrap();
        assert_eq!(p.as_os_str().as_bytes(), b"/tmp/a\xffb");
        args.finish().unwrap();
    }

    #[test]
    fn action_name_extracts_plus_words() {
        assert_eq!(action_name(OsStr::new("+ascii")).as_deref(), Some("ascii"));
        assert_eq!(action_name(OsStr::new("--x")), None);
    }
}
