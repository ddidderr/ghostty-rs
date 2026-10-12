//! Derives the build version metadata for [`ghostty_build_info`](crate):
//!
//! * the app version, release channel and commit (Zig: `build/Config.zig`
//!   version detection + `build/GitVersion.zig`)
//! * the build mode (Zig: `builtin.mode`)
//!
//! The version can be overridden with `GHOSTTY_VERSION_STRING` (Zig:
//! `-Dversion-string`), which must be a semantic version.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn run(cmd: &str, args: &[&str], dir: &Path) -> Option<String> {
    let out = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// The `app-version` from `[workspace.metadata.ghostty]` of the workspace
/// manifest (two levels above this crate).
fn app_version(manifest_dir: &Path) -> (u64, u64, u64) {
    let root = manifest_dir.join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", root.display());
    let text = std::fs::read_to_string(root).unwrap_or_default();
    let v = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("app-version"))
        .and_then(|l| l.split('"').nth(1))
        .unwrap_or("1.3.1");
    let mut it = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(1),
        it.next().unwrap_or(3),
        it.next().unwrap_or(1),
    )
}

/// Returns `(version_string, commit)` mirroring `Config.zig`.
fn detect_version(manifest_dir: &Path) -> (String, Option<String>) {
    println!("cargo:rerun-if-env-changed=GHOSTTY_VERSION_STRING");
    if let Ok(v) = std::env::var("GHOSTTY_VERSION_STRING") {
        let commit = v.split_once('+').map(|(_, b)| b.to_string());
        return (v, commit);
    }

    let (maj, min, pat) = app_version(manifest_dir);
    let base = format!("{maj}.{min}.{pat}");

    // Git unavailable or not a repository: unknown dev version.
    let Some(branch) = run("git", &["rev-parse", "--abbrev-ref", "HEAD"], manifest_dir) else {
        return (format!("{base}-dev+0000000"), Some("0000000".into()));
    };
    if let Some(git_dir) = run("git", &["rev-parse", "--absolute-git-dir"], manifest_dir) {
        let git_dir = PathBuf::from(git_dir);
        println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
        println!("cargo:rerun-if-changed={}", git_dir.join("refs").display());
        // Missing watched paths make Cargo rerun this script on every build.
        // Packing loose refs also changes `refs`, so it will register this
        // optional file once Git creates it.
        let packed_refs = git_dir.join("packed-refs");
        if packed_refs.is_file() {
            println!("cargo:rerun-if-changed={}", packed_refs.display());
        }
    }
    let hash = run(
        "git",
        &[
            "-c",
            "log.showSignature=false",
            "log",
            "--pretty=format:%h",
            "-n",
            "1",
        ],
        manifest_dir,
    )
    .unwrap_or_default();
    let tag = run(
        "git",
        &["describe", "--exact-match", "--tags"],
        manifest_dir,
    )
    .filter(|t| !t.is_empty());

    if let Some(tag) = tag {
        // Tip releases behave just like any other pre-release so we skip.
        if tag != "tip" {
            assert!(
                tag == format!("v{base}"),
                "tagged releases must be in vX.Y.Z format matching the workspace app-version"
            );
            return (base, None);
        }
    }

    // Slashes in branch names are replaced like in `GitVersion.zig`.
    let branch = branch.replace('/', "-");
    (format!("{base}-{branch}+{hash}"), Some(hash))
}

/// The Zig optimize-mode name for the Cargo profile being built.
fn build_mode() -> &'static str {
    // The profile name is the directory below `target/` that holds OUT_DIR.
    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let profile = Path::new(&out)
        .ancestors()
        .skip_while(|p| p.file_name().is_none_or(|n| n != "build"))
        .nth(1)
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let debug_assertions = std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some();
    let opt = std::env::var("OPT_LEVEL").unwrap_or_default();
    if profile == "production" || !debug_assertions {
        ".ReleaseFast"
    } else if opt == "0" {
        ".Debug"
    } else {
        ".ReleaseSafe"
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let (version_string, commit) = detect_version(&manifest_dir);
    // Release channel: stable unless the version has a pre-release part.
    let channel = if version_string.contains('-') {
        "tip"
    } else {
        "stable"
    };
    println!("cargo:rustc-env=GHOSTTY_VERSION_STRING={version_string}");
    println!(
        "cargo:rustc-env=GHOSTTY_COMMIT={}",
        commit.unwrap_or_default()
    );
    println!("cargo:rustc-env=GHOSTTY_RELEASE_CHANNEL={channel}");
    println!("cargo:rustc-env=GHOSTTY_BUILD_MODE={}", build_mode());
}
