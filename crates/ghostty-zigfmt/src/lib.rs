//! Port of Zig 0.16 `std.fmt.parseInt`, `std.fmt.parseUnsigned` and
//! `std.fmt.parseFloat` (`fmt.zig`, `fmt/parse_float.zig` and
//! `fmt/parse_float/{parse,convert_hex,FloatStream}.zig`).
//!
//! The grammars are reproduced exactly: underscores, `0b`/`0o`/`0x`
//! prefixes (both cases, base 0 only), sign rules, hex floats, `inf` /
//! `infinity` / `nan`. Decimal float values are rounded correctly by the
//! standard library once the Zig grammar has accepted the input (Zig's
//! algorithm is also correctly rounded); hex floats follow `convertHex`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

/// `std.fmt.ParseIntError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseIntError {
    /// The result cannot fit in the type specified.
    Overflow,
    /// The input was empty or contained an invalid character.
    InvalidCharacter,
}

impl std::fmt::Display for ParseIntError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseIntError::Overflow => "Overflow",
            ParseIntError::InvalidCharacter => "InvalidCharacter",
        })
    }
}

impl std::error::Error for ParseIntError {}

/// `std.fmt.ParseFloatError`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseFloatError;

/// Integer types that can be targeted by [`parse_int`].
pub trait ZigInt: Sized + Copy {
    const MIN_I128: i128;
    const MAX_I128: i128;
    fn from_i128(v: i128) -> Self;
}

macro_rules! zig_int {
    ($($t:ty),*) => {$(
        impl ZigInt for $t {
            const MIN_I128: i128 = <$t>::MIN as i128;
            const MAX_I128: i128 = <$t>::MAX as i128;
            fn from_i128(v: i128) -> Self {
                v as $t
            }
        }
    )*};
}
zig_int!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

/// `std.fmt.charToDigit`.
fn char_to_digit(c: u8, base: u8) -> Result<u8, ParseIntError> {
    let value = match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'Z' => c - b'A' + 10,
        b'a'..=b'z' => c - b'a' + 10,
        _ => return Err(ParseIntError::InvalidCharacter),
    };
    if value >= base {
        return Err(ParseIntError::InvalidCharacter);
    }
    Ok(value)
}

/// `std.fmt.parseInt(T, buf, base)`.
pub fn parse_int<T: ZigInt>(buf: &[u8], base: u8) -> Result<T, ParseIntError> {
    match buf.first() {
        None => Err(ParseIntError::InvalidCharacter),
        Some(b'+') => parse_int_with_sign(&buf[1..], base, false),
        Some(b'-') => parse_int_with_sign(&buf[1..], base, true),
        Some(_) => parse_int_with_sign(buf, base, false),
    }
}

/// `std.fmt.parseUnsigned(T, buf, base)` (no sign accepted).
pub fn parse_unsigned<T: ZigInt>(buf: &[u8], base: u8) -> Result<T, ParseIntError> {
    parse_int_with_sign(buf, base, false)
}

fn parse_int_with_sign<T: ZigInt>(
    buf: &[u8],
    base: u8,
    negative: bool,
) -> Result<T, ParseIntError> {
    if buf.is_empty() {
        return Err(ParseIntError::InvalidCharacter);
    }

    let mut buf_base = base;
    let mut buf_start = buf;
    if base == 0 {
        // Treat is as a decimal number by default.
        buf_base = 10;
        // Detect the base by looking at buf prefix.
        if buf.len() > 2 && buf[0] == b'0' {
            match buf[1].to_ascii_lowercase() {
                b'b' => {
                    buf_base = 2;
                    buf_start = &buf[2..];
                }
                b'o' => {
                    buf_base = 8;
                    buf_start = &buf[2..];
                }
                b'x' => {
                    buf_base = 16;
                    buf_start = &buf[2..];
                }
                _ => {}
            }
        }
    }

    if buf_start[0] == b'_' || buf_start[buf_start.len() - 1] == b'_' {
        return Err(ParseIntError::InvalidCharacter);
    }

    let mut acc: i128 = 0;
    for &c in buf_start {
        if c == b'_' {
            continue;
        }
        let digit = i128::from(char_to_digit(c, buf_base)?);
        acc = acc
            .checked_mul(i128::from(buf_base))
            .ok_or(ParseIntError::Overflow)?;
        acc = if negative {
            acc.checked_sub(digit)
        } else {
            acc.checked_add(digit)
        }
        .ok_or(ParseIntError::Overflow)?;
        if acc < T::MIN_I128 || acc > T::MAX_I128 {
            return Err(ParseIntError::Overflow);
        }
    }
    Ok(T::from_i128(acc))
}

// ---------------------------------------------------------------------
// Floats

/// Float types that can be targeted by [`parse_float`].
pub trait ZigFloat: Sized + Copy + std::str::FromStr + std::fmt::Display {
    fn is_nan_value(self) -> bool;
    const MANTISSA_BITS: u32;
    const FRACTIONAL_BITS: u32;
    const EXP_BITS: u32;
    const EXP_MAX: i64;
    const EXP_MIN: i64;
    const INF: Self;
    const NAN: Self;
    fn neg(self) -> Self;
    fn zero(negative: bool) -> Self;
    fn from_bits64(bits: u64) -> Self;
}

macro_rules! zig_float {
    ($t:ty, $bits:ty, $m:expr, $e:expr, $max:expr, $min:expr) => {
        impl ZigFloat for $t {
            const MANTISSA_BITS: u32 = $m;
            const FRACTIONAL_BITS: u32 = $m;
            const EXP_BITS: u32 = $e;
            const EXP_MAX: i64 = $max;
            const EXP_MIN: i64 = $min;
            const INF: Self = <$t>::INFINITY;
            const NAN: Self = <$t>::NAN;
            fn is_nan_value(self) -> bool {
                self.is_nan()
            }
            fn neg(self) -> Self {
                -self
            }
            fn zero(negative: bool) -> Self {
                if negative { -0.0 } else { 0.0 }
            }
            fn from_bits64(bits: u64) -> Self {
                <$t>::from_bits(bits as $bits)
            }
        }
    };
}
zig_float!(f32, u32, 23, 8, 127, -126);
zig_float!(f64, u64, 52, 11, 1023, -1022);

struct Number {
    exponent: i64,
    mantissa: u64,
    many_digits: bool,
    hex: bool,
}

/// `FloatStream.zig`.
struct Stream<'a> {
    slice: &'a [u8],
    offset: usize,
    underscore_count: usize,
}

impl<'a> Stream<'a> {
    fn new(slice: &'a [u8]) -> Self {
        Stream {
            slice,
            offset: 0,
            underscore_count: 0,
        }
    }

    fn offset_true(&self) -> usize {
        self.offset - self.underscore_count
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.underscore_count = 0;
    }

    fn first(&self) -> Option<u8> {
        self.slice.get(self.offset).copied()
    }

    fn first_is(&self, cs: &[u8]) -> bool {
        self.first().is_some_and(|c| cs.contains(&c))
    }

    fn first_is_lower(&self, c: u8) -> bool {
        self.first().is_some_and(|ok| ok | 0x20 == c)
    }

    fn first_is_digit(&self, base: u8) -> bool {
        self.first().is_some_and(|c| is_digit(c, base))
    }

    fn advance(&mut self, n: usize) {
        self.offset += n;
    }

    fn scan_digit(&mut self, base: u8) -> Option<u8> {
        loop {
            let ok = self.first()?;
            if ok.is_ascii_digit() {
                self.advance(1);
                return Some(ok - b'0');
            } else if base == 16 && (b'a'..=b'f').contains(&ok) {
                self.advance(1);
                return Some(ok - b'a' + 10);
            } else if base == 16 && (b'A'..=b'F').contains(&ok) {
                self.advance(1);
                return Some(ok - b'A' + 10);
            } else if ok == b'_' {
                self.advance(1);
                self.underscore_count += 1;
            } else {
                return None;
            }
        }
    }
}

fn is_digit(c: u8, base: u8) -> bool {
    if base == 10 {
        c.is_ascii_digit()
    } else {
        c.is_ascii_hexdigit()
    }
}

fn try_parse_digits(stream: &mut Stream<'_>, x: &mut u64, base: u8) {
    while let Some(digit) = stream.scan_digit(base) {
        *x = x
            .wrapping_mul(u64::from(base))
            .wrapping_add(u64::from(digit));
    }
}

fn min_n_digit_int(digit_count: usize) -> u64 {
    let mut n: u64 = 1;
    for _ in 1..digit_count {
        n = n.wrapping_mul(10);
    }
    n
}

fn try_parse_n_digits(stream: &mut Stream<'_>, x: &mut u64, base: u8, n: usize) {
    while *x < min_n_digit_int(n) {
        if let Some(digit) = stream.scan_digit(base) {
            *x = x
                .wrapping_mul(u64::from(base))
                .wrapping_add(u64::from(digit));
        } else {
            break;
        }
    }
}

fn parse_scientific(stream: &mut Stream<'_>) -> Option<i64> {
    let mut exponent: i64 = 0;
    let mut negative = false;

    if let Some(c) = stream.first() {
        negative = c == b'-';
        if c == b'-' || c == b'+' {
            stream.advance(1);
        }
    }
    if stream.first_is_digit(10) {
        while let Some(digit) = stream.scan_digit(10) {
            // no overflows here, saturate well before overflow
            if exponent < 0x1000_0000 {
                exponent = 10 * exponent + i64::from(digit);
            }
        }
        return Some(if negative { -exponent } else { exponent });
    }
    None
}

fn valid_underscores(s: &[u8], base: u8) -> bool {
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'_' {
            // underscore at start of end
            if i == 0 || i + 1 == s.len() {
                return false;
            }
            // consecutive underscores
            if !is_digit(s[i - 1], base) || !is_digit(s[i + 1], base) {
                return false;
            }
            // next is guaranteed a digit, skip an extra
            i += 1;
        }
        i += 1;
    }
    true
}

fn parse_partial_number_base(
    stream: &mut Stream<'_>,
    n: &mut usize,
    base: u8,
    max_mantissa_digits: usize,
    exp_char_lower: u8,
) -> Option<Number> {
    // parse initial digits before dot
    let mut mantissa: u64 = 0;
    try_parse_digits(stream, &mut mantissa, base);
    let int_end = stream.offset_true();
    let mut n_digits = stream.offset_true() as isize;

    // handle dot with the following digits
    let mut exponent: i64 = 0;
    if stream.first_is(b".") {
        stream.advance(1);
        let marker = stream.offset_true();
        try_parse_digits(stream, &mut mantissa, base);
        let n_after_dot = stream.offset_true() - marker;
        exponent = -(n_after_dot as i64);
        n_digits += n_after_dot as isize;
    }

    // adjust required shift to offset mantissa for base-16 (2^4)
    if base == 16 {
        exponent *= 4;
    }

    if n_digits == 0 {
        return None;
    }

    // handle scientific format
    let mut exp_number: i64 = 0;
    if stream.first_is_lower(exp_char_lower) {
        stream.advance(1);
        exp_number = parse_scientific(stream)?;
        exponent += exp_number;
    }

    // length must be complete parsed length
    *n += stream.offset;

    if stream.underscore_count > 0 && !valid_underscores(stream.slice, base) {
        return None;
    }

    // common case with not many digits
    if n_digits <= max_mantissa_digits as isize {
        return Some(Number {
            exponent,
            mantissa,
            many_digits: false,
            hex: base == 16,
        });
    }

    n_digits -= max_mantissa_digits as isize;
    let mut many_digits = false;
    stream.reset(); // re-parse from beginning
    while stream.first_is(b"0._") {
        let next = stream.first().unwrap_or(0);
        if next == b'_' {
            stream.underscore_count += 1;
        } else {
            n_digits -= isize::from(next.saturating_sub(b'0' - 1));
        }
        stream.advance(1);
    }
    if n_digits > 0 {
        // at this point we have more than max_mantissa_digits significant digits
        many_digits = true;
        mantissa = 0;
        stream.reset();
        try_parse_n_digits(stream, &mut mantissa, base, max_mantissa_digits);

        exponent = if mantissa >= min_n_digit_int(max_mantissa_digits) {
            // big int
            int_end as i64 - stream.offset_true() as i64
        } else {
            // the next byte must be present and be '.'
            stream.advance(1);
            let marker = stream.offset_true();
            try_parse_n_digits(stream, &mut mantissa, base, max_mantissa_digits);
            marker as i64 - stream.offset_true() as i64
        };
        if base == 16 {
            exponent *= 4;
        }
        // add back the explicit part
        exponent += exp_number;
    }

    Some(Number {
        exponent,
        mantissa,
        many_digits,
        hex: base == 16,
    })
}

/// Returns the parsed hex-float `Number`, or a validated decimal slice.
enum Parsed {
    Hex(Number),
    Decimal,
}

fn parse_number(s: &[u8]) -> Option<Parsed> {
    let mut consumed: usize = 0;
    let parsed = if s.len() >= 2 && s[0] == b'0' && s[1].eq_ignore_ascii_case(&b'x') {
        let mut stream = Stream::new(&s[2..]);
        consumed += 2;
        parse_partial_number_base(&mut stream, &mut consumed, 16, 16, b'p').map(Parsed::Hex)
    } else {
        let mut stream = Stream::new(s);
        parse_partial_number_base(&mut stream, &mut consumed, 10, 19, b'e').map(|_| Parsed::Decimal)
    };
    // must consume entire float (no trailing data)
    if s.len() == consumed { parsed } else { None }
}

fn parse_inf_or_nan<T: ZigFloat>(s: &[u8], negative: bool) -> Option<T> {
    let starts = |s: &[u8], p: &[u8]| s.len() >= p.len() && s[..p.len()].eq_ignore_ascii_case(p);
    if starts(s, b"inf") {
        let consumed = if starts(&s[3..], b"inity") { 8 } else { 3 };
        if s.len() != consumed {
            return None;
        }
        return Some(if negative { T::INF.neg() } else { T::INF });
    }
    if starts(s, b"nan") {
        return (s.len() == 3).then_some(T::NAN);
    }
    None
}

/// `convert_hex.zig`.
fn convert_hex<T: ZigFloat>(n_: &Number, negative: bool) -> T {
    let mut mantissa = n_.mantissa;
    let mut exponent = n_.exponent;

    if mantissa == 0 {
        return T::zero(negative);
    }

    let max_exp = T::EXP_MAX;
    let min_exp = T::EXP_MIN;
    let mantissa_bits = T::MANTISSA_BITS;
    let fractional_bits = T::FRACTIONAL_BITS;
    let exp_bits = T::EXP_BITS;
    let exp_bias = min_exp - 1;

    // mantissa now implicitly divided by 2^fractional_bits
    exponent += i64::from(fractional_bits);

    while mantissa != 0 && mantissa >> (mantissa_bits + 2) == 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    if n_.many_digits {
        mantissa |= 1;
    }
    while mantissa >> (1 + fractional_bits + 2) != 0 {
        mantissa = (mantissa >> 1) | (mantissa & 1);
        exponent += 1;
    }

    // If exponent is too negative, denormalize in hopes of making it
    // representable. (The -2 is for the rounding bits.)
    while mantissa > 1 && exponent < min_exp - 2 {
        mantissa = (mantissa >> 1) | (mantissa & 1);
        exponent += 1;
    }

    // Round using two bottom bits.
    let mut round = mantissa & 3;
    mantissa >>= 2;
    round |= mantissa & 1; // round to even (round up if mantissa is odd)
    exponent += 2;
    if round == 3 {
        mantissa += 1;
        if mantissa == 1 << (1 + fractional_bits) {
            mantissa >>= 1;
            exponent += 1;
        }
    }

    // Denormal or zero
    if mantissa >> fractional_bits == 0 {
        exponent = exp_bias;
    }

    // Infinity and range error
    if exponent > max_exp {
        return if negative { T::INF.neg() } else { T::INF };
    }

    let mut bits = mantissa & ((1u64 << mantissa_bits) - 1);
    bits |= (((exponent - exp_bias) as u64) & ((1u64 << exp_bits) - 1)) << mantissa_bits;
    if negative {
        bits |= 1u64 << (mantissa_bits + exp_bits);
    }
    T::from_bits64(bits)
}

/// Zig's `{d}` for floats: like Rust's `Display` except NaN is `nan`
/// (infinities are `inf` / `-inf` in both).
#[must_use]
pub fn format_float_d<T: ZigFloat>(v: T) -> String {
    if v.is_nan_value() {
        "nan".to_string()
    } else {
        v.to_string()
    }
}

/// `std.fmt.parseFloat(T, s)`.
pub fn parse_float<T: ZigFloat>(s: &[u8]) -> Result<T, ParseFloatError> {
    if s.is_empty() {
        return Err(ParseFloatError);
    }

    let mut i = 0;
    let negative = s[i] == b'-';
    if s[i] == b'-' || s[i] == b'+' {
        i += 1;
    }
    if s.len() == i {
        return Err(ParseFloatError);
    }
    let s = &s[i..];

    match parse_number(s) {
        None => parse_inf_or_nan::<T>(s, negative).ok_or(ParseFloatError),
        Some(Parsed::Hex(n)) => {
            debug_assert!(n.hex);
            Ok(convert_hex::<T>(&n, negative))
        }
        Some(Parsed::Decimal) => {
            // The grammar is validated; underscores are ignorable now.
            let cleaned: String = s
                .iter()
                .filter(|&&c| c != b'_')
                .map(|&c| char::from(c))
                .collect();
            let v: T = cleaned.parse().map_err(|_| ParseFloatError)?;
            Ok(if negative { v.neg() } else { v })
        }
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    fn pi<T: ZigInt>(s: &str, base: u8) -> Result<T, ParseIntError> {
        parse_int::<T>(s.as_bytes(), base)
    }

    #[test]
    fn parse_int_zig_std_test() {
        assert_eq!(pi::<i32>("-10", 10), Ok(-10));
        assert_eq!(pi::<i32>("+10", 10), Ok(10));
        assert_eq!(pi::<u32>("+10", 10), Ok(10));
        assert_eq!(pi::<u32>("-10", 10), Err(ParseIntError::Overflow));
        for bad in [" 10", "10 ", "_10_", "0x_10_", "0x10_", "0x_10"] {
            assert_eq!(pi::<u32>(bad, 10), Err(ParseIntError::InvalidCharacter));
        }
        assert_eq!(pi::<u8>("255", 10), Ok(255));
        assert_eq!(pi::<u8>("256", 10), Err(ParseIntError::Overflow));

        // +0 and -0 should work for unsigned
        assert_eq!(pi::<u8>("-0", 10), Ok(0));
        assert_eq!(pi::<u8>("+0", 10), Ok(0));

        assert_eq!(pi::<i8>("-128", 10), Ok(-128));
        assert_eq!(pi::<i64>("-4398046511104", 10), Ok(-4_398_046_511_104));

        // empty string or bare +- is invalid
        for bad in ["", "+", "-"] {
            assert_eq!(pi::<u32>(bad, 10), Err(ParseIntError::InvalidCharacter));
            assert_eq!(pi::<i32>(bad, 10), Err(ParseIntError::InvalidCharacter));
        }

        // autodectect the base
        assert_eq!(pi::<i32>("111", 0), Ok(111));
        assert_eq!(pi::<i32>("1_1_1", 0), Ok(111));
        assert_eq!(pi::<i32>("+0b111", 0), Ok(7));
        assert_eq!(pi::<i32>("+0B111", 0), Ok(7));
        assert_eq!(pi::<i32>("+0b1_11", 0), Ok(7));
        assert_eq!(pi::<i32>("+0o111", 0), Ok(73));
        assert_eq!(pi::<i32>("+0O111", 0), Ok(73));
        assert_eq!(pi::<i32>("+0o11_1", 0), Ok(73));
        assert_eq!(pi::<i32>("+0x111", 0), Ok(273));
        assert_eq!(pi::<i32>("-0b111", 0), Ok(-7));
        assert_eq!(pi::<i32>("-0b11_1", 0), Ok(-7));
        assert_eq!(pi::<i32>("-0o111", 0), Ok(-73));
        assert_eq!(pi::<i32>("-0x111", 0), Ok(-273));
        assert_eq!(pi::<i32>("-0X111", 0), Ok(-273));
        assert_eq!(pi::<i32>("-0x1_11", 0), Ok(-273));

        // bare binary/octal/decimal prefix is invalid
        for bad in ["0b", "0o", "0x"] {
            assert_eq!(pi::<u32>(bad, 0), Err(ParseIntError::InvalidCharacter));
        }

        assert_eq!(pi::<i8>("-10", 2), Ok(-2));
        assert_eq!(pi::<i8>("-10", 8), Ok(-8));
        assert_eq!(pi::<i8>("-10", 16), Ok(-16));
    }

    #[test]
    fn parse_unsigned_zig_std_test() {
        let pu = |s: &str, b| parse_unsigned::<u64>(s.as_bytes(), b);
        assert_eq!(parse_unsigned::<u16>(b"050124", 10), Ok(50124));
        assert_eq!(parse_unsigned::<u16>(b"65535", 10), Ok(65535));
        assert_eq!(parse_unsigned::<u16>(b"65_535", 10), Ok(65535));
        assert_eq!(
            parse_unsigned::<u16>(b"65536", 10),
            Err(ParseIntError::Overflow)
        );
        assert_eq!(pu("0ffffffffffffffff", 16), Ok(u64::MAX));
        assert_eq!(pu("0f_fff_fff_fff_fff_fff", 16), Ok(u64::MAX));
        assert_eq!(pu("10000000000000000", 16), Err(ParseIntError::Overflow));
        assert_eq!(parse_unsigned::<u32>(b"DeadBeef", 16), Ok(0xDEAD_BEEF));
        assert_eq!(
            parse_unsigned::<u32>(b"f", 10),
            Err(ParseIntError::InvalidCharacter)
        );
        assert_eq!(
            parse_unsigned::<u8>(b"109", 8),
            Err(ParseIntError::InvalidCharacter)
        );
        assert_eq!(parse_unsigned::<u32>(b"NUMBER", 36), Ok(1_442_151_747));
        // parseUnsigned does not expect a sign
        assert_eq!(
            parse_unsigned::<u8>(b"+0", 10),
            Err(ParseIntError::InvalidCharacter)
        );
        assert_eq!(
            parse_unsigned::<u8>(b"-0", 10),
            Err(ParseIntError::InvalidCharacter)
        );
        assert_eq!(
            parse_unsigned::<u8>(b"", 10),
            Err(ParseIntError::InvalidCharacter)
        );
    }

    #[test]
    fn parse_int_non_ascii_and_prefix_edge_cases() {
        // `buf.len > 2` gates prefix detection: "0x" alone is decimal "0x".
        assert_eq!(pi::<u32>("0x1", 0), Ok(1));
        assert_eq!(pi::<u32>("0_1", 0), Ok(1));
        assert_eq!(pi::<u32>("1__0", 0), Ok(10));
        assert_eq!(pi::<u32>("0X10", 0), Ok(16));
        assert_eq!(
            parse_int::<u32>("é1".as_bytes(), 0),
            Err(ParseIntError::InvalidCharacter)
        );
        // Prefixes are only honoured with base 0.
        assert_eq!(pi::<u32>("0x10", 10), Err(ParseIntError::InvalidCharacter));
        assert_eq!(pi::<u32>("0b11", 16), Ok(0xb11));
    }

    fn pf64(s: &str) -> Result<f64, ParseFloatError> {
        parse_float::<f64>(s.as_bytes())
    }
    fn pf32(s: &str) -> Result<f32, ParseFloatError> {
        parse_float::<f32>(s.as_bytes())
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-7
    }

    #[test]
    fn parse_float_zig_std_test() {
        for bad in ["", "   1", "1abc", "+", "-"] {
            assert!(pf64(bad).is_err(), "{bad:?}");
            assert!(pf32(bad).is_err(), "{bad:?}");
        }
        assert_eq!(pf64("0"), Ok(0.0));
        assert_eq!(pf64("+0"), Ok(0.0));
        assert_eq!(pf64("-0"), Ok(0.0));
        assert!(pf64("-0").unwrap().is_sign_negative());
        assert_eq!(pf64("0e0"), Ok(0.0));
        assert_eq!(pf64("2e3"), Ok(2000.0));
        assert_eq!(pf64("1e0"), Ok(1.0));
        assert_eq!(pf64("-2e3"), Ok(-2000.0));
        assert_eq!(pf64("-1e0"), Ok(-1.0));
        assert_eq!(pf64("1.234e3"), Ok(1234.0));
        assert!(approx(pf64("3.141").unwrap(), 3.141));
        assert!(approx(pf64("-3.141").unwrap(), -3.141));
        assert_eq!(pf64("1e-5000"), Ok(0.0));
        assert_eq!(pf64("1e+5000"), Ok(f64::INFINITY));
        assert_eq!(
            pf64("0.4e0066999999999999999999999999999999999999999999999999999"),
            Ok(f64::INFINITY)
        );
        assert!(approx(
            pf64("0_1_2_3_4_5_6.7_8_9_0_0_0e0_0_1_0").unwrap() / 1e10,
            123_456.789
        ));
        // underscore rule: only between two digits, never repeated.
        for bad in [
            "0123456.789000e_0010",
            "_0123456.789000e0010",
            "0__123456.789000e_0010",
            "0123456_.789000e0010",
            "0123456.789000e0010_",
        ] {
            assert!(pf64(bad).is_err(), "{bad:?}");
        }
        assert!(approx(pf64("1e-2").unwrap(), 0.01));
        assert!(approx(pf64("1234e-2").unwrap(), 12.34));
        assert!(approx(pf64("1.").unwrap(), 1.0));
        assert!(approx(pf64("0.").unwrap(), 0.0));
        assert!(approx(pf64(".1").unwrap(), 0.1));
        assert!(approx(pf64(".0").unwrap(), 0.0));
        assert!(approx(pf64(".1e-1").unwrap(), 0.01));
        for bad in [".", ".e1", "0.e"] {
            assert!(pf64(bad).is_err(), "{bad:?}");
        }
        assert!(approx(pf64("123142.1").unwrap(), 123_142.1));
        assert!(approx(pf64("-123142.1124").unwrap(), -123_142.112_4));
        assert!(approx(
            pf64("2.71828182845904523536").unwrap(),
            2.718_281_828_459_045
        ));
    }

    #[test]
    fn parse_float_nan_and_inf() {
        assert!(pf64("nAn").unwrap().is_nan());
        assert_eq!(pf64("inF"), Ok(f64::INFINITY));
        assert_eq!(pf64("-INF"), Ok(f64::NEG_INFINITY));
        assert_eq!(pf64("+Inf"), Ok(f64::INFINITY));
        assert_eq!(pf32("-iNf"), Ok(f32::NEG_INFINITY));
        assert_eq!(pf64("infinity"), Ok(f64::INFINITY));
        assert_eq!(pf64("-InFiNiTy"), Ok(f64::NEG_INFINITY));
        assert!(pf64("-nan").unwrap().is_nan());
        for bad in ["infx", "infinit", "nanx", "in", "na", "infinityy", "i_nf"] {
            assert!(pf64(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn format_float_d_matches_zig() {
        assert_eq!(format_float_d(f64::NAN), "nan");
        assert_eq!(format_float_d(-f32::NAN), "nan");
        assert_eq!(format_float_d(f64::INFINITY), "inf");
        assert_eq!(format_float_d(f64::NEG_INFINITY), "-inf");
        assert_eq!(format_float_d(13.5f32), "13.5");
        assert_eq!(format_float_d(-0.0f64), "-0");
    }

    #[test]
    fn parse_float_largest_normals() {
        assert_eq!(pf32("3.4028234664E38").unwrap().to_bits(), 0x7f7f_ffff);
        assert_eq!(
            pf64("1.7976931348623157E308").unwrap().to_bits(),
            0x7fef_ffff_ffff_ffff
        );
    }

    #[test]
    fn parse_float_hex_special_and_zero() {
        assert_eq!(pf32("0x9999p9999"), Ok(f32::INFINITY));
        assert_eq!(pf32("-0x9999p9999"), Ok(f32::NEG_INFINITY));
        assert_eq!(pf32("0x0"), Ok(0.0));
        assert_eq!(pf32("-0x0"), Ok(0.0));
        assert_eq!(pf32("0x0p42"), Ok(0.0));
        assert_eq!(pf32("-0x0.00000p42"), Ok(0.0));
        assert_eq!(pf32("0x0.00000p666"), Ok(0.0));
    }

    #[test]
    fn parse_float_hex_f32() {
        assert!(pf32("0x").is_err());
        assert_eq!(pf32("0x1p0"), Ok(1.0));
        assert_eq!(pf32("-0x1p-1"), Ok(-0.5));
        assert_eq!(pf32("0x10p+10"), Ok(16384.0));
        assert_eq!(pf32("0x10p-10"), Ok(0.015_625));
        assert_eq!(pf32("0x0.ffffffp128"), Ok(hexf(0xff_ffff, 128 - 24)));
        assert_eq!(pf32("0x1.fffffeP+127"), Ok(f32::MAX));
        assert_eq!(pf32("-0x1.fffffeP+127"), Ok(-f32::MAX));
        assert_eq!(pf32("0x1p-126"), Ok(f32::MIN_POSITIVE));
        assert_eq!(pf32("-0x1p-126"), Ok(-f32::MIN_POSITIVE));
        assert_eq!(pf32("0x1P-149"), Ok(f32::from_bits(1)));
        assert_eq!(pf32("-0x1P-149"), Ok(-f32::from_bits(1)));
    }

    fn hexf(m: u32, e: i32) -> f32 {
        m as f32 * 2f32.powi(e)
    }

    #[test]
    fn parse_float_hex_f64() {
        assert_eq!(pf64("0x1p0"), Ok(1.0));
        assert_eq!(pf64("-0x1p-1"), Ok(-0.5));
        assert_eq!(pf64("0x10p+10"), Ok(16384.0));
        assert_eq!(pf64("0x10p-10"), Ok(0.015_625));
        assert_eq!(pf64("0x1.fffffffffffffp+1023"), Ok(f64::MAX));
        assert_eq!(pf64("-0x1.fffffffffffffp1023"), Ok(-f64::MAX));
        assert_eq!(pf64("0x1p-1022"), Ok(f64::MIN_POSITIVE));
        assert_eq!(pf64("-0x1p-1022"), Ok(-f64::MIN_POSITIVE));
        assert_eq!(pf64("0x1p-1074"), Ok(f64::from_bits(1)));
        assert_eq!(pf64("-0x1p-1074"), Ok(-f64::from_bits(1)));
    }

    #[test]
    fn parse_float_hex_with_underscores_and_many_digits() {
        assert_eq!(pf64("0x1.cp+3"), Ok(14.0));
        assert_eq!(pf32("0x1.cp+3"), Ok(14.0));
        assert_eq!(pf64("0X1_0"), Ok(16.0));
        assert_eq!(pf64("0x1_0p1_0"), Ok(16.0 * 1024.0));
        assert!(pf64("0x_1").is_err());
        assert!(pf64("0x1_").is_err());
        assert!(pf64("0x1p_1").is_err());
        assert!(pf64("0x1__0").is_err());
        // many_digits: result must match between f32 and a direct f32 round.
        let a = pf32("0xffffffffffffffff.0p0").unwrap();
        assert_eq!(a, 1.844_674_4e19_f32);
        // Decimal numbers with underscores.
        assert_eq!(pf32("1_4"), Ok(14.0));
        assert!(pf32("1__4").is_err());
    }
}
