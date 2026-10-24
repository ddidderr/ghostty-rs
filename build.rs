//! Root build script.
//!
//! * `cfg(gtk)` on Linux and FreeBSD, where the GTK runtime is built (Zig
//!   `apprt.Runtime.default`); elsewhere `ghostty` is the helper CLI.
//! * Linux: link the final binary with lazy symbol binding. Rust links
//!   executables with `-z now` (eager binding) by default. The GTK apprt
//!   compiles in code for newer GTK/libadwaita API (e.g. the GTK 4.16 CSS
//!   diagnostics) that is only ever called after a runtime `at_least` check,
//!   exactly like the Zig build, which links GTK dynamically with lazy
//!   binding. With eager binding the dynamic loader would refuse to start the
//!   program on an older library that merely lacks such a symbol; with lazy
//!   binding the symbol is only resolved if the guarded code actually runs.
//! * Windows (Zig: `GhosttyExe.zig`): use the GUI subsystem and embed
//!   `dist/windows/ghostty.rc` (icon, version resource and the manifest that
//!   declares per-monitor DPI awareness and the common-controls v6 activation
//!   context).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // `cfg(gtk)`: the GTK runtime is built (Zig `apprt.Runtime.default`).
    println!("cargo::rustc-check-cfg=cfg(gtk)");
    if matches!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref(),
        Ok("linux" | "freebsd")
    ) {
        println!("cargo:rustc-cfg=gtk");
    }
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => println!("cargo:rustc-link-arg-bins=-Wl,-z,lazy"),
        Ok("windows") => windows(),
        _ => {}
    }
}

fn windows() {
    println!("cargo:rerun-if-changed=dist/windows/ghostty.rc");
    println!("cargo:rerun-if-changed=dist/windows/ghostty.ico");
    println!("cargo:rerun-if-changed=dist/windows/ghostty.manifest");

    // `exe.subsystem = .Windows`.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/SUBSYSTEM:WINDOWS");
        println!("cargo:rustc-link-arg-bins=/ENTRY:mainCRTStartup");
    } else {
        println!("cargo:rustc-link-arg-bins=-Wl,--subsystem,windows");
    }

    // `exe.addWin32ResourceFile(.{ .file = "dist/windows/ghostty.rc" })`.
    // The resource is only linked into the binary targets, not the
    // (nonexistent) library target of this package.
    embed_resource::compile_for("dist/windows/ghostty.rc", ["ghostty"], embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the Windows resource file");
}
