//! Emits `cfg(ghostty_debug_build)` for the Cargo `dev` profile (Zig
//! `builtin.mode == .Debug`). `debug_assertions` is not equivalent: the
//! repository's `release` profile enables it, like Zig's `ReleaseSafe`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rustc-check-cfg=cfg(ghostty_debug_build)");
    let profile = std::env::var("PROFILE").unwrap_or_default();
    println!("cargo:rustc-env=GHOSTTY_BUILD_PROFILE={profile}");
    if profile == "debug" {
        println!("cargo:rustc-cfg=ghostty_debug_build");
    }
}
