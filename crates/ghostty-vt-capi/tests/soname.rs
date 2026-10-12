//! The shared library must carry the versioned soname `libghostty-vt.so.<major>`
//! straight from the build (Zig sets `.version` on the shared library), not
//! via a post-install `patchelf` step.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

mod common;

use std::process::Command;

#[test]
fn shared_library_has_versioned_soname() {
    let lib = common::install_native_lib();
    let so = lib.join("libghostty-vt.so");
    let Ok(out) = Command::new("readelf").arg("-d").arg(&so).output() else {
        eprintln!("readelf not available; skipping");
        return;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let want = format!("[libghostty-vt.so.{}]", common::VT_MAJOR);
    assert!(
        text.lines()
            .any(|l| l.contains("SONAME") && l.contains(&want)),
        "expected SONAME {want}:\n{text}"
    );
    // build.rs is the single source of the version (install-lib.sh reads it).
    let build_rs = std::fs::read_to_string(common::manifest_dir().join("build.rs")).unwrap();
    assert!(build_rs.contains(&format!("const VT_VERSION: &str = \"{}.", common::VT_MAJOR)));
}
