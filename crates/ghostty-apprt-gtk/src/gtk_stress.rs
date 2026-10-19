//! Temporary in-process GTK teardown stress test, enabled with
//! `GHOSTTY_GTK_STRESS=1`. Normal config, shell, renderer and GTK startup run
//! first. A GLib timer then drives the existing pane/tab handlers, yielding
//! between operations so idle rebuilds, rendering and child processes run.

use std::{
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use adw::prelude::*;
use ghostty_datastruct::split_tree::Goto;
use gtk::glib;

use crate::class::{application::Application, split_tree::SplitTree, tab::Tab, window::Window};

const MAX_TABS: usize = 6;
const MAX_PANES: usize = 8;

/// Report sustained waits at most once a second, not on every 1 ms poll.
#[derive(Default)]
struct WaitState {
    since: Option<Instant>,
    reported: Option<Instant>,
}

impl WaitState {
    fn report(&mut self, now: Instant, step: u64, reason: &str) -> bool {
        let since = *self.since.get_or_insert(now);
        if now.duration_since(self.reported.unwrap_or(since)) < Duration::from_secs(1) {
            return false;
        }
        self.reported = Some(now);
        eprintln!(
            "GTK-STRESS wait step={step} reason={reason} elapsed-ms={}",
            now.duration_since(since).as_millis(),
        );
        true
    }

    fn resumed(&mut self, now: Instant, step: u64) {
        if let Some(since) = self.since.take()
            && self.reported.take().is_some()
        {
            eprintln!(
                "GTK-STRESS resumed step={step} waited-ms={}",
                now.duration_since(since).as_millis(),
            );
        }
    }
}

/// A new split may remain unmapped (and never initialize its core) while
/// other panes are usable. After a sustained wait, do what a user clicking
/// a visible pane would do. Never initialize a core or force GL realization.
fn recover_target(window: &Window, split: &SplitTree, step: u64) {
    if let Some(tree) = split.get_tree()
        && let Some(entry) = tree.iterator().find(|entry| {
            entry.view.is_mapped() && entry.view.is_realized() && entry.view.core().is_some()
        })
    {
        eprintln!(
            "GTK-STRESS recover step={step} action=focus-ready-pane surface={:?}",
            entry.view.id(),
        );
        entry.view.grab_focus();
        return;
    }

    // A tab without a usable pane need not stop the entire stress session.
    // Selecting another tab lets GTK map it normally; retry on the next tick.
    let view = window.tab_view();
    if view.n_pages() > 1
        && let Some(page) = view.selected_page()
    {
        let next = (view.page_position(&page) + 1) % view.n_pages();
        eprintln!("GTK-STRESS recover step={step} action=switch-tab target-tab={next}");
        view.set_selected_page(&view.nth_page(next));
    } else {
        eprintln!("GTK-STRESS recover step={step} action=request-layout");
        window.queue_resize();
    }
}

/// Also isolates the application from existing single-instance sessions and
/// accepts pane/tab close confirmations in this disposable test process.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("GHOSTTY_GTK_STRESS").as_deref() == Ok("1"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Split,
    ClosePane,
    Zoom,
    NewTab,
    CloseTab,
    SwitchTab,
    FocusPane,
}

/// Weighted choices based on the current tree, never on assumed success of
/// a previous operation. Each tab retains at least one pane; one tab survives.
fn choices(tabs: usize, panes: usize) -> Vec<Action> {
    let mut actions = Vec::new();
    if panes < MAX_PANES {
        actions.extend([Action::Split; 3]);
    }
    if panes > 1 {
        actions.extend([Action::ClosePane; 2]);
        actions.extend([Action::Zoom; 4]);
        actions.push(Action::FocusPane);
    }
    if tabs < MAX_TABS {
        actions.push(Action::NewTab);
    }
    if tabs > 1 {
        actions.push(Action::CloseTab);
        actions.extend([Action::SwitchTab; 2]);
    }
    actions
}

/// SplitMix64 needs no dependency and accepts every seed, including zero.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next() % len as u64) as usize
    }
}

fn env_number(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            eprintln!("GTK-STRESS invalid {name}={value:?}; using {default}");
            default
        }),
        Err(_) => default,
    }
}

/// The caller removes this source when the application's run loop exits.
pub(crate) fn start(app: &Application) -> Option<glib::SourceId> {
    if !enabled() {
        return None;
    }
    let default_seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ u64::from(std::process::id());
    let seed = env_number("GHOSTTY_GTK_STRESS_SEED", default_seed);
    let interval = env_number("GHOSTTY_GTK_STRESS_INTERVAL_MS", 5).clamp(1, 60_000);
    eprintln!(
        "GTK-STRESS start pid={} seed={seed} interval-ms={interval}..{} max-tabs={MAX_TABS} max-panes-per-tab={MAX_PANES}",
        std::process::id(),
        interval * 3,
    );

    let app = app.downgrade();
    let window = glib::WeakRef::<Window>::new();
    let mut attached = false;
    let mut random = Random(seed);
    let mut step = 0u64;
    let mut wait = WaitState::default();
    let mut next_action = Instant::now() + Duration::from_secs(2);
    Some(glib::timeout_add_local(
        Duration::from_millis(1),
        move || {
            let now = Instant::now();
            // Stay registered until run_loop removes us, including if the user
            // manually closes the target window. Do not switch to another window.
            if now < next_action {
                return glib::ControlFlow::Continue;
            }
            let Some(app) = app.upgrade() else {
                wait.report(now, step, "application-gone");
                return glib::ControlFlow::Continue;
            };
            if !attached
                && let Some(first) = app
                    .windows()
                    .into_iter()
                    .find_map(|w| w.downcast::<Window>().ok())
            {
                window.set(Some(&first));
                attached = true;
            }
            let Some(window) = window.upgrade().filter(WidgetExt::is_mapped) else {
                wait.report(now, step, "window-unmapped-or-gone");
                return glib::ControlFlow::Continue;
            };
            let view = window.tab_view();
            let Some(page) = view.selected_page() else {
                wait.report(now, step, "no-selected-tab");
                return glib::ControlFlow::Continue;
            };
            let Ok(tab) = page.child().downcast::<Tab>() else {
                wait.report(now, step, "selected-child-not-tab");
                return glib::ControlFlow::Continue;
            };
            let split = tab.get_split_tree();
            let Some(surface) = split.get_active_surface() else {
                if wait.report(now, step, "no-active-surface") {
                    recover_target(&window, &split, step);
                }
                return glib::ControlFlow::Continue;
            };
            // Wait for asynchronous tree attachment and normal surface startup.
            if !surface.is_realized() || surface.core().is_none() {
                if wait.report(now, step, "surface-not-ready") {
                    eprintln!(
                        "GTK-STRESS target surface={:?} realized={} mapped={} core={} zoomed={} width={} height={}",
                        surface.id(),
                        surface.is_realized(),
                        surface.is_mapped(),
                        surface.core().is_some(),
                        split.get_is_zoomed(),
                        surface.width(),
                        surface.height(),
                    );
                    recover_target(&window, &split, step);
                }
                return glib::ControlFlow::Continue;
            }
            let panes = split.get_tree().map_or(0, |tree| tree.iterator().count());
            if panes == 0 {
                wait.report(now, step, "empty-tree");
                return glib::ControlFlow::Continue;
            }
            let tabs = view.n_pages() as usize;
            wait.resumed(now, step);
            let actions = choices(tabs, panes);
            let action = actions[random.index(actions.len())];
            let direction = ["left", "right", "up", "down"][random.index(4)];
            let selected = view.page_position(&page) as usize;
            let target_tab = if tabs > 1 {
                (selected + 1 + random.index(tabs - 1)) % tabs
            } else {
                selected
            };
            let delay = interval + random.next() % (interval * 2 + 1);
            next_action = Instant::now() + Duration::from_millis(delay);
            step += 1;
            // stderr is independent of the user's log filters and records the
            // attempted operation before a synchronous GTK/NVIDIA crash can occur.
            eprintln!(
                "GTK-STRESS step={step} seed={seed} tabs={tabs} tab={selected} panes={panes} zoomed={} surface={:?} action={action:?} direction={direction} target-tab={target_tab} delay-ms={delay}",
                split.get_is_zoomed(),
                surface.id(),
            );
            match action {
                Action::Split => {
                    if let Err(error) = surface
                        .activate_action("split-tree.new-split", Some(&direction.to_variant()))
                    {
                        eprintln!("GTK-STRESS split action failed: {error}");
                    }
                }
                Action::ClosePane => surface.close(),
                Action::Zoom => {
                    if let Err(error) = surface.activate_action("split-tree.zoom", None) {
                        eprintln!("GTK-STRESS zoom action failed: {error}");
                    }
                }
                Action::NewTab => window.new_tab(Some(&surface)),
                Action::CloseTab => view.close_page(&page),
                Action::SwitchTab => view.set_selected_page(&view.nth_page(target_tab as i32)),
                Action::FocusPane => {
                    split.goto(Goto::Next);
                }
            }
            glib::ControlFlow::Continue
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_recovery_is_throttled_and_restarts_after_progress() {
        let mut wait = WaitState::default();
        let start = Instant::now();
        // One-second grace period lets normal asynchronous startup finish.
        for ms in 0..1000 {
            assert!(!wait.report(start + Duration::from_millis(ms), 79, "surface-not-ready"));
        }
        assert!(wait.report(start + Duration::from_secs(1), 79, "surface-not-ready"));
        // Changing wait reasons must not defeat the recovery/log rate limit.
        for ms in 1001..2000 {
            assert!(!wait.report(start + Duration::from_millis(ms), 79, "no-active-surface"));
        }
        assert!(wait.report(start + Duration::from_secs(2), 79, "no-active-surface"));
        wait.resumed(start + Duration::from_secs(2), 79);

        let restart = start + Duration::from_secs(3);
        assert!(!wait.report(restart, 80, "surface-not-ready"));
        assert!(!wait.report(
            restart + Duration::from_millis(999),
            80,
            "surface-not-ready"
        ));
        assert!(wait.report(restart + Duration::from_secs(1), 80, "surface-not-ready"));
    }

    #[test]
    fn action_choices_keep_a_session_alive_and_bound_growth() {
        for tabs in 1..=MAX_TABS {
            for panes in 1..=MAX_PANES {
                let actions = choices(tabs, panes);
                assert_ne!(actions, [] as [Action; 0]);
                assert_eq!(actions.contains(&Action::CloseTab), tabs > 1);
                assert_eq!(actions.contains(&Action::ClosePane), panes > 1);
                assert_eq!(actions.contains(&Action::Zoom), panes > 1);
                assert_eq!(actions.contains(&Action::NewTab), tabs < MAX_TABS);
                assert_eq!(actions.contains(&Action::Split), panes < MAX_PANES);
            }
        }
    }
}
