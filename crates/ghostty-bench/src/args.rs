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

/// Parsed `--key[=value]` arguments, in command line order.
#[derive(Debug, Default)]
pub struct Args {
    // Values are OS strings: file names (`--data=...`) need not be UTF-8.
    // Every occurrence is kept: like `cli/args.zig` each one is applied in
    // order when the field is taken.
    map: HashMap<String, Vec<Option<OsString>>>,
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
        let mut map: HashMap<String, Vec<Option<OsString>>> = HashMap::new();
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
            map.entry(key).or_default().push(value);
        }
        Ok(Args { map })
    }

    /// Apply every occurrence of `key` in order (Zig `parseIntoField`): an
    /// empty value resets to the default, otherwise `set` parses the value
    /// (`None` = no `=value`). The first error wins.
    fn apply<T: Clone>(
        &mut self,
        key: &str,
        default: T,
        mut set: impl FnMut(Option<&OsStr>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut cur = default.clone();
        for v in self.map.remove(key).unwrap_or_default() {
            cur = match v.as_deref() {
                Some(v) if v.is_empty() => default.clone(),
                v => set(v)?,
            };
        }
        Ok(cur)
    }

    /// Take a typed value, falling back to `default`.
    pub fn take<T: ZigArg + Clone>(&mut self, key: &str, default: T) -> Result<T, String> {
        self.apply(key, default, |v| {
            let v = v.ok_or_else(|| format!("option --{key} requires a value"))?;
            T::parse_zig(&os_bytes(v))
                .ok_or_else(|| format!("invalid value for --{key}: {}", v.to_string_lossy()))
        })
    }

    pub fn take_opt_string(&mut self, key: &str) -> Result<Option<String>, String> {
        self.apply(key, None, |v| {
            let v = v.ok_or_else(|| format!("option --{key} requires a value"))?;
            v.to_str()
                .map(|s| Some(s.to_owned()))
                .ok_or_else(|| format!("invalid value for --{key}: {}", v.to_string_lossy()))
        })
    }

    /// Take an enum value (`parseIntoField` on an enum): every occurrence is
    /// validated against `variants` in order (a bad value is an error even if
    /// a later one is fine); an empty value resets to `default`.
    pub fn take_enum<T: Copy>(
        &mut self,
        key: &str,
        default: T,
        variants: &[(&str, T)],
    ) -> Result<T, String> {
        self.apply(key, default, |v| {
            let v = v.ok_or_else(|| format!("option --{key} requires a value"))?;
            let bad = || format!("invalid value for --{key}: {}", v.to_string_lossy());
            let v = v.to_str().ok_or_else(bad)?;
            variants
                .iter()
                .find(|(n, _)| *n == v)
                .map(|(_, t)| *t)
                .ok_or_else(bad)
        })
    }

    /// Take a file path value (any bytes are allowed).
    pub fn take_opt_path(&mut self, key: &str) -> Result<Option<PathBuf>, String> {
        self.apply(key, None, |v| {
            let v = v.ok_or_else(|| format!("option --{key} requires a value"))?;
            Ok(Some(PathBuf::from(v)))
        })
    }

    /// Take a boolean flag (`--key`, `--key=true`, `--key=false`; `parseBool`).
    pub fn take_flag(&mut self, key: &str) -> Result<bool, String> {
        self.apply(key, false, |v| match v.map(os_bytes) {
            None => Ok(true),
            Some(v) => match v.as_slice() {
                b"1" | b"t" | b"T" | b"true" => Ok(true),
                b"0" | b"f" | b"F" | b"false" => Ok(false),
                _ => Err(format!(
                    "invalid value for --{key}: {}",
                    String::from_utf8_lossy(&v)
                )),
            },
        })
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
    fn take_enum_validates_every_occurrence() {
        let t = |v: &[&str]| {
            Args::parse(v.iter().map(OsString::from))
                .unwrap()
                .take_enum("mode", 0u8, &[("noop", 0), ("table", 1)])
        };
        assert!(t(&["--mode=bad", "--mode=noop"]).is_err());
        assert!(t(&["--mode=noop", "--mode=bad"]).is_err());
        assert!(t(&["--mode=bad", "--mode="]).is_err());
        assert_eq!(t(&["--mode=table", "--mode="]).unwrap(), 0);
        assert_eq!(t(&["--mode=noop", "--mode=table"]).unwrap(), 1);
        assert_eq!(t(&[]).unwrap(), 0);
    }

    #[test]
    fn duplicates_apply_in_order_and_first_error_wins() {
        let a = |v: &[&str]| Args::parse(v.iter().map(OsString::from)).unwrap();
        // A bad value earlier is an error even if a later one is fine.
        assert!(
            a(&["--terminal-rows=bad", "--terminal-rows=10"])
                .take("terminal-rows", 80u16)
                .is_err()
        );
        // The last good value wins.
        assert_eq!(
            a(&["--terminal-rows=5", "--terminal-rows=10"])
                .take("terminal-rows", 80u16)
                .unwrap(),
            10
        );
        // An empty value resets to the default.
        assert_eq!(
            a(&["--terminal-rows=5", "--terminal-rows="])
                .take("terminal-rows", 80u16)
                .unwrap(),
            80
        );
        assert_eq!(
            a(&["--terminal-rows="])
                .take("terminal-rows", 80u16)
                .unwrap(),
            80
        );
        // Missing value is still an error.
        assert!(
            a(&["--terminal-rows"])
                .take("terminal-rows", 80u16)
                .is_err()
        );
        // Optional path: empty resets to unset.
        assert_eq!(
            a(&["--data=/x", "--data="]).take_opt_path("data").unwrap(),
            None
        );
        assert_eq!(a(&["--data="]).take_opt_path("data").unwrap(), None);
        // Flags: empty resets, parseBool spellings only.
        assert!(!a(&["--time", "--time="]).take_flag("time").unwrap());
        assert!(a(&["--time=T"]).take_flag("time").unwrap());
        assert!(a(&["--time=yes"]).take_flag("time").is_err());
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
