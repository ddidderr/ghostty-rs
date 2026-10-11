//! OpenGL function loader for the `GLArea`.
//!
//! GTK uses libepoxy, which exports dispatching stubs for every GL entry
//! point and resolves them against whichever context (EGL or GLX) is current.
//! So `dlsym` on libepoxy gives us a loader that works on both Wayland and
//! X11, which is what `ghostty-renderer`'s `prepare_context(|name| ..)` and
//! `glow::Context::from_loader_function` need. Call it only while the GLArea
//! context is current.

use std::ffi::c_void;
use std::sync::OnceLock;

fn epoxy() -> Option<&'static libloading::Library> {
    static LIB: OnceLock<Option<libloading::Library>> = OnceLock::new();
    LIB.get_or_init(|| {
        for name in ["libepoxy.so.0", "libepoxy.so"] {
            // SAFETY: loading libepoxy runs only its trivial constructors;
            // GTK itself already links it into this process.
            #[allow(unsafe_code)]
            if let Ok(lib) = unsafe { libloading::Library::new(name) } {
                return Some(lib);
            }
        }
        log::error!("unable to load libepoxy; OpenGL functions are unavailable");
        None
    })
    .as_ref()
}

/// Resolve an OpenGL function by name (null if unavailable).
#[must_use]
pub fn proc_address(name: &str) -> *const c_void {
    let Some(lib) = epoxy() else {
        return std::ptr::null();
    };
    // SAFETY: we only read the symbol's address; the library is kept alive
    // for the whole process lifetime by the `static` above, so the returned
    // pointer stays valid. Callers cast it to the correct GL signature.
    #[allow(unsafe_code)]
    unsafe {
        lib.get::<*const c_void>(name.as_bytes())
            .map_or(std::ptr::null(), |sym| *sym)
    }
}
