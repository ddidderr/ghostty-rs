//! Synthetic data generators (port of `ZIG/src/synthetic`): random bytes,
//! UTF-8 and OSC sequences, used to feed the benchmarks.
use ghostty_vt::string_encoding::url_percent_encode;
use std::io::{self, Write};

/// xoshiro256++ (what Zig's `std.Random.DefaultPrng` is).
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // splitmix64 seeding, as in Zig.
        let mut x = seed;
        let mut s = [0u64; 4];
        for v in &mut s {
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            *v = z ^ (z >> 31);
        }
        Rng { s }
    }

    pub fn from_time() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        Rng::new(nanos as u64)
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    pub fn bytes(&mut self, out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }

    pub fn boolean(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    pub fn float(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform-ish (biased) integer in `min..=max`.
    pub fn int_range_at_most(&mut self, min: u64, max: u64) -> u64 {
        debug_assert!(min <= max);
        let span = u128::from(max - min) + 1;
        min + ((u128::from(self.next_u64()) * span) >> 64) as u64
    }

    pub fn usize_range(&mut self, min: usize, max: usize) -> usize {
        self.int_range_at_most(min as u64, max as u64) as usize
    }

    /// Index chosen with probability proportional to `weights`.
    pub fn weighted_index(&mut self, weights: &[f64]) -> usize {
        let sum: f64 = weights.iter().sum();
        let mut v = self.float() * sum;
        for (i, w) in weights.iter().enumerate() {
            if v < *w {
                return i;
            }
            v -= w;
        }
        weights.len() - 1
    }
}

/// Alphabet of bytes for which `f` is true.
pub fn generate_alphabet(f: impl Fn(u8) -> bool) -> Vec<u8> {
    (0..=255u8).filter(|&c| f(c)).collect()
}

/// Generates bytes.
#[derive(Debug, Clone)]
pub struct Bytes<'a> {
    pub min_len: usize,
    pub max_len: usize,
    pub alphabet: Option<&'a [u8]>,
}

impl Default for Bytes<'_> {
    fn default() -> Self {
        Bytes {
            min_len: 1,
            max_len: usize::MAX,
            alphabet: None,
        }
    }
}

impl<'a> Bytes<'a> {
    #[must_use]
    pub fn new_alphabet(&self, alphabet: Option<&'a [u8]>) -> Bytes<'a> {
        Bytes {
            alphabet,
            ..self.clone()
        }
    }

    #[must_use]
    pub fn at_most(&self, new_max: usize) -> Bytes<'a> {
        Bytes {
            alphabet: self.alphabet,
            min_len: self.min_len.min(self.max_len.min(new_max)),
            max_len: self.max_len.min(new_max),
        }
    }

    /// Write random data, returning the number of bytes written.
    pub fn write<W: Write>(&self, rng: &mut Rng, w: &mut W) -> io::Result<usize> {
        debug_assert!(self.min_len >= 1);
        debug_assert!(self.max_len >= self.min_len);
        let len = rng.usize_range(self.min_len, self.max_len);
        let mut buf = [0u8; 8];
        let mut remaining = len;
        while remaining > 0 {
            let data = &mut buf[..remaining.min(8)];
            rng.bytes(data);
            if let Some(a) = self.alphabet {
                for b in data.iter_mut() {
                    *b = a[*b as usize % a.len()];
                }
            }
            w.write_all(data)?;
            remaining -= data.len();
        }
        Ok(len)
    }

    /// `next` of the Zig generator: at most `max_len`.
    pub fn next<W: Write>(&self, rng: &mut Rng, w: &mut W, max_len: usize) -> io::Result<()> {
        self.at_most(max_len).write(rng, w).map(|_| ())
    }
}

/// Generates UTF-8 with a configurable distribution of encoding lengths.
#[derive(Debug, Clone)]
pub struct Utf8 {
    pub min_len: usize,
    pub max_len: usize,
    /// Relative weights for 1, 2, 3 and 4 byte encodings.
    pub p_length: [f64; 4],
}

impl Default for Utf8 {
    fn default() -> Self {
        Utf8 {
            min_len: 1,
            max_len: usize::MAX,
            p_length: [1.0; 4],
        }
    }
}

impl Utf8 {
    pub fn next<W: Write>(&self, rng: &mut Rng, w: &mut W, max_len: usize) -> io::Result<()> {
        let len = rng.usize_range(self.min_len, self.max_len).min(max_len);
        let mut rem = len;
        while rem > 0 {
            let mut n = rng.weighted_index(&self.p_length) + 1;
            while n > rem {
                n -= 1;
            }
            let cp = match n {
                1 => rng.int_range_at_most(0x00, 0x7F),
                2 => rng.int_range_at_most(0x80, 0x7FF),
                3 => rng.int_range_at_most(0x800, 0xFFFF),
                _ => rng.int_range_at_most(0x10000, 0x10FFFF),
            } as u32;
            // Surrogates can't be encoded; redo the loop.
            let Some(ch) = char::from_u32(cp) else {
                continue;
            };
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            w.write_all(s.as_bytes())?;
            rem -= s.len();
        }
        Ok(())
    }
}

/// Valid OSC request kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidKind {
    ChangeWindowTitle,
    PromptStart,
    PromptEnd,
    EndOfInput,
    EndOfCommand,
    RxvtNotify,
    MouseShape,
    ClipboardOperation,
    ReportPwd,
    HyperlinkStart,
    HyperlinkEnd,
    ConemuProgress,
    Iterm2Notification,
}

const VALID_KINDS: [ValidKind; 13] = [
    ValidKind::ChangeWindowTitle,
    ValidKind::PromptStart,
    ValidKind::PromptEnd,
    ValidKind::EndOfInput,
    ValidKind::EndOfCommand,
    ValidKind::RxvtNotify,
    ValidKind::MouseShape,
    ValidKind::ClipboardOperation,
    ValidKind::ReportPwd,
    ValidKind::HyperlinkStart,
    ValidKind::HyperlinkEnd,
    ValidKind::ConemuProgress,
    ValidKind::Iterm2Notification,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidKind {
    /// Literally random bytes. Might even be valid, but probably not.
    Random,
    /// A good prefix, but ultimately invalid format.
    GoodPrefix,
}

const INVALID_KINDS: [InvalidKind; 2] = [InvalidKind::Random, InvalidKind::GoodPrefix];

/// Generates random terminal OSC requests.
#[derive(Debug, Clone)]
pub struct Osc {
    /// Probability of a valid OSC sequence being generated.
    pub p_valid: f64,
    pub p_valid_kind: [f64; 13],
    pub p_invalid_kind: [f64; 2],
    kv_alphabet: Vec<u8>,
    full_alphabet: Vec<u8>,
    ascii_alphabet: Vec<u8>,
    alphabetic_alphabet: Vec<u8>,
    alphanumeric_alphabet: Vec<u8>,
}

impl Default for Osc {
    fn default() -> Self {
        Osc::new(1.0)
    }
}

impl Osc {
    pub fn new(p_valid: f64) -> Osc {
        Osc {
            p_valid,
            p_valid_kind: [1.0; 13],
            p_invalid_kind: [1.0; 2],
            kv_alphabet: generate_alphabet(|c| {
                !matches!(c, 0x1b | 0x07 | b';' | b'=') && (0x20..0x7f).contains(&c)
            }),
            full_alphabet: generate_alphabet(|c| !matches!(c, 0x1b | 0x07)),
            ascii_alphabet: generate_alphabet(|c| (0x20..0x7f).contains(&c)),
            alphabetic_alphabet: generate_alphabet(|c| c.is_ascii_alphabetic()),
            alphanumeric_alphabet: generate_alphabet(|c| c.is_ascii_alphanumeric()),
        }
    }

    fn bytes(&self) -> Bytes<'_> {
        Bytes {
            alphabet: Some(&self.full_alphabet),
            ..Bytes::default()
        }
    }

    /// Get the next OSC request, prefixed `ESC ]` and terminated by `BEL`.
    /// `max_len` must be at least 3.
    pub fn next<W: Write>(&self, rng: &mut Rng, w: &mut W, max_len: usize) -> io::Result<()> {
        assert!(max_len >= 3);
        w.write_all(b"\x1b]")?;
        self.next_unwrapped(rng, w, max_len - 3)?;
        w.write_all(b"\x07")
    }

    fn next_unwrapped<W: Write>(&self, rng: &mut Rng, w: &mut W, max_len: usize) -> io::Result<()> {
        if rng.float() > self.p_valid {
            let k = INVALID_KINDS[rng.weighted_index(&self.p_invalid_kind)];
            self.invalid_exact(rng, w, k, max_len)
        } else {
            let k = VALID_KINDS[rng.weighted_index(&self.p_valid_kind)];
            self.valid_exact(rng, w, k, max_len)
        }
    }

    #[allow(clippy::too_many_lines)]
    fn valid_exact<W: Write>(
        &self,
        rng: &mut Rng,
        w: &mut W,
        k: ValidKind,
        max_len: usize,
    ) -> io::Result<()> {
        match k {
            ValidKind::ChangeWindowTitle => {
                if max_len < 3 {
                    return Ok(());
                }
                w.write_all(b"0;")?;
                self.bytes().at_most(max_len - 3).write(rng, w)?;
            }
            ValidKind::PromptStart => {
                if max_len < 4 {
                    return Ok(());
                }
                let mut remaining = max_len;
                w.write_all(b"133;A")?;
                remaining -= 4;
                if rng.boolean() && remaining >= 6 {
                    w.write_all(b";aid=")?;
                    remaining -= 5;
                    remaining -= self
                        .bytes()
                        .new_alphabet(Some(&self.kv_alphabet))
                        .at_most(remaining.min(16))
                        .write(rng, w)?;
                }
                if rng.boolean() && remaining >= 9 {
                    w.write_all(b";redraw=")?;
                    w.write_all(if rng.boolean() { b"1" } else { b"0" })?;
                }
            }
            ValidKind::PromptEnd => {
                if max_len >= 4 {
                    w.write_all(b"133;B")?;
                }
            }
            ValidKind::EndOfInput => {
                if max_len < 5 {
                    return Ok(());
                }
                let mut remaining = max_len;
                w.write_all(b"133;C")?;
                remaining -= 5;
                if rng.boolean() {
                    let prefix = b";cmdline_url=";
                    if remaining < prefix.len() + 1 {
                        return Ok(());
                    }
                    w.write_all(prefix)?;
                    remaining -= prefix.len();
                    let mut tmp = Vec::new();
                    self.bytes()
                        .new_alphabet(Some(&self.ascii_alphabet))
                        .at_most(remaining.min(128))
                        .write(rng, &mut tmp)?;
                    url_percent_encode(w, &tmp)?;
                }
            }
            ValidKind::EndOfCommand => {
                if max_len < 4 {
                    return Ok(());
                }
                w.write_all(b"133;D")?;
                if rng.boolean() && max_len >= 7 {
                    write!(w, ";{}", rng.next_u64() as u8)?;
                }
            }
            ValidKind::MouseShape => {
                if max_len < 4 {
                    return Ok(());
                }
                w.write_all(b"22;")?;
                self.bytes()
                    .new_alphabet(Some(&self.alphabetic_alphabet))
                    .at_most(32.min(max_len - 3))
                    .write(rng, w)?;
            }
            ValidKind::RxvtNotify => {
                let prefix = b"777;notify;";
                if max_len < prefix.len() {
                    return Ok(());
                }
                let mut remaining = max_len;
                w.write_all(prefix)?;
                remaining -= prefix.len();
                remaining -= self
                    .bytes()
                    .new_alphabet(Some(&self.kv_alphabet))
                    .at_most(remaining.saturating_sub(2).clamp(1, 32))
                    .write(rng, w)?;
                w.write_all(b";")?;
                remaining -= 1;
                if remaining >= 1 {
                    self.bytes().at_most(remaining).write(rng, w)?;
                }
            }
            ValidKind::ClipboardOperation => {
                w.write_all(b"52;")?;
                let mut remaining = max_len.saturating_sub(3);
                if rng.boolean() {
                    remaining -= self
                        .bytes()
                        .new_alphabet(Some(&self.alphabetic_alphabet))
                        .at_most(1)
                        .write(rng, w)?;
                }
                w.write_all(b";")?;
                remaining = remaining.saturating_sub(1);
                if rng.boolean() && remaining >= 1 {
                    self.bytes().at_most(remaining).write(rng, w)?;
                }
            }
            ValidKind::ReportPwd => {
                let prefix = b"7;file://localhost";
                if max_len < prefix.len() {
                    return Ok(());
                }
                let mut remaining = max_len;
                w.write_all(prefix)?;
                remaining -= prefix.len();
                for _ in 0..rng.usize_range(2, 5) {
                    w.write_all(b"/")?;
                    remaining = remaining.saturating_sub(1);
                    if remaining == 0 {
                        break;
                    }
                    remaining -= self
                        .bytes()
                        .new_alphabet(Some(&self.alphanumeric_alphabet))
                        .at_most(remaining.min(16))
                        .write(rng, w)?;
                }
            }
            ValidKind::HyperlinkStart => {
                w.write_all(b"8;")?;
                if rng.boolean() {
                    w.write_all(b"id=")?;
                    self.alnum().at_most(16).write(rng, w)?;
                }
                w.write_all(b";https://localhost")?;
                for _ in 0..rng.usize_range(2, 5) {
                    w.write_all(b"/")?;
                    self.alnum().at_most(16).write(rng, w)?;
                }
            }
            ValidKind::HyperlinkEnd => {
                if max_len >= 3 {
                    w.write_all(b"8;;")?;
                }
            }
            ValidKind::ConemuProgress => {
                w.write_all(b"9;")?;
                let c = rng.int_range_at_most(0, 4);
                match c {
                    0 | 3 => write!(w, ";{c}")?,
                    _ => {
                        if rng.boolean() {
                            write!(w, ";{c}")?;
                        } else {
                            write!(w, ";{c};{}", rng.int_range_at_most(0, 100))?;
                        }
                    }
                }
            }
            ValidKind::Iterm2Notification => {
                if max_len < 3 {
                    return Ok(());
                }
                // Prefix ensures this is not interpreted as a ConEmu OSC.
                w.write_all(b"9;_")?;
                self.bytes()
                    .new_alphabet(Some(&self.ascii_alphabet))
                    .at_most(max_len - 3)
                    .write(rng, w)?;
            }
        }
        Ok(())
    }

    fn alnum(&self) -> Bytes<'_> {
        self.bytes().new_alphabet(Some(&self.alphanumeric_alphabet))
    }

    fn invalid_exact<W: Write>(
        &self,
        rng: &mut Rng,
        w: &mut W,
        k: InvalidKind,
        max_len: usize,
    ) -> io::Result<()> {
        match k {
            InvalidKind::Random => {
                self.bytes().at_most(max_len).write(rng, w)?;
            }
            InvalidKind::GoodPrefix => {
                w.write_all(b"133;")?;
                self.bytes()
                    .at_most(max_len.saturating_sub(4).max(1))
                    .write(rng, w)?;
            }
        }
        Ok(())
    }
}

/// A fixed-capacity writer that fails when full (Zig `Writer.fixed`).
pub struct Fixed<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> Fixed<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Fixed { buf, len: 0 }
    }

    pub fn buffered(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl Write for Fixed<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let n = data.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&data[..n]);
        self.len += n;
        if n == 0 && !data.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "fixed buffer full",
            ));
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `ghostty-synthetic +ascii`: endless printable-ASCII (no space) stream.
pub fn run_ascii<W: Write>(w: &mut W, rng: &mut Rng) -> io::Result<()> {
    let alphabet = generate_alphabet(|c| c != b' ' && (0x20..0x7f).contains(&c));
    let generator = Bytes {
        min_len: 1024,
        max_len: 1024,
        alphabet: Some(&alphabet),
    };
    loop {
        if finished(generator.write(rng, w))? {
            return Ok(());
        }
    }
}

/// `ghostty-synthetic +utf8`: endless UTF-8 stream.
pub fn run_utf8<W: Write>(w: &mut W, rng: &mut Rng) -> io::Result<()> {
    let generator = Utf8::default();
    loop {
        if finished(generator.next(rng, w, 1024).map(|()| 0))? {
            return Ok(());
        }
    }
}

/// `ghostty-synthetic +sgr` (Rust-only): endless SGR/CSI-heavy text that
/// resembles colorized program output: colored words, cursor movement,
/// erase sequences and newlines.
pub fn run_sgr<W: Write>(w: &mut W, rng: &mut Rng) -> io::Result<()> {
    let words = generate_alphabet(|c| c.is_ascii_lowercase());
    let mut line = Vec::with_capacity(512);
    loop {
        line.clear();
        for _ in 0..rng.usize_range(4, 12) {
            match rng.usize_range(0, 9) {
                0 => write!(line, "\x1b[{}m", rng.usize_range(30, 37))?,
                1 => write!(
                    line,
                    "\x1b[38;2;{};{};{}m",
                    rng.next_u64() as u8,
                    rng.next_u64() as u8,
                    rng.next_u64() as u8
                )?,
                2 => write!(
                    line,
                    "\x1b[1;{};{}m",
                    rng.usize_range(30, 37),
                    rng.usize_range(40, 47)
                )?,
                3 => line.extend_from_slice(b"\x1b[0m"),
                4 => write!(line, "\x1b[38;5;{}m", rng.next_u64() as u8)?,
                5 => line.extend_from_slice(b"\x1b[K"),
                6 => write!(line, "\x1b[{}C", rng.usize_range(1, 4))?,
                7 => line.extend_from_slice(b"\x1b[4m"),
                _ => line.extend_from_slice(b"\x1b[m"),
            }
            let n = rng.usize_range(1, 12);
            for _ in 0..n {
                line.push(words[rng.usize_range(0, words.len() - 1)]);
            }
            line.push(b' ');
        }
        line.extend_from_slice(b"\x1b[0m\r\n");
        if finished(w.write_all(&line).map(|()| 0))? {
            return Ok(());
        }
    }
}

/// How `+osc` frames its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OscStyle {
    /// Write all OSC data, including `ESC ]` and ST.
    Streaming,
    /// Only write data, prefixed with a little-endian usize length; used for
    /// testing just the OSC parser.
    Parser,
}

/// `ghostty-synthetic +osc`: endless OSC stream.
pub fn run_osc<W: Write>(
    w: &mut W,
    rng: &mut Rng,
    p_valid: f64,
    style: OscStyle,
) -> io::Result<()> {
    let generator = Osc::new(p_valid);
    let mut buf = [0u8; 1024];
    loop {
        let mut fixed = Fixed::new(&mut buf);
        generator.next(rng, &mut fixed, 1024)?;
        let data = fixed.buffered();
        let res = match style {
            OscStyle::Streaming => w.write_all(data),
            OscStyle::Parser => w
                .write_all(&(data.len() - 3).to_le_bytes())
                .and_then(|()| w.write_all(&data[2..data.len() - 1])),
        };
        if finished(res.map(|()| 0))? {
            return Ok(());
        }
    }
}

/// Broken pipe (stdout closed) ends the generator successfully.
fn finished(r: io::Result<usize>) -> io::Result<bool> {
    match r {
        Ok(_) => Ok(false),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(true),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghostty_vt::osc::Parser;

    const SEED: u64 = 0xC0FFEEEEEEEEEEEE;

    #[test]
    fn bytes() {
        let mut rng = Rng::new(0);
        let mut out = Vec::new();
        let v = Bytes {
            min_len: 256,
            max_len: 256,
            alphabet: None,
        };
        v.next(&mut rng, &mut out, 256).unwrap();
        assert_eq!(out.len(), 256);
    }

    #[test]
    fn utf8() {
        let mut rng = Rng::new(0);
        let mut out = Vec::new();
        let v = Utf8 {
            min_len: 256,
            max_len: 256,
            ..Utf8::default()
        };
        v.next(&mut rng, &mut out, 256).unwrap();
        assert_eq!(out.len(), 256);
        assert!(std::str::from_utf8(&out).is_ok());
    }

    #[test]
    fn osc_generator() {
        let mut rng = Rng::new(SEED);
        let mut buf = [0u8; 256];
        let generator = Osc::new(1.0);
        for _ in 0..50 {
            let mut w = Fixed::new(&mut buf);
            generator.next(&mut rng, &mut w, 256).unwrap();
            assert_ne!(w.buffered(), b"");
        }
    }

    #[test]
    fn osc_generator_valid() {
        let mut rng = Rng::new(SEED);
        let mut buf = [0u8; 256];
        let generator = Osc::new(1.0);
        for _ in 0..50 {
            let mut w = Fixed::new(&mut buf);
            generator.next(&mut rng, &mut w, 256).unwrap();
            let seq = w.buffered();
            let mut p = Parser::init(false);
            for &c in &seq[2..seq.len() - 1] {
                p.next(c);
            }
            assert!(p.end(None).is_some(), "{seq:?}");
        }
    }

    #[test]
    fn osc_generator_invalid() {
        let mut rng = Rng::new(SEED);
        let mut buf = [0u8; 256];
        let generator = Osc::new(0.0);
        for _ in 0..50 {
            let mut w = Fixed::new(&mut buf);
            generator.next(&mut rng, &mut w, 256).unwrap();
            let seq = w.buffered();
            let mut p = Parser::init(false);
            for &c in &seq[2..seq.len() - 1] {
                p.next(c);
            }
            assert!(p.end(None).is_none(), "{seq:?}");
        }
    }
}
