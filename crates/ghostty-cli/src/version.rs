//! `+version` (ZIG/src/cli/version.zig).

use std::io::{IsTerminal, Write};

use crate::{
    build_info::{self, BuildInfo},
    error::Error,
};

/// The `version` command is used to display information about Ghostty. Recognized as
/// either `+version` or `--version`.
pub fn run(_args: &[String]) -> Result<u8, Error> {
    let stdout = std::io::stdout();
    let tty = stdout.is_terminal();
    let mut out = std::io::BufWriter::new(stdout.lock());
    write_version(&mut out, &build_info::get(), tty)?;
    out.flush()?;
    Ok(0)
}

fn version_na(v: Option<crate::build_info::Version>) -> String {
    v.map_or_else(|| "unknown".to_string(), |v| v.to_string())
}

/// Write the version text.
pub fn write_version(out: &mut dyn Write, info: &BuildInfo, tty: bool) -> std::io::Result<()> {
    if tty {
        if let Some(commit) = &info.commit {
            write!(
                out,
                "\x1b]8;;https://github.com/ghostty-org/ghostty/commit/{commit}\x1b\\"
            )?;
        }
    }
    write!(out, "Ghostty {}\n\n", info.version_string)?;
    if tty {
        write!(out, "\x1b]8;;\x1b\\")?;
    }

    writeln!(out, "Version")?;
    writeln!(out, "  - version: {}", info.version_string)?;
    writeln!(out, "  - channel: {}", info.release_channel)?;

    writeln!(out, "Build Config")?;
    // The Zig build prints its compiler version here.
    writeln!(out, "  - Rust version  : {}", env!("GHOSTTY_RUSTC_VERSION"))?;
    writeln!(out, "  - build mode    : {}", info.build_mode)?;
    writeln!(out, "  - app runtime   : {}", info.app_runtime)?;
    writeln!(out, "  - font engine   : {}", info.font_backend)?;
    writeln!(out, "  - renderer      : {}", info.renderer)?;
    // There is no libxev in the Rust port.
    writeln!(out, "  - libxev        : n/a")?;
    if info.app_runtime == ".gtk" {
        let kernel_info = ghostty_os::get_kernel_info();
        writeln!(
            out,
            "  - kernel version: {}",
            kernel_info
                .as_deref()
                .unwrap_or("Kernel information unavailable")
        )?;
        let de = match ghostty_os::desktop_environment() {
            ghostty_os::DesktopEnvironment::Gnome => "gnome",
            ghostty_os::DesktopEnvironment::Macos => "macos",
            ghostty_os::DesktopEnvironment::Other => "other",
            ghostty_os::DesktopEnvironment::Windows => "windows",
        };
        writeln!(out, "  - desktop env   : {de}")?;
        writeln!(out, "  - GTK version   :")?;
        writeln!(out, "    build         : {}", version_na(info.gtk_build))?;
        writeln!(out, "    runtime       : {}", version_na(info.gtk_runtime))?;
        writeln!(out, "  - libadwaita    : enabled")?;
        writeln!(out, "    build         : {}", version_na(info.adw_build))?;
        writeln!(out, "    runtime       : {}", version_na(info.adw_runtime))?;
        writeln!(
            out,
            "  - libX11        : {}",
            if info.x11 { "enabled" } else { "disabled" }
        )?;
        writeln!(
            out,
            "  - libwayland    : {}",
            if info.wayland { "enabled" } else { "disabled" }
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn version_output_layout() {
        let mut buf = Vec::new();
        let info = BuildInfo {
            version_string: "1.3.1".into(),
            commit: None,
            release_channel: "stable".into(),
            ..BuildInfo::default()
        };
        write_version(&mut buf, &info, false).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with(
            "Ghostty 1.3.1\n\nVersion\n  - version: 1.3.1\n  - channel: stable\nBuild Config\n"
        ));
        assert!(s.contains("  - libwayland    : enabled\n"));
    }

    #[test]
    fn version_tty_link() {
        let info = BuildInfo {
            commit: Some("abc".into()),
            ..BuildInfo::default()
        };
        let mut buf = Vec::new();
        write_version(&mut buf, &info, true).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.starts_with("\x1b]8;;https://github.com/ghostty-org/ghostty/commit/abc\x1b\\Ghostty")
        );
    }
}
