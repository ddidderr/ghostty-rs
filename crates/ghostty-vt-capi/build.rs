/// libghostty-vt version (Zig: `GhosttyLibVt` `version = .{ 0, 1, 0 }`).
/// `scripts/install-lib.sh` reads this line to name the installed files, so
/// keep the format.
const VT_VERSION: &str = "0.1.0";

/// The soname / install name of the shared library: `libghostty-vt.so.<major>`
/// on ELF platforms (Zig sets `.version` on the shared library, which makes
/// the linker emit the versioned soname), `@rpath/libghostty-vt.<major>.dylib`
/// on Apple platforms.
fn lib_name(target_os: &str, version: &str) -> Option<String> {
    let major = version.split('.').next().unwrap_or(version);
    match target_os {
        "linux" | "android" | "freebsd" => Some(format!("libghostty-vt.so.{major}")),
        "macos" | "ios" => Some(format!("@rpath/libghostty-vt.{major}.dylib")),
        _ => None,
    }
}

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match (lib_name(&target_os, VT_VERSION), target_os.as_str()) {
        (Some(name), "linux" | "android" | "freebsd") => {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-soname,{name}");
        }
        (Some(name), _) => {
            println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,{name}");
        }
        _ => {}
    }
    println!("cargo:rerun-if-changed=build.rs");
}

#[cfg(test)]
mod tests {
    use super::lib_name;

    #[test]
    fn versioned_names() {
        assert_eq!(
            lib_name("linux", "0.1.0").as_deref(),
            Some("libghostty-vt.so.0")
        );
        assert_eq!(
            lib_name("macos", "1.2.3").as_deref(),
            Some("@rpath/libghostty-vt.1.dylib")
        );
        assert_eq!(lib_name("windows", "0.1.0"), None);
    }
}
