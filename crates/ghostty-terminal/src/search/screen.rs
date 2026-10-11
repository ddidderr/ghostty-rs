//! Searches for a needle within a Screen, handling active area updates,
//! pages being pruned from the screen (e.g. scrollback limits), and more.
//!
//! Unlike our lower-level searchers (like `ActiveSearch` and
//! `PageListSearch`), this will cache and store all search results so the
//! caller can re-access them as needed. This structure does this because it
//! is intended to help the caller handle the case where the Screen is
//! changing while the user is searching.
//!
//! An inactive screen can continue to be searched in the background, and when
//! screen state changes, the renderer/caller can access the existing search
//! results without needing to re-search everything. This prevents a particularly
//! nasty UX where going to alt screen (e.g. neovim) and then back would
//! restart the full scrollback search.
//!
//! Deviation: Zig stores `*Screen`. Here the screen is passed to every method
//! that touches it (`new`, `deinit`, `feed`, `reload_active`, `select`,
//! `search_all`); `tick`, `matches`, `selected_match` etc. do not need it.

use super::active::ActiveSearch;
use super::pagelist::PageListSearch;
use super::sliding_window::{Direction, SlidingWindow};
use crate::highlight::{Flattened, Tracked};
use crate::page::Error;
use crate::page_list::{NodeId, PageList, Pin, TrackedPin};
use crate::screen::Screen;
use crate::size::CellCountInt;

#[derive(Debug)]
pub struct ScreenSearch {
    /// Identity of the screen being searched (its address). Only used to
    /// detect that a screen was replaced; never dereferenced.
    screen_id: usize,

    /// The active area search state
    pub active: ActiveSearch,

    /// The history (scrollback) search state. May be `None` if there is
    /// no history yet.
    history: Option<HistorySearch>,

    /// Current state of the search, a state machine.
    pub state: State,

    /// The currently selected match, if any. As the screen contents
    /// change or get pruned, the screen search will do its best to keep
    /// this accurate.
    pub selected: Option<SelectedMatch>,

    /// The results found so far. These are stored separately because history
    /// is mostly immutable once found, while active area results may
    /// change. This lets us easily reset the active area results for a
    /// re-search scenario.
    pub history_results: Vec<Flattened>,
    pub active_results: Vec<Flattened>,

    /// The dimensions of the screen. When this changes we need to
    /// restart the whole search, currently.
    rows: CellCountInt,
    cols: CellCountInt,
}

#[derive(Debug)]
pub struct SelectedMatch {
    /// Index from the end of the match list (0 = most recent match)
    pub idx: usize,

    /// Tracked highlight so we can detect movement.
    pub highlight: Tracked,
}

impl SelectedMatch {
    pub fn deinit(self, screen: &mut Screen) {
        self.highlight.deinit(screen);
    }
}

/// History search state.
#[derive(Debug)]
struct HistorySearch {
    /// The actual searcher state.
    searcher: PageListSearch,

    /// The pin for the first node that this searcher is searching from.
    /// We use this when the active area changes to find the diff between
    /// the top of the new active area and the previous start point
    /// to determine if we need to search more history.
    start_pin: TrackedPin,

    /// Cached node of `start_pin`. Zig reads `start_pin.node` during `tick`
    /// without a lock; here the tracked pin lives in the PageList so the
    /// node is cached and refreshed whenever the screen is locked
    /// (`feed`, `reload_active`).
    start_node: NodeId,
}

impl HistorySearch {
    fn deinit(mut self, screen: &mut Screen) {
        self.searcher.deinit(&mut screen.pages);
        screen.pages.untrack_pin(self.start_pin);
    }
}

/// Search state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Currently searching the active area
    Active,

    /// Currently searching the history area
    History,

    /// History search is waiting for more data to be fed before
    /// it can progress.
    HistoryFeed,

    /// Search is complete given the current terminal state.
    Complete,
}

impl State {
    pub fn is_complete(self) -> bool {
        matches!(self, State::Complete)
    }

    pub fn needs_feed(self) -> bool {
        match self {
            State::HistoryFeed => true,

            // Not obvious but complete search states will prune
            // stale history results on feed.
            State::Complete => true,

            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Select {
    /// Next selection, in reverse order (newest to oldest),
    /// non-wrapping.
    Next,

    /// Prev selection, in forward order (oldest to newest),
    /// non-wrapping.
    Prev,
}

/// Errors from `tick`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TickError {
    #[error("feed required")]
    FeedRequired,
    #[error("search complete")]
    SearchComplete,
    #[error("page error: {0}")]
    Page(Error),
}

fn screen_id(screen: &Screen) -> usize {
    std::ptr::from_ref::<Screen>(screen) as usize
}

fn pin_garbage(list: &PageList, h: TrackedPin) -> bool {
    // A pin that can't be resolved is as good as garbage.
    list.tracked_pin(h).is_none_or(|p| p.garbage)
}

impl ScreenSearch {
    /// Initialize a screen search for the given screen and needle.
    pub fn new(screen: &mut Screen, needle_unowned: &[u8]) -> Result<Self, Error> {
        let mut result = ScreenSearch {
            screen_id: screen_id(screen),
            rows: screen.pages.rows,
            cols: screen.pages.cols,
            active: ActiveSearch::new(needle_unowned),
            history: None,
            state: State::Active,
            selected: None,
            active_results: Vec::new(),
            history_results: Vec::new(),
        };

        // Update our initial active area state
        if let Err(e) = result.reload_active(screen) {
            result.deinit(screen);
            return Err(e);
        }

        Ok(result)
    }

    /// Release tracked pins held in the screen.
    pub fn deinit(&mut self, screen: &mut Screen) {
        if let Some(h) = self.history.take() {
            h.deinit(screen);
        }
        if let Some(m) = self.selected.take() {
            m.deinit(screen);
        }
    }

    /// The address identity of the screen this search was created for.
    pub fn screen_id(&self) -> usize {
        self.screen_id
    }

    /// True if this search was created for the given screen (by identity).
    pub fn is_for_screen(&self, screen: &Screen) -> bool {
        self.screen_id == screen_id(screen)
    }

    /// The needle that this search is using.
    pub fn needle(&self) -> &[u8] {
        assert_eq!(self.active.window.direction(), Direction::Forward);
        self.active.window.needle()
    }

    /// Returns the total number of matches found so far.
    pub fn matches_len(&self) -> usize {
        self.active_results.len() + self.history_results.len()
    }

    /// Returns all matches. The matches are ordered from most recent to
    /// oldest (e.g. bottom of the screen to top of the screen).
    pub fn matches(&self) -> Vec<Flattened> {
        // Active does a forward search, so we add the active results then
        // reverse them. There are usually not many active results so this
        // is fast enough compared to adding them in reverse order.
        assert_eq!(self.active.window.direction(), Direction::Forward);
        let mut results: Vec<Flattened> = self.active_results.clone();
        results.reverse();

        // History does a backward search, so we can just append them after.
        results.extend(self.history_results.iter().cloned());
        results
    }

    /// Search the full screen state. This will block until the search
    /// is complete. For performance, it is recommended to use `tick` and
    /// `feed` to incrementally make progress on the search instead.
    pub fn search_all(&mut self, screen: &mut Screen) -> Result<(), Error> {
        loop {
            match self.tick() {
                Ok(()) => {}
                Err(TickError::Page(e)) => return Err(e),
                Err(TickError::FeedRequired) => self.feed(screen)?,
                Err(TickError::SearchComplete) => return Ok(()),
            }
        }
    }

    /// Make incremental progress on the search without accessing any
    /// screen state (so no lock is required).
    ///
    /// This will return `FeedRequired` if the search cannot make progress
    /// without being fed more data. In this case, the caller should call
    /// the `feed` function to provide more data to the searcher.
    ///
    /// This will return `SearchComplete` if the search is fully complete.
    /// This is to signal to the caller that it can move to a more efficient
    /// sleep/wait state until there is more work to do (e.g. new data to feed).
    pub fn tick(&mut self) -> Result<(), TickError> {
        match self.state {
            State::Active => self.tick_active(),
            State::History => self.tick_history(),
            State::HistoryFeed => return Err(TickError::FeedRequired),
            State::Complete => return Err(TickError::SearchComplete),
        }
        Ok(())
    }

    /// Feed more data to the searcher so it can continue searching. This
    /// accesses the screen state, so the caller must hold the necessary locks.
    ///
    /// Feed on a complete screen search will perform some cleanup of
    /// potentially stale history results (pruned) and reclaim some memory.
    pub fn feed(&mut self, screen: &mut Screen) -> Result<(), Error> {
        // If the screen resizes, we have to reset our entire search. That
        // isn't ideal but we don't have a better way right now to handle
        // reflowing the search results beyond putting a tracked pin for
        // every single result.
        if screen.pages.rows != self.rows || screen.pages.cols != self.cols {
            // Reinit
            let needle = self.needle().to_vec();
            let new = ScreenSearch::new(screen, &needle)?;

            // Deinit/reinit
            self.deinit(screen);
            *self = new;

            // New result should have matching dimensions
            assert_eq!(screen.pages.rows, self.rows);
            assert_eq!(screen.pages.cols, self.cols);
        }

        let Some(h) = self.history.as_mut() else {
            // No history to feed, search is complete.
            self.state = State::Complete;
            return Ok(());
        };

        // Future: we may want to feed multiple pages at once here to
        // lower the frequency of lock acquisitions.
        if !h.searcher.feed(&mut screen.pages)? {
            // No more data to feed, search is complete.
            self.state = State::Complete;

            // We use this opportunity to also clean up older history
            // results that may be gone due to scrollback pruning, though.
            self.prune_history(screen);

            return Ok(());
        }

        // Depending on our state handle where feed goes
        match self.state {
            // If we're searching active or history, then feeding doesn't
            // change the state.
            State::Active | State::History => {}

            // Feed goes back to searching history.
            State::HistoryFeed => self.state = State::History,

            // If we're complete then the feed call above should always
            // return false and we can't reach this.
            State::Complete => unreachable!(),
        }
        Ok(())
    }

    fn prune_history(&mut self, screen: &Screen) {
        // Go through our history results in order (newest to oldest) to find
        // any result that contains an invalid serial. Prune up to that
        // point.
        for i in 0..self.history_results.len() {
            let hl = &self.history_results[i];
            let lowest = hl.chunks[0].serial;
            if lowest < screen.pages.page_serial_min {
                // Everything from here forward we assume is invalid because
                // our history results only get older.
                self.history_results.truncate(i);
                self.history_results.shrink_to_fit();
                return;
            }
        }
    }

    fn tick_active(&mut self) {
        // For the active area, we consume the entire search in one go
        // because the active area is generally small.
        while let Some(hl) = self.active.next() {
            self.active_results.push(hl);
        }

        // We've consumed the entire active area, move to history.
        self.state = State::History;
    }

    fn tick_history(&mut self) {
        let Some(history) = self.history.as_mut() else {
            // No history to search, we're done.
            self.state = State::Complete;
            return;
        };

        // Try to consume all the loaded matches in one go, because
        // the search is generally fast for loaded data.
        while let Some(hl) = history.searcher.next() {
            // Ignore selections that are found within the starting
            // node since those are covered by the active area search.
            if hl.chunks[0].node == history.start_node {
                continue;
            }

            self.history_results.push(hl);

            // Since history only appends to our results in reverse order,
            // we don't need to update any selected match state. The index
            // and prior results are unaffected.
        }

        // We need to be fed more data.
        self.state = State::HistoryFeed;
    }

    /// Reload the active area because it has changed.
    ///
    /// Since it is very fast, this will also do the full active area
    /// search again, too. This avoids any complexity around the search
    /// state machine.
    ///
    /// The caller must hold the necessary locks to access the screen state.
    pub fn reload_active(&mut self, screen: &mut Screen) -> Result<(), Error> {
        // If our selection pin became garbage it means we scrolled off
        // the end. Clear our selection and on exit of this function,
        // try to select the last match.
        let select_prev: bool = 'select_prev: {
            let Some(m) = &self.selected else {
                break 'select_prev false;
            };
            if !pin_garbage(&screen.pages, m.highlight.start)
                && !pin_garbage(&screen.pages, m.highlight.end)
            {
                break 'select_prev false;
            }

            if let Some(m) = self.selected.take() {
                m.deinit(screen);
            }
            true
        };

        let result = self.reload_active_inner(screen);

        if select_prev && let Err(e) = self.select(screen, Select::Prev) {
            log::info!("reload failed to reset search selection err={e:?}");
        }

        result
    }

    /// Drop all history state and results.
    fn reset_history(&mut self, screen: &mut Screen) {
        if let Some(h) = self.history.take() {
            h.deinit(screen);
        }
        self.history_results.clear();
    }

    fn reload_active_inner(&mut self, screen: &mut Screen) -> Result<(), Error> {
        if let Some(history_node) = self.active.update(&screen.pages) {
            self.reload_history(screen, history_node)?;
        } else {
            // No history node means we have no history
            if self.history.is_some() {
                self.reset_history(screen);
            }

            // If we have a selection in the history area, we need to
            // move it to the end of the active area.
            if let Some(m) = &self.selected {
                let active_len = self.active_results.len();
                if m.idx >= active_len {
                    if let Some(m) = self.selected.take() {
                        m.deinit(screen);
                    }
                    if let Err(e) = self.select(screen, Select::Prev) {
                        log::info!("reload failed to reset search selection err={e:?}");
                    }
                }
            }
        }

        // Figure out if we need to fixup our selection later because
        // it was in the active area.
        let old_active_len = self.active_results.len();
        let old_selection_idx: Option<usize> = self.selected.as_ref().map(|m| m.idx);

        if let Err(e) = self.reload_active_results(screen, old_active_len, old_selection_idx) {
            // This is the error scenario. If something fails below,
            // our active area is probably gone, so we just go back
            // to the first result because our selection can't be trusted.
            if old_selection_idx.is_some_and(|i| i < old_active_len)
                && let Some(m) = self.selected.take()
            {
                m.deinit(screen);
                if let Err(e) = self.select(screen, Select::Next) {
                    log::info!("reload failed to reset search selection err={e:?}");
                }
            }
            return Err(e);
        }

        Ok(())
    }

    /// The `history:` block of Zig's `reloadActive`: account for any active
    /// area growth that would cause new pages to move into our history.
    fn reload_history(&mut self, screen: &mut Screen, history_node: NodeId) -> Result<(), Error> {
        // If our screen has no scrollback then we have no history.
        if screen.no_scrollback {
            assert!(self.history.is_none());
            return Ok(());
        }

        // If our start pin became garbage, it means we pruned all
        // the way up through it, so we have no history anymore.
        // Reset our history state.
        if let Some(h) = &self.history
            && pin_garbage(&screen.pages, h.start_pin)
        {
            self.reset_history(screen);
        }

        let Some(history) = self.history.as_mut() else {
            // No history search yet, but we now have history. So let's
            // initialize.
            let needle = self.needle().to_vec();
            let mut search = PageListSearch::new(&needle, &mut screen.pages, history_node)?;

            let pin = match screen.pages.track_pin(Pin {
                node: history_node,
                y: 0,
                x: 0,
                garbage: false,
            }) {
                Ok(p) => p,
                Err(e) => {
                    search.deinit(&mut screen.pages);
                    return Err(e);
                }
            };

            self.history = Some(HistorySearch {
                searcher: search,
                start_pin: pin,
                start_node: history_node,
            });

            // We don't need to update any history since we had no history
            // before, so we can break out of the whole conditional.
            return Ok(());
        };

        let mut start = screen
            .pages
            .tracked_pin(history.start_pin)
            .ok_or(Error::InvalidId)?;
        history.start_node = start.node;
        if start.node == history_node {
            // No change in the starting node, we're done.
            return Ok(());
        }

        // Do a forward search from our prior node to this one. We
        // collect all the results into a new list. We ASSUME that
        // reloadActive is being called frequently enough that there isn't
        // a massive amount of history to search here.
        let needle = self.active.window.needle().to_vec();
        let mut window = SlidingWindow::new(Direction::Forward, &needle);
        loop {
            window.append(&screen.pages, start.node);
            if start.node == history_node {
                break;
            }
            let Some(next) = screen.pages.node(start.node).and_then(|n| n.next) else {
                break;
            };
            start = Pin {
                node: next,
                y: 0,
                x: 0,
                garbage: false,
            };
            screen.pages.set_tracked_pin(history.start_pin, start)?;
        }
        history.start_node = start.node;
        assert_eq!(start.node, history_node);

        let mut results: Vec<Flattened> = Vec::with_capacity(self.history_results.len());
        while let Some(hl) = window.next() {
            if hl.chunks[0].node == history_node {
                continue;
            }
            results.push(hl);
        }

        // If we have no matches then there is nothing to change
        // in our history (fast path)
        if results.is_empty() {
            return Ok(());
        }

        // The number added to our history. Needed for updating
        // our selection if we have one.
        let added_len = results.len();

        // Matches! Reverse our list then append all the remaining
        // history items that didn't start on our original node.
        results.reverse();
        results.append(&mut self.history_results);
        self.history_results = results;

        // If our prior selection was in the history area, update
        // the offset.
        if let Some(m) = &mut self.selected {
            let active_len = self.active_results.len();
            if m.idx >= active_len {
                m.idx += added_len;

                // Moving the idx should not change our targeted result
                // since the history is immutable.
                if cfg!(debug_assertions) {
                    let hl = &self.history_results[m.idx - active_len];
                    let sp = screen.pages.tracked_pin(m.highlight.start);
                    debug_assert!(sp.is_some_and(|p| p.eql(hl.start_pin())));
                }
            }
        }

        Ok(())
    }

    /// The tail of Zig's `reloadActive`: reset our active search results,
    /// search again and fix up the selection.
    fn reload_active_results(
        &mut self,
        screen: &mut Screen,
        old_active_len: usize,
        old_selection_idx: Option<usize>,
    ) -> Result<(), Error> {
        // Reset our active search results and search again.
        self.active_results.clear();
        match self.state {
            // If we're in the active state we run a normal tick so
            // we can move into a better state.
            State::Active => self.tick_active(),

            // Otherwise, just tick it and move back to whatever state
            // we were in.
            old_state => {
                self.tick_active();
                self.state = old_state;
            }
        }

        // If we have no scrollback, we need to prune any active results
        // that aren't in the actual active area. We only do this for the
        // no scrollback scenario because with scrollback we actually
        // rely on our active search searching by page to find history
        // items as well. This is all related to the fact that PageList
        // scrollback limits are discrete by page size except we special
        // case zero.
        if screen.no_scrollback && !self.active_results.is_empty() {
            let tl = screen
                .pages
                .get_top_left(crate::point::Tag::Active)
                .ok_or(Error::InvalidCoordinate)?;
            let mut keep_from: Option<usize> = None;
            for (i, hl) in self.active_results.iter().enumerate() {
                if !tl.before(&screen.pages, hl.end_pin()) {
                    // Pruned no matter what at some point for not being in
                    // the active area.
                    continue;
                }

                // In the active area! Since our results are sorted
                // that means everything after this is also in the active
                // area, so we prune up to this i.
                keep_from = Some(i);
                break;
            }
            match keep_from {
                Some(i) => {
                    self.active_results.drain(..i);
                }
                // None are in the active area...
                None => self.active_results.clear(),
            }
        }

        // Now we have to fixup our selection if we had one.
        'fixup: {
            let Some(old_idx) = old_selection_idx else {
                break 'fixup;
            };
            let Some(m) = self.selected.as_mut() else {
                break 'fixup;
            };

            // If our old selection wasn't in the active area, then we
            // need to fix up our offsets.
            if old_idx >= old_active_len {
                m.idx -= old_active_len;
                m.idx += self.active_results.len();
                break 'fixup;
            }

            // We search for the matching highlight in the new active results.
            let ms = screen.pages.tracked_pin(m.highlight.start);
            let me = screen.pages.tracked_pin(m.highlight.end);
            if let (Some(ms), Some(me)) = (ms, me) {
                for (i, hl) in self.active_results.iter().enumerate() {
                    let untracked = hl.untracked();
                    if ms.eql(untracked.start) && me.eql(untracked.end) {
                        // Found it! Update our index.
                        m.idx = self.active_results.len() - 1 - i;
                        break 'fixup;
                    }
                }
            }

            // No match, just go back to the first match.
            if let Some(m) = self.selected.take() {
                m.deinit(screen);
            }
            if let Err(e) = self.select(screen, Select::Next) {
                log::info!("reload failed to reset search selection err={e:?}");
            }
        }

        Ok(())
    }

    /// Return the selected match.
    ///
    /// This does not require read/write access to the underlying screen.
    pub fn selected_match(&self) -> Option<Flattened> {
        let sel = self.selected.as_ref()?;
        let active_len = self.active_results.len();
        if sel.idx < active_len {
            return Some(self.active_results[active_len - 1 - sel.idx].clone());
        }

        let history_len = self.history_results.len();
        if sel.idx < active_len + history_len {
            return Some(self.history_results[sel.idx - active_len].clone());
        }

        None
    }

    /// Select the next or previous search result. This requires read/write
    /// access to the underlying screen, since we utilize tracked pins to
    /// ensure our selection sticks with contents changing.
    pub fn select(&mut self, screen: &mut Screen, to: Select) -> Result<bool, Error> {
        // All selection requires valid pins so we prune history and
        // reload our active area immediately. This ensures all search
        // results point to valid nodes.
        self.reload_active(screen)?;
        self.prune_history(screen);

        match to {
            Select::Next => self.select_next(screen),
            Select::Prev => self.select_prev(screen),
        }
    }

    fn select_next(&mut self, screen: &mut Screen) -> Result<bool, Error> {
        // Get our previous match so we can change it. If we have no
        // prior match, we have the easy task of getting the first.
        let Some(prev_idx) = self.selected.as_ref().map(|m| m.idx) else {
            // Get our highlight
            let hl: &Flattened = if let Some(hl) = self.active_results.last() {
                // Active is in forward order
                hl
            } else if let Some(hl) = self.history_results.first() {
                // History is in reverse order
                hl
            } else {
                // No matches at all. Can't select anything.
                return Ok(false);
            };

            // Pin it so we can track any movement
            let tracked = hl.untracked().track(screen)?;

            // Our selection is index zero since we just started and
            // we store our selection.
            self.selected = Some(SelectedMatch {
                idx: 0,
                highlight: tracked,
            });
            return Ok(true);
        };

        let next_idx = prev_idx + 1;
        let active_len = self.active_results.len();
        let history_len = self.history_results.len();
        if next_idx >= active_len + history_len {
            // No more matches. We don't wrap or reset the match currently.
            return Ok(false);
        }
        let hl: &Flattened = if next_idx < active_len {
            &self.active_results[active_len - 1 - next_idx]
        } else {
            &self.history_results[next_idx - active_len]
        };

        // Pin it so we can track any movement
        let tracked = hl.untracked().track(screen)?;

        // Free our previous match and setup our new selection
        if let Some(prev) = self.selected.take() {
            prev.deinit(screen);
        }
        self.selected = Some(SelectedMatch {
            idx: next_idx,
            highlight: tracked,
        });

        Ok(true)
    }

    fn select_prev(&mut self, screen: &mut Screen) -> Result<bool, Error> {
        // Get our previous match so we can change it. If we have no
        // prior match, we have the easy task of getting the last.
        let Some(prev_idx) = self.selected.as_ref().map(|m| m.idx) else {
            // Get our highlight (oldest match)
            let hl: &Flattened = if let Some(hl) = self.history_results.last() {
                // History is in reverse order, so last item is oldest
                hl
            } else if let Some(hl) = self.active_results.first() {
                // Active is in forward order, so first item is oldest
                hl
            } else {
                // No matches at all. Can't select anything.
                return Ok(false);
            };

            // Pin it so we can track any movement
            let tracked = hl.untracked().track(screen)?;

            // Our selection is the last index since we just started
            // and we store our selection.
            let active_len = self.active_results.len();
            let history_len = self.history_results.len();
            self.selected = Some(SelectedMatch {
                idx: active_len + history_len - 1,
                highlight: tracked,
            });
            return Ok(true);
        };

        // Can't go below zero
        if prev_idx == 0 {
            // No more matches. We don't wrap or reset the match currently.
            return Ok(false);
        }

        let next_idx = prev_idx - 1;
        let active_len = self.active_results.len();
        let hl: &Flattened = if next_idx < active_len {
            &self.active_results[active_len - 1 - next_idx]
        } else {
            &self.history_results[next_idx - active_len]
        };

        // Pin it so we can track any movement
        let tracked = hl.untracked().track(screen)?;

        // Free our previous match and setup our new selection
        if let Some(prev) = self.selected.take() {
            prev.deinit(screen);
        }
        self.selected = Some(SelectedMatch {
            idx: next_idx,
            highlight: tracked,
        });

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::*;
    use super::*;
    use crate::point::{Point, Tag};
    use crate::terminal::Terminal;

    fn pts(t: &Terminal, h: &Flattened, tag: Tag) -> (Option<Point>, Option<Point>) {
        let sel = h.untracked();
        (
            t.screen().pages.point_from_pin(tag, sel.start),
            t.screen().pages.point_from_pin(tag, sel.end),
        )
    }

    fn expect(t: &Terminal, h: &Flattened, tag: Tag, s: (u16, u32), e: (u16, u32)) {
        assert_eq!(
            pts(t, h, tag),
            (Some(pt(tag, s.0, s.1)), Some(pt(tag, e.0, e.1)))
        );
    }

    fn expect_start(t: &Terminal, h: &Flattened, tag: Tag, s: (u16, u32)) {
        assert_eq!(pts(t, h, tag).0, Some(pt(tag, s.0, s.1)));
    }

    fn total_pages(t: &Terminal) -> usize {
        t.screen().pages.total_pages()
    }

    #[test]
    fn simple_search() {
        let mut t = terminal(10, 2);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();
        assert_eq!(2, search.active_results.len());
        // We don't test history results since there is overlap

        let matches = search.matches();
        assert_eq!(2, matches.len());
        expect(&t, &matches[0], Tag::Screen, (0, 2), (3, 2));
        expect(&t, &matches[1], Tag::Screen, (0, 0), (3, 0));
        search.deinit(t.screen_mut());
    }

    #[test]
    fn simple_search_with_history() {
        let mut t = terminal_scrollback(10, 2, usize::MAX);
        feed(&mut t, b"Fizz\r\n");
        while total_pages(&t) < 3 {
            feed(&mut t, b"\r\n");
        }
        for _ in 0..t.screen().pages.rows {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"hello.");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();
        assert_eq!(0, search.active_results.len());

        let matches = search.matches();
        assert_eq!(1, matches.len());
        expect(&t, &matches[0], Tag::Screen, (0, 0), (3, 0));
        search.deinit(t.screen_mut());
    }

    #[test]
    fn reload_active_with_history_change() {
        let mut t = terminal_scrollback(10, 2, usize::MAX);
        feed(&mut t, b"Fizz\r\n");

        // Start up our search which will populate our initial active area.
        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();
        assert_eq!(1, search.matches().len());

        // Grow into two pages so our history pin will move.
        while total_pages(&t) < 2 {
            feed(&mut t, b"\r\n");
        }
        for _ in 0..t.screen().pages.rows {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"2Fizz");

        // Active area changed so reload
        search.reload_active(t.screen_mut()).unwrap();
        search.search_all(t.screen_mut()).unwrap();

        {
            let matches = search.matches();
            assert_eq!(2, matches.len());
            expect(&t, &matches[1], Tag::Screen, (0, 0), (3, 0));
            expect(&t, &matches[0], Tag::Active, (1, 1), (4, 1));
        }

        // Reset the screen which will make our pin garbage.
        t.full_reset();
        feed(&mut t, b"WeFizzing");
        search.reload_active(t.screen_mut()).unwrap();
        search.search_all(t.screen_mut()).unwrap();

        {
            let matches = search.matches();
            assert_eq!(1, matches.len());
            expect(&t, &matches[0], Tag::Active, (2, 0), (5, 0));
        }
        search.deinit(t.screen_mut());
    }

    #[test]
    fn active_change_contents() {
        let mut t = terminal(10, 5);
        feed(&mut t, b"Fuzz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();
        assert_eq!(1, search.active_results.len());

        // Erase the screen, move our cursor to the top, and change contents.
        feed(&mut t, b"\x1b[2J\x1b[H"); // Clear screen and move home
        feed(&mut t, b"Bang\r\nFizz\r\nHello!");

        search.reload_active(t.screen_mut()).unwrap();
        search.search_all(t.screen_mut()).unwrap();
        assert_eq!(1, search.active_results.len());

        let matches = search.matches();
        assert_eq!(1, matches.len());
        expect(&t, &matches[0], Tag::Screen, (0, 1), (3, 1));
        search.deinit(t.screen_mut());
    }

    #[test]
    fn select_next() {
        let mut t = terminal(10, 2);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();

        // Initially no selection
        assert!(search.selected_match().is_none());

        // Select our next match (first)
        search.search_all(t.screen_mut()).unwrap();
        search.select(t.screen_mut(), Select::Next).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 2),
            (3, 2),
        );

        // Next match
        search.select(t.screen_mut(), Select::Next).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );

        // Next match (no wrap)
        search.select(t.screen_mut(), Select::Next).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );
        search.deinit(t.screen_mut());
    }

    #[test]
    fn select_in_active_changes_contents_completely() {
        let mut t = terminal(10, 5);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();
        search.select(t.screen_mut(), Select::Next).unwrap();
        search.select(t.screen_mut(), Select::Next).unwrap();
        // Initial selection is the first fizz
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );

        // Erase the screen, move our cursor to the top, and change contents.
        feed(&mut t, b"\x1b[2J\x1b[H"); // Clear screen and move home
        feed(&mut t, b"Fuzz\r\nFizz\r\nHello!");

        search.reload_active(t.screen_mut()).unwrap();
        // Our selection should move to the first
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 1),
            (3, 1),
        );

        // Erase the screen, redraw with same contents.
        feed(&mut t, b"\x1b[2J\x1b[H"); // Clear screen and move home
        feed(&mut t, b"Fuzz\r\nFizz\r\nFizz");

        search.reload_active(t.screen_mut()).unwrap();
        // Our selection should not move to the first
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 1),
            (3, 1),
        );
        search.deinit(t.screen_mut());
    }

    #[test]
    fn select_into_history() {
        let mut t = terminal_scrollback(10, 2, usize::MAX);
        feed(&mut t, b"Fizz\r\n");
        while total_pages(&t) < 3 {
            feed(&mut t, b"\r\n");
        }
        for _ in 0..t.screen().pages.rows {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"hello.");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();

        // Get all matches
        search.select(t.screen_mut(), Select::Next).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );

        // Erase the screen, redraw with same contents.
        feed(&mut t, b"\x1b[2J\x1b[H"); // Clear screen and move home
        feed(&mut t, b"yo yo");

        search.reload_active(t.screen_mut()).unwrap();
        // Our selection should not move since the history is still active.
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );

        // Create some new history by adding more lines.
        feed(&mut t, b"\r\nfizz\r\nfizz\r\nfizz");
        search.reload_active(t.screen_mut()).unwrap();
        // Our selection should not move since the history is still not
        // pruned.
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );
        search.deinit(t.screen_mut());
    }

    #[test]
    fn select_prev() {
        let mut t = terminal(10, 2);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();

        // Initially no selection
        assert!(search.selected_match().is_none());

        // Select prev (oldest first)
        search.search_all(t.screen_mut()).unwrap();
        search.select(t.screen_mut(), Select::Prev).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );

        // Prev match (towards newest)
        search.select(t.screen_mut(), Select::Prev).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 2),
            (3, 2),
        );

        // Prev match (no wrap, stays at newest)
        search.select(t.screen_mut(), Select::Prev).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 2),
            (3, 2),
        );
        search.deinit(t.screen_mut());
    }

    #[test]
    fn select_prev_then_next() {
        let mut t = terminal(10, 2);
        feed(&mut t, b"Fizz\r\nBuzz\r\nFizz\r\nBang");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();

        // Select next (newest first)
        search.select(t.screen_mut(), Select::Next).unwrap();
        expect_start(&t, &search.selected_match().unwrap(), Tag::Screen, (0, 2));

        // Select next (older)
        search.select(t.screen_mut(), Select::Next).unwrap();
        expect_start(&t, &search.selected_match().unwrap(), Tag::Screen, (0, 0));

        // Select prev (back to newer)
        search.select(t.screen_mut(), Select::Prev).unwrap();
        expect_start(&t, &search.selected_match().unwrap(), Tag::Screen, (0, 2));
        search.deinit(t.screen_mut());
    }

    #[test]
    fn select_prev_with_history() {
        let mut t = terminal_scrollback(10, 2, usize::MAX);
        feed(&mut t, b"Fizz\r\n");
        while total_pages(&t) < 3 {
            feed(&mut t, b"\r\n");
        }
        for _ in 0..t.screen().pages.rows {
            feed(&mut t, b"\r\n");
        }
        feed(&mut t, b"Fizz.");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();

        // Select prev (oldest first, should be in history)
        search.select(t.screen_mut(), Select::Prev).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Screen,
            (0, 0),
            (3, 0),
        );

        // Select prev (towards newer, should move to active area)
        search.select(t.screen_mut(), Select::Prev).unwrap();
        expect(
            &t,
            &search.selected_match().unwrap(),
            Tag::Active,
            (0, 1),
            (3, 1),
        );
        search.deinit(t.screen_mut());
    }

    #[test]
    fn screen_search_no_scrollback_has_no_history() {
        let mut t = terminal_scrollback(10, 2, 0);

        // Alt screen has no scrollback
        t.switch_screen(crate::screen_set::Key::Alternate).unwrap();

        // This will probably stop working at some point and we'll have
        // no way to test it using public APIs, but at the time of writing
        // this test, CSI 22 J (scroll complete) pushes into scrollback
        // with alt screen.
        feed(&mut t, b"Fizz\r\n");
        feed(&mut t, b"\x1b[22J");
        feed(&mut t, b"hello.");

        let mut search = ScreenSearch::new(t.screen_mut(), b"Fizz").unwrap();
        search.search_all(t.screen_mut()).unwrap();
        assert_eq!(0, search.active_results.len());
        assert_eq!(0, search.matches().len());
        search.deinit(t.screen_mut());
    }
}
