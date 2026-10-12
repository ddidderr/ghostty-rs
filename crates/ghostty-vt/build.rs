//! Generates the sorted, lowercase X11 color table (`x11_table.rs`) from
//! `src/res/rgb.txt`, so name lookups need neither a lazily built map nor a
//! lowercased copy of the query (Zig builds the same table at comptime, so
//! looking up a color never allocates).

use std::{collections::HashSet, env, fmt::Write as _, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/res/rgb.txt");
    println!("cargo:rerun-if-changed=build.rs");

    let data = fs::read_to_string("src/res/rgb.txt").expect("read rgb.txt");
    let mut seen = HashSet::new();
    let mut entries: Vec<(String, u8, u8, u8)> = Vec::new();
    for line in data.split('\n').filter(|l| !l.is_empty()) {
        let comp = |r: std::ops::Range<usize>| -> u8 {
            line[r].trim_matches(' ').parse().expect("bad rgb.txt")
        };
        let (r, g, b) = (comp(0..3), comp(4..7), comp(8..11));
        let name = line[12..]
            .trim_matches(|c| c == ' ' || c == '\t' || c == '\n')
            .to_ascii_lowercase();
        // The first spelling of a (case-insensitive) name wins.
        if seen.insert(name.clone()) {
            entries.push((name, r, g, b));
        }
    }
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut out = String::from("&[\n");
    for (name, r, g, b) in &entries {
        writeln!(out, "    ({name:?}, {r}, {g}, {b}),").expect("write");
    }
    out.push_str("]\n");
    let dest = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("x11_table.rs");
    fs::write(dest, out).expect("write x11_table.rs");
}
