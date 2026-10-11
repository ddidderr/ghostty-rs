//! Character sets (ZIG/src/terminal/charsets.zig).

/// The available charset slots for a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slots {
    G0,
    G1,
    G2,
    G3,
}

/// The name of the active slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActiveSlot {
    GL,
    GR,
}

/// The list of supported character sets and their associated tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Charset {
    Utf8,
    Ascii,
    British,
    DecSpecial,
}

/// Our table length is 256 so we can contain all ASCII chars.
pub const TABLE_LEN: usize = 256;

/// The table for the given charset. The returned table is guaranteed to be
/// 256 entries that can be used to map ASCII to the given charset.
///
/// # Panics
/// Panics for `Charset::Utf8`: utf8 is not a table, callers should
/// double-check if the charset is utf8 and NOT use tables.
pub fn table(set: Charset) -> &'static [u16; TABLE_LEN] {
    match set {
        Charset::British => &BRITISH,
        Charset::DecSpecial => &DEC_SPECIAL,
        Charset::Utf8 => unreachable!("utf8 has no table"),
        Charset::Ascii => &ASCII,
    }
}

/// Creates a table that maps ASCII to ASCII as a getting started point.
const fn init_table() -> [u16; TABLE_LEN] {
    let mut result = [0u16; TABLE_LEN];
    let mut i = 0;
    while i < TABLE_LEN {
        result[i] = i as u16;
        i += 1;
    }
    result
}

/// Just a basic c => c ascii table
static ASCII: [u16; TABLE_LEN] = init_table();

/// https://vt100.net/docs/vt220-rm/chapter2.html
static BRITISH: [u16; TABLE_LEN] = {
    let mut t = init_table();
    t[0x23] = 0x00a3;
    t
};

/// https://en.wikipedia.org/wiki/DEC_Special_Graphics
static DEC_SPECIAL: [u16; TABLE_LEN] = {
    let mut t = init_table();
    t[0x60] = 0x25C6;
    t[0x61] = 0x2592;
    t[0x62] = 0x2409;
    t[0x63] = 0x240C;
    t[0x64] = 0x240D;
    t[0x65] = 0x240A;
    t[0x66] = 0x00B0;
    t[0x67] = 0x00B1;
    t[0x68] = 0x2424;
    t[0x69] = 0x240B;
    t[0x6a] = 0x2518;
    t[0x6b] = 0x2510;
    t[0x6c] = 0x250C;
    t[0x6d] = 0x2514;
    t[0x6e] = 0x253C;
    t[0x6f] = 0x23BA;
    t[0x70] = 0x23BB;
    t[0x71] = 0x2500;
    t[0x72] = 0x23BC;
    t[0x73] = 0x23BD;
    t[0x74] = 0x251C;
    t[0x75] = 0x2524;
    t[0x76] = 0x2534;
    t[0x77] = 0x252C;
    t[0x78] = 0x2502;
    t[0x79] = 0x2264;
    t[0x7a] = 0x2265;
    t[0x7b] = 0x03C0;
    t[0x7c] = 0x2260;
    t[0x7d] = 0x00A3;
    t[0x7e] = 0x00B7;
    t
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lengths() {
        for set in [Charset::Ascii, Charset::British, Charset::DecSpecial] {
            assert_eq!(table(set).len(), 256);
        }
    }

    #[test]
    fn table_contents() {
        assert_eq!(table(Charset::British)[0x23], 0xa3);
        assert_eq!(table(Charset::Ascii)[0x41], 0x41);
        assert_eq!(table(Charset::DecSpecial)[0x71], 0x2500);
    }
}
