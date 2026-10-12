fn main() {
    // Ship the shared library under Ghostty's name (`libghostty-vt.so`).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "linux" | "android" | "freebsd" => {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,libghostty-vt.so");
        }
        "macos" | "ios" => {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libghostty-vt.dylib");
        }
        _ => {}
    }
    println!("cargo:rerun-if-changed=build.rs");
}
