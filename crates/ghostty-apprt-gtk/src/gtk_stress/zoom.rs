//! Alternate isolated three-pane zoom sessions with the ordinary random driver.
//! Match the original core's GL texture dimensions, not its window dimensions.

use std::time::{Duration, Instant};

use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

use super::{MAX_TABS, Random};
use crate::class::{split_tree::SplitTree, surface::Surface, tab::Tab, window::Window};

// From the original core's GTK texture, recorded in
// ../gtkbug_reproducer/OBSERVATIONS.md: BGRA8, stride 2516 bytes.
const TEXTURE_WIDTH: i32 = 629;
const TEXTURE_HEIGHT: i32 = 245;
const ZOOM_DURATION: Duration = Duration::from_secs(10);
const SETTLE: Duration = Duration::from_millis(200);

fn zoom_delay(random: &mut Random) -> Duration {
    // Mostly short, human-like pauses, with occasional longer frame retention.
    let ms = if random.index(4) == 0 {
        700 + random.next() % 1200
    } else {
        80 + random.next() % 221
    };
    Duration::from_millis(ms)
}

fn normal_duration(random: &mut Random) -> Duration {
    Duration::from_millis(5000 + random.next() % 5001)
}

fn pixels(surface: &Surface) -> (i32, i32, i32) {
    let area = surface.imp().gl_area.get();
    let scale = area.scale_factor().max(1);
    (area.width() * scale, area.height() * scale, scale)
}

fn ready(surface: &Surface) -> bool {
    surface.is_mapped() && surface.is_realized() && surface.core().is_some()
}

/// Restore ordinary expansion before returning this tab to random stress.
/// Weak references must not keep removed tabs/panes (or their GL state) alive.
struct Layout {
    split: glib::WeakRef<SplitTree>,
    request: (i32, i32),
    align: (gtk::Align, gtk::Align),
}

impl Layout {
    fn constrain(split: &SplitTree) -> Self {
        let saved = Self {
            split: split.downgrade(),
            request: (split.width_request(), split.height_request()),
            align: (split.halign(), split.valign()),
        };
        // A large left pane and two half-height right panes. Calibration below
        // accounts for dividers, CSS, decoration and the actual GTK scale.
        split.set_halign(gtk::Align::Start);
        split.set_valign(gtk::Align::Start);
        split.set_size_request(1300, 550);
        saved
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        if let Some(split) = self.split.upgrade() {
            split.set_size_request(self.request.0, self.request.1);
            split.set_halign(self.align.0);
            split.set_valign(self.align.1);
        }
    }
}

struct Session {
    tab: glib::WeakRef<Tab>,
    target: glib::WeakRef<Surface>,
    _layout: Layout,
    created: Instant,
    until: Option<Instant>,
    corrections: u8,
}

#[derive(Default)]
enum Phase {
    #[default]
    Begin,
    Zoom(Session),
    Normal(Instant),
}

#[derive(Default)]
pub(super) struct Cycle {
    phase: Phase,
}

impl Cycle {
    fn normal(&mut self, random: &mut Random, now: Instant) {
        let duration = normal_duration(random);
        eprintln!(
            "GTK-STRESS phase=normal duration-ms={}",
            duration.as_millis()
        );
        self.phase = Phase::Normal(now + duration);
    }

    /// Some(delay) owns this timer tick; None delegates to random stress.
    pub(super) fn tick(
        &mut self,
        window: &Window,
        random: &mut Random,
        step: &mut u64,
        now: Instant,
    ) -> Option<Duration> {
        if let Phase::Normal(until) = self.phase {
            if now < until {
                return None;
            }
            self.phase = Phase::Begin;
        }
        let view = window.tab_view();
        if matches!(self.phase, Phase::Begin) {
            // Close a spare tab first if at the cap; never close the last one.
            if view.n_pages() >= MAX_TABS as i32 {
                eprintln!("GTK-STRESS phase=zoom-setup action=close-spare-tab");
                view.close_page(&view.nth_page(0));
                return Some(SETTLE);
            }
            eprintln!(
                "GTK-STRESS phase=zoom-setup action=new-tab target-texture={TEXTURE_WIDTH}x{TEXTURE_HEIGHT}"
            );
            window.new_tab(window.get_active_surface().as_ref());
            let Some(tab) = view
                .selected_page()
                .and_then(|p| p.child().downcast::<Tab>().ok())
            else {
                self.normal(random, now);
                return Some(SETTLE);
            };
            self.phase = Phase::Zoom(Session {
                tab: tab.downgrade(),
                target: glib::WeakRef::new(),
                _layout: Layout::constrain(&tab.get_split_tree()),
                created: now,
                until: None,
                corrections: 0,
            });
            return Some(SETTLE);
        }
        let Phase::Zoom(session) = &mut self.phase else {
            unreachable!()
        };
        let page = session.tab.upgrade().and_then(|tab| {
            (0..view.n_pages())
                .map(|i| view.nth_page(i))
                .find(|p| p.child() == tab)
        });
        let Some(page) = page else {
            eprintln!("GTK-STRESS phase=zoom-aborted reason=tab-removed");
            self.normal(random, now);
            return Some(SETTLE);
        };
        if view.selected_page().as_ref() != Some(&page) {
            view.set_selected_page(&page);
            return Some(SETTLE);
        }
        let tab = page.child().downcast::<Tab>().expect("session tab");
        let split = tab.get_split_tree();
        if session.until.is_some_and(|until| now >= until) {
            if split.get_is_zoomed() {
                *step += 1;
                eprintln!("GTK-STRESS step={step} phase=zoom-end action=Zoom zoomed=true");
                if let Err(error) = split.activate_action("split-tree.zoom", None) {
                    eprintln!("GTK-STRESS final unzoom failed: {error}");
                }
            }
            self.normal(random, now);
            return Some(SETTLE);
        }
        if session.until.is_none() && now.duration_since(session.created) > Duration::from_secs(10)
        {
            eprintln!("GTK-STRESS phase=zoom-aborted reason=setup-timeout");
            self.normal(random, now);
            return Some(SETTLE);
        }
        let Some(tree) = split.get_tree() else {
            return Some(SETTLE);
        };
        let panes = tree.iterator().count();
        if panes < 3 && session.until.is_none() {
            if let Some(surface) = split.get_active_surface().filter(ready) {
                let direction = if panes == 1 { "right" } else { "down" };
                *step += 1;
                eprintln!(
                    "GTK-STRESS step={step} phase=zoom-setup panes={panes} action=Split direction={direction}"
                );
                surface.grab_focus();
                if let Err(error) =
                    split.activate_action("split-tree.new-split", Some(&direction.to_variant()))
                {
                    eprintln!("GTK-STRESS zoom-setup split failed: {error}");
                }
            }
            return Some(SETTLE);
        }
        if panes < 3 {
            eprintln!("GTK-STRESS phase=zoom-aborted reason=panes-removed");
            self.normal(random, now);
            return Some(SETTLE);
        }
        if session.target.upgrade().is_none() && session.until.is_none() {
            // Do not accidentally choose the large pane just because one of
            // the smaller panes has not finished its first frame yet.
            if !tree.iterator().all(|entry| ready(entry.view)) {
                return Some(SETTLE);
            }
            let smallest = tree.iterator().filter(|e| ready(e.view)).min_by_key(|e| {
                let (w, h, _) = pixels(e.view);
                i64::from(w) * i64::from(h)
            });
            if let Some(entry) = smallest {
                session.target.set(Some(entry.view));
            }
        }
        let Some(target) = session
            .target
            .upgrade()
            .filter(|s| tree.iterator().any(|e| e.view == s))
        else {
            return Some(SETTLE);
        };
        if !ready(&target) {
            target.grab_focus();
            return Some(SETTLE);
        }
        target.grab_focus();
        if split.get_active_surface().as_ref() != Some(&target) {
            return Some(SETTLE);
        }
        let (width, height, scale) = pixels(&target);
        if session.until.is_none() {
            let matched = (width, height) == (TEXTURE_WIDTH, TEXTURE_HEIGHT);
            // GTK allocations are integral logical pixels. An odd physical
            // width cannot be represented at scale 2; report that explicitly.
            let representable = TEXTURE_WIDTH % scale == 0 && TEXTURE_HEIGHT % scale == 0;
            if !matched && representable && session.corrections < 20 {
                let w = (split.width() + 2 * (TEXTURE_WIDTH - width) / scale).clamp(1, 16384);
                let h = (split.height() + 2 * (TEXTURE_HEIGHT - height) / scale).clamp(1, 16384);
                session.corrections += 1;
                eprintln!(
                    "GTK-STRESS phase=zoom-sizing measured={width}x{height} scale={scale} request={w}x{h} attempt={}",
                    session.corrections
                );
                split.set_size_request(w, h);
                return Some(SETTLE);
            }
            eprintln!(
                "GTK-STRESS phase=zoom duration-ms=10000 panes={panes} surface={:?} texture={width}x{height} target={TEXTURE_WIDTH}x{TEXTURE_HEIGHT} exact={matched} scale={scale} settle-ms=500",
                target.id()
            );
            if !matched {
                eprintln!(
                    "GTK-STRESS size-mismatch: GTK allocation/scale prevented exact texture size; use GDK_SCALE=1 for odd pixel dimensions"
                );
            }
            session.until = Some(now + ZOOM_DURATION);
            return Some(Duration::from_millis(500));
        }
        let delay = zoom_delay(random);
        *step += 1;
        eprintln!(
            "GTK-STRESS step={step} phase=zoom panes={panes} zoomed={} surface={:?} texture={width}x{height} action=Zoom delay-ms={}",
            split.get_is_zoomed(),
            target.id(),
            delay.as_millis()
        );
        if let Err(error) = split.activate_action("split-tree.zoom", None) {
            eprintln!("GTK-STRESS zoom action failed: {error}");
        }
        // Wake at the phase deadline even if the sampled pause would cross it.
        Some(
            delay.min(
                session
                    .until
                    .expect("zoom deadline")
                    .saturating_duration_since(now),
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_timings_include_short_and_sporadic_long_pauses() {
        let mut random = Random(42);
        let mut short = 0;
        let mut long = 0;
        for _ in 0..1000 {
            let ms = zoom_delay(&mut random).as_millis();
            assert!((80..=300).contains(&ms) || (700..1900).contains(&ms));
            if ms < 700 {
                short += 1;
            } else {
                long += 1;
            }
            assert!((5000..=10000).contains(&normal_duration(&mut random).as_millis()));
        }
        assert!(short > long && long > 0);
    }
}
