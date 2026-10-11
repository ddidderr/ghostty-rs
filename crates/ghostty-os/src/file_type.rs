//! Port of `src/file_type.zig`.

struct TypeDetails {
    typ: FileType,
    sigs: &'static [&'static [Option<u8>]],
    exts: &'static [&'static str],
}

#[allow(clippy::unnecessary_wraps)]
const fn b(v: u8) -> Option<u8> {
    Some(v)
}

const TYPE_DETAILS: &[TypeDetails] = &[
    TypeDetails {
        typ: FileType::Jpeg,
        sigs: &[
            &[b(0xFF), b(0xD8), b(0xFF), b(0xDB)],
            &[
                b(0xFF),
                b(0xD8),
                b(0xFF),
                b(0xE0),
                b(0x00),
                b(0x10),
                b(0x4A),
                b(0x46),
                b(0x49),
                b(0x46),
                b(0x00),
                b(0x01),
            ],
            &[b(0xFF), b(0xD8), b(0xFF), b(0xEE)],
            &[
                b(0xFF),
                b(0xD8),
                b(0xFF),
                b(0xE1),
                None,
                None,
                b(0x45),
                b(0x78),
                b(0x69),
                b(0x66),
                b(0x00),
                b(0x00),
            ],
            &[b(0xFF), b(0xD8), b(0xFF), b(0xE0)],
        ],
        exts: &[".jpg", ".jpeg", ".jfif"],
    },
    TypeDetails {
        typ: FileType::Png,
        sigs: &[&[
            b(0x89),
            b(0x50),
            b(0x4E),
            b(0x47),
            b(0x0D),
            b(0x0A),
            b(0x1A),
            b(0x0A),
        ]],
        exts: &[".png"],
    },
    TypeDetails {
        typ: FileType::Gif,
        sigs: &[
            &[b(b'G'), b(b'I'), b(b'F'), b(b'8'), b(b'7'), b(b'a')],
            &[b(b'G'), b(b'I'), b(b'F'), b(b'8'), b(b'9'), b(b'a')],
        ],
        exts: &[".gif"],
    },
    TypeDetails {
        typ: FileType::Bmp,
        sigs: &[&[b(b'B'), b(b'M')]],
        exts: &[".bmp"],
    },
    TypeDetails {
        typ: FileType::Qoi,
        sigs: &[&[b(b'q'), b(b'o'), b(b'i'), b(b'f')]],
        exts: &[".qoi"],
    },
    TypeDetails {
        typ: FileType::Webp,
        sigs: &[&[
            b(0x52),
            b(0x49),
            b(0x46),
            b(0x46),
            None,
            None,
            None,
            None,
            b(0x57),
            b(0x45),
            b(0x42),
            b(0x50),
        ]],
        exts: &[".webp"],
    },
];

/// This is a helper for detecting file types based on magic bytes.
///
/// Ref: https://en.wikipedia.org/wiki/List_of_file_signatures
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// JPEG image file.
    Jpeg,
    /// PNG image file.
    Png,
    /// GIF image file.
    Gif,
    /// BMP image file.
    Bmp,
    /// QOI image file.
    Qoi,
    /// WebP image file.
    Webp,
    /// Unknown file format.
    Unknown,
}

impl FileType {
    /// Detect file type based on the magic bytes
    /// at the start of the provided file contents.
    pub fn detect(contents: &[u8]) -> FileType {
        for typ in TYPE_DETAILS {
            for signature in typ.sigs {
                if contents.len() >= signature.len()
                    && contents
                        .iter()
                        .zip(signature.iter())
                        .all(|(f, sig)| sig.is_none_or(|s| *f == s))
                {
                    return typ.typ;
                }
            }
        }
        FileType::Unknown
    }

    /// Guess file type from its extension.
    pub fn guess_from_extension(extension: &str) -> FileType {
        for typ in TYPE_DETAILS {
            for ext in typ.exts {
                if extension.eq_ignore_ascii_case(ext) {
                    return typ.typ;
                }
            }
        }
        FileType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Zig file has no tests; these cover the detection logic.
    #[test]
    fn detect_magic_bytes() {
        assert_eq!(
            FileType::detect(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1]),
            FileType::Png
        );
        assert_eq!(FileType::detect(b"GIF89a...."), FileType::Gif);
        assert_eq!(FileType::detect(b"BM"), FileType::Bmp);
        assert_eq!(FileType::detect(b"qoif"), FileType::Qoi);
        assert_eq!(
            FileType::detect(b"RIFF\x01\x02\x03\x04WEBP"),
            FileType::Webp
        );
        assert_eq!(
            FileType::detect(b"RIFF\x01\x02\x03\x04WEBX"),
            FileType::Unknown
        );
        assert_eq!(
            FileType::detect(&[0xFF, 0xD8, 0xFF, 0xE1, 9, 9, 0x45, 0x78, 0x69, 0x66, 0, 0]),
            FileType::Jpeg
        );
        assert_eq!(FileType::detect(&[0xFF, 0xD8, 0xFF, 0xE0]), FileType::Jpeg);
        assert_eq!(FileType::detect(b""), FileType::Unknown);
    }

    #[test]
    fn guess_from_extension_case_insensitive() {
        assert_eq!(FileType::guess_from_extension(".JPG"), FileType::Jpeg);
        assert_eq!(FileType::guess_from_extension(".jfif"), FileType::Jpeg);
        assert_eq!(FileType::guess_from_extension(".webp"), FileType::Webp);
        assert_eq!(FileType::guess_from_extension(".txt"), FileType::Unknown);
    }
}
