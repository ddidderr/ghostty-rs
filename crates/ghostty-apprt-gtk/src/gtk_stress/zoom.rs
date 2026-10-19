//! Alternate zoom-only and random operations on the existing tabs and panes.
//! Phase changes preserve the current layout, focus, and zoom state.

use std::time::{Duration, Instant};

use adw::{prelude::*, subclass::prelude::*};

use super::Random;
use crate::class::{surface::Surface, tab::Tab, window::Window};

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

fn pixels(surface: &Surface) -> (i32, i32) {
    let area = surface.imp().gl_area.get();
    let scale = area.scale_factor().max(1);
    (area.width() * scale, area.height() * scale)
}

fn ready(surface: &Surface) -> bool {
    surface.is_mapped() && surface.is_realized() && surface.core().is_some()
}

#[derive(Default)]
enum Phase {
    #[default]
    Begin,
    Zoom(Instant),
    Normal(Instant),
}

#[derive(Default)]
pub(super) struct Cycle {
    id: usize,
    phase: Phase,
}

impl Cycle {
    pub(super) fn new(id: usize) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }

    fn normal(&mut self, random: &mut Random, now: Instant) {
        let id = self.id;
        let duration = normal_duration(random);
        eprintln!(
            "GTK-STRESS window={id} phase=normal duration-ms={}",
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
        let id = self.id;
        match self.phase {
            Phase::Begin => {
                // Let random operations establish the layout for the first zoom phase.
                self.normal(random, now);
                return None;
            }
            Phase::Normal(until) => {
                if now < until {
                    return None;
                }
                eprintln!(
                    "GTK-STRESS window={id} phase=zoom duration-ms={}",
                    ZOOM_DURATION.as_millis()
                );
                self.phase = Phase::Zoom(now + ZOOM_DURATION);
            }
            Phase::Zoom(until) if now >= until => {
                self.normal(random, now);
                return None;
            }
            Phase::Zoom(_) => {}
        }
        let Phase::Zoom(until) = self.phase else {
            unreachable!()
        };
        // The deadline applies even when the existing layout cannot zoom or a
        // surface is still starting up. Never manufacture panes for this phase.
        let remaining = until.saturating_duration_since(now);
        let wait = Some(SETTLE.min(remaining));
        let view = window.tab_view();
        let Some(tab) = view
            .selected_page()
            .and_then(|page| page.child().downcast::<Tab>().ok())
        else {
            return wait;
        };
        let split = tab.get_split_tree();
        let panes = split.get_tree().map_or(0, |tree| tree.iterator().count());
        if panes < 2 {
            return wait;
        }
        let Some(target) = split.get_active_surface().filter(ready) else {
            return wait;
        };
        let (width, height) = pixels(&target);
        let delay = zoom_delay(random).min(remaining);
        *step += 1;
        eprintln!(
            "GTK-STRESS window={id} step={step} phase=zoom panes={panes} zoomed={} surface={:?} texture={width}x{height} action=Zoom delay-ms={}",
            split.get_is_zoomed(),
            target.id(),
            delay.as_millis()
        );
        if let Err(error) = split.activate_action("split-tree.zoom", None) {
            eprintln!("GTK-STRESS window={id} zoom action failed: {error}");
        }
        Some(delay)
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
