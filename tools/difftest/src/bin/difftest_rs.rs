//! Rust twin of the Zig driver: `difftest_rs <cols> <rows> <max_scrollback> <file>`.
use std::io::Write as _;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    assert!(
        a.len() == 5,
        "usage: difftest_rs cols rows max_scrollback file"
    );
    let input = std::fs::read(&a[4]).expect("read input");
    let out = ghostty_difftest::dump(
        &input,
        a[1].parse().expect("cols"),
        a[2].parse().expect("rows"),
        a[3].parse().expect("scrollback"),
    );
    std::io::stdout().write_all(out.as_bytes()).expect("write");
}
