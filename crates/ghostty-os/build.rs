//! Emits `cfg(ghostty_debug_build)` for the Cargo `dev` profile (Zig
//! `builtin.mode == .Debug`). `debug_assertions` is not equivalent: the
//! repository's `release` profile enables it, like Zig's `ReleaseSafe`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROFILE");
    // Recompile resource lookup when the packager changes the embedded path.
    println!("cargo:rerun-if-env-changed=GHOSTTY_BUILD_RESOURCES_DIR");
    println!("cargo:rustc-check-cfg=cfg(ghostty_debug_build)");
    if std::env::var("PROFILE").is_ok_and(|p| p == "debug") {
        println!("cargo:rustc-cfg=ghostty_debug_build");
    }
}
