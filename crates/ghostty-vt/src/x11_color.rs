//! X11 color names (ZIG/src/terminal/x11_color.zig).

use crate::color::Rgb;
use std::collections::HashMap;
use std::sync::OnceLock;

/// This is the rgb.txt file from the X11 project. This was last sourced
/// from this location: https://gitlab.freedesktop.org/xorg/app/rgb
/// This data is licensed under the MIT/X11 license while this file is
/// licensed under the same license as Ghostty.
const DATA: &str = include_str!("res/rgb.txt");

/// The map of all available X11 colors. Keys are lowercase ASCII; lookups
/// are ASCII case-insensitive (see [`get`]).
fn map() -> &'static HashMap<String, Rgb> {
    static MAP: OnceLock<HashMap<String, Rgb>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        // Parse the line. This is not very robust parsing, because we expect
        // a very exact format for rgb.txt.
        for line in DATA.split('\n') {
            if line.is_empty() {
                continue;
            }
            let r: u8 = line[0..3].trim_matches(' ').parse().expect("bad rgb.txt");
            let g: u8 = line[4..7].trim_matches(' ').parse().expect("bad rgb.txt");
            let b: u8 = line[8..11].trim_matches(' ').parse().expect("bad rgb.txt");
            let name = line[12..].trim_matches(|c| c == ' ' || c == '\t' || c == '\n');
            m.entry(name.to_ascii_lowercase())
                .or_insert(Rgb { r, g, b });
        }
        m
    })
}

/// Look up an X11 color by name (ASCII case-insensitive).
pub fn get(name: &str) -> Option<Rgb> {
    map().get(&name.to_ascii_lowercase()).copied()
}

/// Byte-slice variant of [`get`].
pub fn get_bytes(name: &[u8]) -> Option<Rgb> {
    get(std::str::from_utf8(name).ok()?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn x11_color_map() {
        assert_eq!(None, get("nosuchcolor"));
        assert_eq!(
            Rgb {
                r: 255,
                g: 255,
                b: 255
            },
            get("white").unwrap()
        );
        assert_eq!(
            Rgb {
                r: 0,
                g: 250,
                b: 154
            },
            get("medium spring green").unwrap()
        );
        assert_eq!(
            Rgb {
                r: 34,
                g: 139,
                b: 34
            },
            get("ForestGreen").unwrap()
        );
        assert_eq!(
            Rgb {
                r: 34,
                g: 139,
                b: 34
            },
            get("FoReStGReen").unwrap()
        );
        assert_eq!(Rgb { r: 0, g: 0, b: 0 }, get("black").unwrap());
        assert_eq!(Rgb { r: 255, g: 0, b: 0 }, get("red").unwrap());
        assert_eq!(Rgb { r: 0, g: 255, b: 0 }, get("green").unwrap());
        assert_eq!(Rgb { r: 0, g: 0, b: 255 }, get("blue").unwrap());
        assert_eq!(
            Rgb {
                r: 124,
                g: 252,
                b: 0
            },
            get("lawngreen").unwrap()
        );
        assert_eq!(
            Rgb {
                r: 0,
                g: 250,
                b: 154
            },
            get("mediumspringgreen").unwrap()
        );
        assert_eq!(
            Rgb {
                r: 34,
                g: 139,
                b: 34
            },
            get("forestgreen").unwrap()
        );
    }
}
