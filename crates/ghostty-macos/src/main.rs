//! `ghostty-macos-app`: the executable inside `Ghostty.app` (Swift `main.swift`).
//! An empty stub on non-Apple targets.

#[cfg(target_vendor = "apple")]
fn main() {
    ghostty_macos::app::entry::run();
}

#[cfg(not(target_vendor = "apple"))]
fn main() {
    eprintln!("ghostty-macos-app only runs on macOS");
    std::process::exit(1);
}
