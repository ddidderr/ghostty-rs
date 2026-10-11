//! Owned OSC 8 hyperlinks and per-page identifiers.
/// Nonzero page-local hyperlink identifier.
pub type Id = u16;
/// Mapping from a cell index to its hyperlink.
pub type Map = std::collections::HashMap<usize, Id>;
/// An explicit OSC 8 ID or an automatically generated screen-local ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HyperlinkId {
    /// Application-provided byte string.
    Explicit(Vec<u8>),
    /// Screen-generated identifier.
    Implicit(u32),
}
/// A decoded hyperlink owning its ID and URI.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hyperlink {
    /// Hyperlink identity.
    pub id: HyperlinkId,
    /// URI bytes, which need not be UTF-8.
    pub uri: Vec<u8>,
}
impl Hyperlink {
    /// Hash the same tag, ID, URI, and slice lengths as Zig's page entry.
    pub(crate) fn zig_hash(&self) -> u64 {
        let implicit;
        let explicit_len;
        let uri_len = (self.uri.len() as u64).to_le_bytes();
        let parts: [&[u8]; 5] = match &self.id {
            HyperlinkId::Explicit(id) => {
                explicit_len = (id.len() as u64).to_le_bytes();
                [&[0], id, &explicit_len, &self.uri, &uri_len]
            }
            HyperlinkId::Implicit(id) => {
                implicit = id.to_le_bytes();
                [&[1], &implicit, &[], &self.uri, &uri_len]
            }
        };
        wyhash_parts(&parts)
    }
    /// Duplicate the owned strings.
    pub fn dupe(&self) -> Self {
        self.clone()
    }
    /// Compare identity and URI.
    pub fn eql(&self, other: &Self) -> bool {
        self == other
    }
    /// Logical string allocation including 32-byte chunk rounding.
    pub fn string_bytes(&self) -> usize {
        self.uri.len().next_multiple_of(32)
            + match &self.id {
                HyperlinkId::Explicit(v) => v.len().next_multiple_of(32),
                HyperlinkId::Implicit(_) => 0,
            }
    }
}
/// Owned page entry; unlike Zig it needs no base address for decoding.
pub type PageEntry = Hyperlink;
/// Reference-counted set of page entries.
pub type Set = crate::ref_counted_set::RefCountedSet<PageEntry>;

/// Wyhash over discontiguous borrowed strings. The virtual concatenation avoids
/// allocating a temporary buffer for every linked cell during reflow.
fn wyhash_parts(parts: &[&[u8]]) -> u64 {
    const SECRET: [u64; 4] = [
        0xa0761d6478bd642f,
        0xe7037ed1a0b428db,
        0x8ebc6af09c88c6e3,
        0x589965cc75374cc3,
    ];
    fn mix(a: u64, b: u64) -> u64 {
        let product = u128::from(a) * u128::from(b);
        product as u64 ^ (product >> 64) as u64
    }
    let len: usize = parts.iter().map(|part| part.len()).sum();
    let byte = |mut index: usize| {
        for part in parts {
            if index < part.len() {
                return part[index];
            }
            index -= part.len();
        }
        unreachable!("hash byte in bounds")
    };
    let read = |offset: usize, count: usize| -> u64 {
        (0..count).fold(0, |value, i| value | u64::from(byte(offset + i)) << (8 * i))
    };
    let seed = mix(SECRET[0], SECRET[1]);
    let mut state = [seed; 3];
    let (mut a, mut b);
    if len <= 16 {
        if len >= 4 {
            let end = len - 4;
            let quarter = (len >> 3) << 2;
            a = (read(0, 4) << 32) | read(quarter, 4);
            b = (read(end, 4) << 32) | read(end - quarter, 4);
        } else if len > 0 {
            a = (u64::from(byte(0)) << 16)
                | (u64::from(byte(len >> 1)) << 8)
                | u64::from(byte(len - 1));
            b = 0;
        } else {
            a = 0;
            b = 0;
        }
    } else {
        let mut offset = 0;
        while offset + 48 < len {
            for i in 0..3 {
                state[i] = mix(
                    read(offset + 16 * i, 8) ^ SECRET[i + 1],
                    read(offset + 16 * i + 8, 8) ^ state[i],
                );
            }
            offset += 48;
        }
        state[0] ^= state[1] ^ state[2];
        while offset + 16 < len {
            state[0] = mix(read(offset, 8) ^ SECRET[1], read(offset + 8, 8) ^ state[0]);
            offset += 16;
        }
        a = read(len - 16, 8);
        b = read(len - 8, 8);
    }
    a ^= SECRET[1];
    b ^= state[0];
    let product = u128::from(a) * u128::from(b);
    mix(
        product as u64 ^ SECRET[0] ^ len as u64,
        (product >> 64) as u64 ^ SECRET[1],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wyhash_matches_zig_at_block_boundaries() {
        // Generated with std.hash.Wyhash.hash(0, bytes[0..len]) in Zig.
        let bytes: Vec<u8> = (0..193).collect();
        for (len, expected) in [
            (0, 0x0409638ee2bde459),
            (1, 0xfbe5af10e5f8bd85),
            (3, 0xf971e76c35096d43),
            (4, 0xed0d4340e81b7c4d),
            (16, 0xff5ae257316b07b5),
            (17, 0x7ed011944b0c00ad),
            (32, 0x129f13f84e73df6d),
            (48, 0xce6cc055c4aa2354),
            (49, 0xdd118c769fb13542),
            (64, 0xae9ac4f9962d6746),
            (96, 0x4acbe2a4e0e44872),
            (97, 0xad119c0496db2624),
            (193, 0xab0aa27f27ed9125),
        ] {
            for split in 0..=len {
                assert_eq!(
                    wyhash_parts(&[&bytes[..split], &[], &bytes[split..len]]),
                    expected,
                    "len={len}, split={split}"
                );
            }
        }
    }

    #[test]
    fn hyperlink_hash_matches_zig_auto_hash_encoding() {
        // Zig autoHash(tag), autoHash/autoHashStrat(ID), autoHashStrat(URI).
        let mut link = Hyperlink {
            id: HyperlinkId::Implicit(42),
            uri: b"https://example.com".to_vec(),
        };
        assert_eq!(link.zig_hash(), 0x2482ebf45efe791b);
        link.id = HyperlinkId::Explicit(b"id=sample".to_vec());
        assert_eq!(link.zig_hash(), 0x80dc43ba27d83463);
    }
}
