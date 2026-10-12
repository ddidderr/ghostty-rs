//! OpenGL function loader for the `GLArea`.
//!
//! GTK uses libepoxy, which exports a lazily-resolving function pointer for
//! every GL entry point (resolved against whichever context, EGL or GLX, is
//! current). So reading those pointers gives us a loader that works on both Wayland and
//! X11, which is what `ghostty-renderer`'s `prepare_context(|name| ..)` and
//! `glow::Context::from_loader_function` need. Call it only while the GLArea
//! context is current.

use std::{ffi::c_void, sync::OnceLock};

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
///
/// libepoxy does not export `glFoo` symbols; it exports a function pointer
/// *variable* `epoxy_glFoo` per entry point. The variable initially points
/// at a resolver stub which, on first call, resolves the real function for
/// the current context (EGL/GLX) and rewrites the variable. Reading the
/// variable therefore yields a callable function pointer.
#[must_use]
pub fn proc_address(name: &str) -> *const c_void {
    let Some(lib) = epoxy() else {
        return std::ptr::null();
    };
    let symbol = format!("epoxy_{name}");
    // SAFETY: we read the pointer value stored in the exported variable; the
    // library is kept alive for the whole process lifetime by the `static`
    // above, so the returned pointer stays valid. Callers cast it to the
    // correct GL signature.
    #[allow(unsafe_code)]
    unsafe {
        lib.get::<*const *const c_void>(symbol.as_bytes())
            .map_or(std::ptr::null(), |sym| **sym)
    }
}
