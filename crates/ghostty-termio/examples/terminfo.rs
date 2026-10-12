//! Prints the xterm-ghostty terminfo source (Zig: `ghostty +terminfo` in
//! `main_build_data.zig`). Used by `scripts/gen-terminfo.sh`.

fn main() {
    print!("{}", ghostty_termio::terminfo::ghostty.encode_to_string());
}
