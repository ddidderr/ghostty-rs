//! Builds the wasm module and runs `tests/node_test.js` against it with
//! node, using stub JS imports (a fake canvas measuring API). Skipped when
//! node or the `wasm32-unknown-unknown` target is not available.

use std::{path::PathBuf, process::Command};

#[test]
fn wasm_module_runs_under_node() {
    if Command::new("node").arg("--version").output().is_err() {
        eprintln!("skipping: node not found");
        return;
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| manifest.join("../../target"), PathBuf::from);
    // A separate directory so this does not contend with the outer build.
    let target_dir = target_dir.join("wasm-test");

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = Command::new(cargo)
        .args([
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "ghostty-wasm",
            "--manifest-path",
        ])
        .arg(manifest.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("run cargo");
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("can't find crate for `std`")
            || stderr.contains("target may not be installed")
        {
            eprintln!("skipping: wasm32-unknown-unknown target not installed");
            return;
        }
        panic!("wasm build failed:\n{stderr}");
    }

    let wasm = target_dir.join("wasm32-unknown-unknown/debug/ghostty_wasm.wasm");
    assert!(wasm.exists(), "missing {}", wasm.display());

    let out = Command::new("node")
        .arg(manifest.join("tests/node_test.js"))
        .arg(&wasm)
        .output()
        .expect("run node");
    assert!(
        out.status.success(),
        "node test failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}
