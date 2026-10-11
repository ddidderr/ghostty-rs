//! Compiles the blueprint UI files with `blueprint-compiler` and bundles them
//! (plus CSS and icons) into a GResource, mirroring
//! ZIG/src/apprt/gtk/build/{blueprint,gresource}.zig.

#![allow(clippy::unwrap_used)]
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREFIX: &str = "/com/mitchellh/ghostty";
const APP_ID: &str = "com.mitchellh.ghostty";
const ICON_SIZES: &[u32] = &[16, 32, 128, 256, 512, 1024];
const CSS: &[&str] = &[
    "style.css",
    "style-dark.css",
    "style-hc.css",
    "style-hc-dark.css",
];
// (major, minor, name). The inspector / imgui blueprints are deferred.
const BLUEPRINTS: &[(u32, u32, &str)] = &[
    (1, 0, "clipboard-confirmation-dialog"),
    (1, 4, "clipboard-confirmation-dialog"),
    (1, 2, "close-confirmation-dialog"),
    (1, 2, "config-errors-dialog"),
    (1, 2, "debug-warning"),
    (1, 3, "debug-warning"),
    (1, 2, "resize-overlay"),
    (1, 2, "search-overlay"),
    (1, 2, "key-state-overlay"),
    (1, 5, "split-tree"),
    (1, 5, "split-tree-split"),
    (1, 2, "surface"),
    (1, 5, "surface-scrolled-window"),
    (1, 3, "surface-child-exited"),
    (1, 5, "tab"),
    (1, 5, "title-dialog"),
    (1, 5, "window"),
    (1, 5, "command-palette"),
];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let res = manifest.join("resources");
    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=build.rs");

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n");

    let _ = writeln!(xml, "  <gresource prefix=\"{PREFIX}\">");
    for name in CSS {
        let _ = writeln!(
            xml,
            "    <file compressed=\"true\" alias=\"{name}\">css/{name}</file>"
        );
    }
    xml.push_str("  </gresource>\n");

    let _ = writeln!(xml, "  <gresource prefix=\"{PREFIX}/icons\">");
    for &s in ICON_SIZES {
        let _ = writeln!(
            xml,
            "    <file alias=\"{s}x{s}/apps/{APP_ID}.png\">icons/{s}.png</file>"
        );
        let _ = writeln!(
            xml,
            "    <file alias=\"{s}x{s}@2/apps/{APP_ID}.png\">icons/{}.png</file>",
            s * 2
        );
    }
    xml.push_str("  </gresource>\n");

    let _ = writeln!(xml, "  <gresource prefix=\"{PREFIX}/ui\">");
    for &(major, minor, name) in BLUEPRINTS {
        let rel = format!("{major}.{minor}/{name}");
        let input = res.join(format!("ui/{rel}.blp"));
        let output = out.join(format!("ui/{rel}.ui"));
        compile_blueprint(&input, &output);
        let _ = writeln!(xml, "    <file alias=\"{rel}.ui\">ui/{rel}.ui</file>");
    }
    xml.push_str("  </gresource>\n</gresources>\n");

    let xml_path = out.join("ghostty.gresource.xml");
    std::fs::write(&xml_path, xml).unwrap();

    glib_build_tools::compile_resources(
        &[out.as_path(), res.as_path()],
        xml_path.to_str().unwrap(),
        "ghostty.gresource",
    );
}

fn compile_blueprint(input: &Path, output: &Path) {
    std::fs::create_dir_all(output.parent().unwrap()).unwrap();
    let st = Command::new("blueprint-compiler")
        .arg("compile")
        .arg("--output")
        .arg(output)
        .arg(input)
        .output()
        .unwrap_or_else(|e| {
            panic!("`blueprint-compiler` (>= 0.16) is required to build ghostty-apprt-gtk: {e}")
        });
    assert!(
        st.status.success(),
        "blueprint-compiler failed for {}:\n{}",
        input.display(),
        String::from_utf8_lossy(&st.stderr)
    );
}
