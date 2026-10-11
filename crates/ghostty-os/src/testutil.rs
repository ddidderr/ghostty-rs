//! Test helpers. Process environment mutation is `unsafe` in Rust 2024, so
//! tests go through `temp_env`, which serializes env access with a global
//! lock. Tests that merely *read* env vars that other tests mutate should
//! take the same lock via [`with_env_lock`].

/// Run `f` while holding the global env lock (no variables changed).
pub fn with_env_lock<R>(f: impl FnOnce() -> R) -> R {
    temp_env::with_vars(Vec::<(&str, Option<&str>)>::new(), f)
}
