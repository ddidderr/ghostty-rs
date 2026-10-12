//! Oniguruma `MatchParam` extensions missing from the `onig` crate.
//!
//! Ghostty's `StringMap` bounds regex work with
//! `MatchParam.setRetryLimitInSearch` (a *whole-search* retry budget); the
//! `onig` crate only wraps the per-match limit, so the search limit is set
//! here through `onig_sys`.

use onig::MatchParam;

/// Set the retry limit in search (Oniguruma's aggregate backtracking-work
/// budget across the entire search, as opposed to per match attempt).
pub fn set_retry_limit_in_search(param: &mut MatchParam, limit: u32) {
    #[allow(unsafe_code)]
    // SAFETY: `param.as_raw()` is the live, non-null `OnigMatchParam*` owned
    // by `param` (allocated by `onig_new_match_param` in
    // `MatchParam::default`) and we hold `&mut` so nothing else uses it for
    // the duration of the call; the function only writes an integer field.
    unsafe {
        onig_sys::onig_set_retry_limit_in_search_of_match_param(
            param.as_raw(),
            std::os::raw::c_ulong::from(limit),
        );
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use onig::{Regex, SearchOptions};

    use super::*;

    fn search(re: &Regex, text: &str, limit: Option<u32>) -> Result<Option<usize>, onig::Error> {
        let mut param = MatchParam::default();
        if let Some(l) = limit {
            set_retry_limit_in_search(&mut param, l);
        }
        re.search_with_param(
            text,
            0,
            text.len(),
            SearchOptions::SEARCH_OPTION_NONE,
            None,
            param,
        )
    }

    #[test]
    fn small_limit_aborts_catastrophic_search() {
        // Classic exponential backtracking pattern.
        let re = Regex::new("(a+)+$").unwrap();
        let text = format!("{}b", "a".repeat(40));
        // With a tiny whole-search budget the search errors out.
        assert!(search(&re, &text, Some(1000)).is_err());
    }

    #[test]
    fn generous_limit_allows_normal_search() {
        let re = Regex::new("b+").unwrap();
        assert_eq!(search(&re, "aaabbb", Some(100_000)).unwrap(), Some(3));
    }
}
