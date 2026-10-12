//! `apprt.Target.cval` and `apprt.Action.cval`: the conversion of core
//! actions to the `ghostty_target_s` / `ghostty_action_s` C structs
//! (ZIG/src/apprt/action.zig).
#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_int};

use ghostty_apprt::{
    action::{Action, KeySequence, KeyTable, Target},
    runtime_api::SurfaceId,
};
use ghostty_renderer::Health;
use ghostty_vt::osc::ProgressState;

use crate::{
    config_capi::{CConfig, default_trigger, trigger_to_c},
    types::{
        ghostty_action_command_finished_s,
        ghostty_action_s,
        ghostty_action_u,
        ghostty_config_t,
        ghostty_target_s,
        ghostty_target_u,
        target_tag,
    },
};

/// Memory that the pointers inside a converted action point to. It must
/// outlive the callback invocation.
#[derive(Default)]
pub struct Keep {
    strings: Vec<CString>,
    counted: Vec<Box<[u8]>>,
    config: Option<ghostty_config_t>,
}

impl Keep {
    /// A NUL terminated copy of `s` (it ends at the first interior NUL).
    fn cstr(&mut self, s: &str) -> *const c_char {
        self.cstr_bytes(s.as_bytes())
    }

    /// [`Keep::cstr`] for raw bytes.
    fn cstr_bytes(&mut self, s: &[u8]) -> *const c_char {
        let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
        let c = CString::new(&s[..end]).unwrap_or_default();
        let ptr = c.as_ptr();
        // The heap buffer of a CString doesn't move when the Vec grows.
        self.strings.push(c);
        ptr
    }

    /// The complete `s` (interior NULs included) followed by a NUL, for
    /// pointer + length pairs: the advertised length is always `s.len()` and
    /// the allocation holds at least that many bytes (Zig passes the whole
    /// backing slice).
    fn counted_bytes(&mut self, s: &[u8]) -> *const c_char {
        let mut v = Vec::with_capacity(s.len() + 1);
        v.extend_from_slice(s);
        v.push(0);
        let b = v.into_boxed_slice();
        let ptr = b.as_ptr().cast::<c_char>();
        self.counted.push(b);
        ptr
    }
}

impl Drop for Keep {
    fn drop(&mut self) {
        if let Some(h) = self.config.take() {
            // SAFETY: created by `CConfig::into_handle` in `action_to_c`
            // and not handed out beyond the callback.
            unsafe { crate::config_capi::ghostty_config_free(h) };
        }
    }
}

/// `Target.cval`. The surface handle is the id of the surface (the address
/// of the embedded runtime's surface object, see `embedded.rs`).
#[must_use]
pub fn target_to_c(target: &Target<SurfaceId>) -> ghostty_target_s {
    match target {
        Target::App => ghostty_target_s {
            tag: target_tag::APP,
            target: ghostty_target_u {
                surface: std::ptr::null_mut(),
            },
        },
        Target::Surface(id) => ghostty_target_s {
            tag: target_tag::SURFACE,
            target: ghostty_target_u {
                surface: *id as usize as *mut std::ffi::c_void,
            },
        },
    }
}

fn zeroed_union() -> ghostty_action_u {
    // SAFETY: every field of the union is plain old data for which the
    // all-zero bit pattern is valid (integers, floats, null pointers, bool).
    unsafe { std::mem::zeroed() }
}

/// `Action.cval`.
#[must_use]
pub fn action_to_c(action: &Action, keep: &mut Keep) -> ghostty_action_s {
    let mut v = zeroed_union();
    match action {
        Action::CloseTab(m) => v.close_tab_mode = *m as c_int,
        Action::NewSplit(d) => v.new_split = *d as c_int,
        Action::ToggleFullscreen(f) => v.toggle_fullscreen = *f as c_int,
        Action::MoveTab(m) => v.move_tab.amount = m.amount,
        Action::GotoTab(t) => v.goto_tab = t.int(),
        Action::GotoSplit(s) => v.goto_split = *s as c_int,
        Action::GotoWindow(w) => v.goto_window = *w as c_int,
        Action::ResizeSplit(r) => {
            v.resize_split.amount = r.amount;
            v.resize_split.direction = r.direction as c_int;
        }
        Action::SizeLimit(s) => {
            v.size_limit.min_width = s.min_width;
            v.size_limit.min_height = s.min_height;
            v.size_limit.max_width = s.max_width;
            v.size_limit.max_height = s.max_height;
        }
        Action::InitialSize(s) => {
            v.initial_size.width = s.width;
            v.initial_size.height = s.height;
        }
        Action::CellSize(s) => {
            v.cell_size.width = s.width;
            v.cell_size.height = s.height;
        }
        Action::Scrollbar(s) => {
            v.scrollbar.total = s.total as u64;
            v.scrollbar.offset = s.offset as u64;
            v.scrollbar.len = s.len as u64;
        }
        Action::Inspector(i) => v.inspector = *i as c_int,
        Action::DesktopNotification(n) => {
            v.desktop_notification.title = keep.cstr_bytes(&n.title.to_bytes());
            v.desktop_notification.body = keep.cstr_bytes(&n.body.to_bytes());
        }
        Action::SetTitle(t) => v.set_title.title = keep.cstr_bytes(&t.title.to_bytes()),
        Action::SetTabTitle(t) => v.set_tab_title.title = keep.cstr_bytes(&t.title.to_bytes()),
        Action::PromptTitle(p) => v.prompt_title = *p as c_int,
        Action::Pwd(p) => {
            v.pwd.pwd = keep.cstr_bytes(&p.pwd.to_bytes());
        }
        Action::MouseShape(s) => v.mouse_shape = *s as c_int,
        Action::MouseVisibility(m) => v.mouse_visibility = *m as c_int,
        Action::MouseOverLink(l) => {
            let url = l.url.to_bytes();
            v.mouse_over_link.url = keep.counted_bytes(&url);
            v.mouse_over_link.len = url.len();
        }
        Action::RendererHealth(h) => {
            v.renderer_health = match h {
                Health::Healthy => 0,
                Health::Unhealthy => 1,
            };
        }
        Action::QuitTimer(q) => v.quit_timer = *q as c_int,
        Action::FloatWindow(f) => v.float_window = *f as c_int,
        Action::SecureInput(s) => v.secure_input = *s as c_int,
        Action::KeySequence(KeySequence::Trigger(t)) => {
            v.key_sequence.active = true;
            v.key_sequence.trigger = trigger_to_c(&t.cval());
        }
        Action::KeySequence(KeySequence::End) => {
            v.key_sequence.active = false;
            v.key_sequence.trigger = default_trigger();
        }
        Action::KeyTable(KeyTable::Activate(name)) => {
            v.key_table.tag = 0;
            let bytes = name.to_bytes();
            v.key_table.value.activate.name = keep.counted_bytes(&bytes);
            v.key_table.value.activate.len = bytes.len();
        }
        Action::KeyTable(KeyTable::Deactivate) => v.key_table.tag = 1,
        Action::KeyTable(KeyTable::DeactivateAll) => v.key_table.tag = 2,
        Action::ColorChange(c) => {
            v.color_change.kind = c.kind.int();
            v.color_change.r = c.r;
            v.color_change.g = c.g;
            v.color_change.b = c.b;
        }
        Action::ReloadConfig(r) => v.reload_config.soft = r.soft,
        Action::ConfigChange(c) => {
            // The embedder reads (and clones) the config during the
            // callback; it is freed again when `keep` drops.
            if let Some(old) = keep.config.take() {
                // SAFETY: created below by an earlier conversion.
                unsafe { crate::config_capi::ghostty_config_free(old) };
            }
            let handle = CConfig::new((*c.config).clone()).into_handle();
            keep.config = Some(handle);
            v.config_change.config = handle;
        }
        Action::OpenUrl(u) => {
            v.open_url.kind = u.kind as c_int;
            // The URL may be the escaped form of a non-UTF-8 path; hand the
            // embedder the real bytes.
            let url = u.url.to_bytes();
            v.open_url.url = keep.counted_bytes(&url);
            v.open_url.len = url.len();
        }
        Action::ShowChildExited(c) => {
            v.child_exited.exit_code = c.exit_code;
            v.child_exited.timetime_ms = c.runtime_ms;
        }
        Action::ProgressReport(p) => {
            v.progress_report.state = match p.state {
                ProgressState::Remove => 0,
                ProgressState::Set => 1,
                ProgressState::Error => 2,
                ProgressState::Indeterminate => 3,
                ProgressState::Pause => 4,
            };
            v.progress_report.progress = p.progress.map_or(-1, |p| p as i8);
        }
        Action::CommandFinished(c) => {
            v.command_finished = ghostty_action_command_finished_s {
                exit_code: c.exit_code.map_or(-1, i16::from),
                duration: c.duration.duration,
            };
        }
        Action::StartSearch(s) => v.start_search.needle = keep.cstr(&s.needle),
        Action::SearchTotal(s) => {
            v.search_total.total = s.total.map_or(-1, |t| t as isize);
        }
        Action::SearchSelected(s) => {
            v.search_selected.selected = s.selected.map_or(-1, |t| t as isize);
        }
        Action::Readonly(r) => v.readonly = *r as c_int,
        _ => {}
    }
    ghostty_action_s {
        tag: action.key() as c_int,
        action: v,
    }
}

#[cfg(test)]
mod tests {
    use ghostty_apprt::action::{
        CellSize,
        CommandFinished,
        GotoTab,
        MouseOverLink,
        ResizeSplit,
        ResizeSplitDirection,
        SearchTotal,
        SetTitle,
    };

    use super::*;

    #[test]
    fn c_title_actions_end_at_the_first_interior_nul() {
        use ghostty_os::osbytes::EncodedOs;
        let mut keep = Keep::default();
        for tab in [false, true] {
            let title = SetTitle {
                title: EncodedOs::from_bytes(b"a\0b"),
            };
            let action = if tab {
                Action::SetTabTitle(title)
            } else {
                Action::SetTitle(title)
            };
            let c = action_to_c(&action, &mut keep);
            // SAFETY: both title union members have identical layout, pointer owned by keep.
            let title = unsafe { std::ffi::CStr::from_ptr(c.action.set_title.title) };
            assert_eq!(title.to_bytes(), b"a");
        }
    }

    #[test]
    fn c_title_actions_and_key_table_names_forward_original_bytes() {
        use ghostty_os::osbytes::EncodedOs;
        let raw = b"name\xff\xef\xb7\x90\xc3\xa9";
        let mut keep = Keep::default();
        for tab in [false, true] {
            let title = SetTitle {
                title: EncodedOs::from_bytes(raw),
            };
            let action = if tab {
                Action::SetTabTitle(title)
            } else {
                Action::SetTitle(title)
            };
            let c = action_to_c(&action, &mut keep);
            // SAFETY: both title union members have identical layout, pointer owned by keep.
            let title = unsafe { std::ffi::CStr::from_ptr(c.action.set_title.title) };
            assert_eq!(title.to_bytes(), raw);
        }
        let c = action_to_c(
            &Action::KeyTable(KeyTable::Activate(EncodedOs::from_bytes(raw))),
            &mut keep,
        );
        // SAFETY: the tag is key_table.activate and keep owns the buffer.
        let name = unsafe {
            std::slice::from_raw_parts(
                c.action.key_table.value.activate.name.cast::<u8>(),
                c.action.key_table.value.activate.len,
            )
        };
        assert_eq!(name, raw);
    }

    #[test]
    fn counted_buffers_hold_exactly_len_bytes() {
        use ghostty_os::osbytes::EncodedOs;
        let mut keep = Keep::default();
        let raw = b"a\0b";
        let c = action_to_c(
            &Action::KeyTable(KeyTable::Activate(EncodedOs::from_bytes(raw))),
            &mut keep,
        );
        // SAFETY: tag is key_table.activate; keep owns len + 1 bytes.
        let (p, n) = unsafe {
            (
                c.action.key_table.value.activate.name.cast::<u8>(),
                c.action.key_table.value.activate.len,
            )
        };
        assert_eq!(n, 3);
        // SAFETY: the allocation holds `n` bytes plus a NUL.
        assert_eq!(unsafe { std::slice::from_raw_parts(p, n + 1) }, b"a\0b\0");

        let c = action_to_c(
            &Action::MouseOverLink(MouseOverLink {
                url: EncodedOs::from_bytes(b"u\0v"),
            }),
            &mut keep,
        );
        // SAFETY: tag is mouse_over_link.
        let (p, n) = unsafe {
            (
                c.action.mouse_over_link.url.cast::<u8>(),
                c.action.mouse_over_link.len,
            )
        };
        assert_eq!(n, 3);
        // SAFETY: as above.
        assert_eq!(unsafe { std::slice::from_raw_parts(p, n + 1) }, b"u\0v\0");

        let c = action_to_c(
            &Action::OpenUrl(ghostty_apprt::action::OpenUrl {
                kind: ghostty_apprt::action::OpenUrlKind::default(),
                url: EncodedOs::from_bytes(b"x\0y"),
            }),
            &mut keep,
        );
        // SAFETY: tag is open_url.
        let (p, n) = unsafe { (c.action.open_url.url.cast::<u8>(), c.action.open_url.len) };
        assert_eq!(n, 3);
        // SAFETY: as above.
        assert_eq!(unsafe { std::slice::from_raw_parts(p, n + 1) }, b"x\0y\0");
    }

    #[test]
    fn notification_forwards_raw_bytes() {
        use ghostty_os::osbytes::EncodedOs;
        let mut keep = Keep::default();
        let c = action_to_c(
            &Action::DesktopNotification(ghostty_apprt::action::DesktopNotification {
                title: EncodedOs::from_bytes(b"t\xff"),
                body: EncodedOs::from_bytes(b"b\xff"),
            }),
            &mut keep,
        );
        // SAFETY: tag is desktop_notification; keep owns the strings.
        let (t, b) = unsafe {
            (
                std::ffi::CStr::from_ptr(c.action.desktop_notification.title),
                std::ffi::CStr::from_ptr(c.action.desktop_notification.body),
            )
        };
        assert_eq!((t.to_bytes(), b.to_bytes()), (&b"t\xff"[..], &b"b\xff"[..]));
    }

    #[test]
    fn target_cval() {
        let t = target_to_c(&Target::App);
        assert_eq!(t.tag, 0);
        let t = target_to_c(&Target::Surface(0x1000));
        assert_eq!(t.tag, 1);
        // SAFETY: tag says surface.
        assert_eq!(unsafe { t.target.surface } as usize, 0x1000);
    }

    #[test]
    fn simple_payloads() {
        let mut keep = Keep::default();
        let a = action_to_c(
            &Action::CellSize(CellSize {
                width: 7,
                height: 9,
            }),
            &mut keep,
        );
        assert_eq!(a.tag, 25);
        // SAFETY: tag says cell_size.
        assert_eq!(
            unsafe { (a.action.cell_size.width, a.action.cell_size.height) },
            (7, 9)
        );

        let a = action_to_c(&Action::GotoTab(GotoTab::Last), &mut keep);
        // SAFETY: tag says goto_tab.
        assert_eq!(unsafe { a.action.goto_tab }, -3);

        let a = action_to_c(
            &Action::ResizeSplit(ResizeSplit {
                amount: 5,
                direction: ResizeSplitDirection::Left,
            }),
            &mut keep,
        );
        // SAFETY: tag says resize_split.
        assert_eq!(
            unsafe {
                (
                    a.action.resize_split.amount,
                    a.action.resize_split.direction,
                )
            },
            (5, 2)
        );

        let a = action_to_c(&Action::SearchTotal(SearchTotal { total: None }), &mut keep);
        // SAFETY: tag says search_total.
        assert_eq!(unsafe { a.action.search_total.total }, -1);

        let a = action_to_c(
            &Action::CommandFinished(CommandFinished {
                exit_code: None,
                duration: ghostty_config::types_b::Duration { duration: 5 },
            }),
            &mut keep,
        );
        // SAFETY: tag says command_finished.
        assert_eq!(
            unsafe {
                (
                    a.action.command_finished.exit_code,
                    a.action.command_finished.duration,
                )
            },
            (-1, 5)
        );
    }

    #[test]
    fn strings_stay_valid_while_kept() {
        let mut keep = Keep::default();
        let a = action_to_c(
            &Action::SetTitle(SetTitle {
                title: "hello".into(),
            }),
            &mut keep,
        );
        // SAFETY: tag says set_title; the pointer points into `keep`.
        let s = unsafe { std::ffi::CStr::from_ptr(a.action.set_title.title) };
        assert_eq!(s.to_str().unwrap(), "hello");

        let a = action_to_c(
            &Action::MouseOverLink(MouseOverLink {
                url: ghostty_os::osbytes::EncodedOs::from_bytes(b"https://x"),
            }),
            &mut keep,
        );
        // SAFETY: tag says mouse_over_link.
        assert_eq!(unsafe { a.action.mouse_over_link.len }, 9);
    }

    #[test]
    fn config_change_hands_out_a_config() {
        let mut keep = Keep::default();
        let cfg = std::sync::Arc::new(ghostty_config::config::Config::default());
        let a = action_to_c(
            &Action::ConfigChange(ghostty_apprt::action::ConfigChange { config: cfg }),
            &mut keep,
        );
        // SAFETY: tag says config_change.
        let h = unsafe { a.action.config_change.config };
        let mut out = false;
        let key = "maximize";
        // SAFETY: valid handle and destination.
        assert!(unsafe {
            crate::config_capi::ghostty_config_get(
                h,
                (&raw mut out).cast(),
                key.as_ptr().cast(),
                key.len(),
            )
        });
    }
}
