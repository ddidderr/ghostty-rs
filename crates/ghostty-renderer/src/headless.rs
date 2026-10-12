//! Headless GL context (surfaceless EGL, Mesa) for tests and tools that need a
//! current OpenGL 4.3 core context without a window system.
//!
//! This is a test-support API (not used by the application): integration tests
//! of other crates (e.g. `ghostty-core/tests/e2e_*.rs`) use it to build a real
//! [`Renderer`](crate::Renderer) without GTK. The context is made current on
//! the thread that calls [`Headless::new`] and must be dropped on that thread.

#![allow(unsafe_code)]

use std::ffi::c_void;

use khronos_egl as egl;

// EGL_PLATFORM_SURFACELESS_MESA, falling back to the default display.
const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

/// A surfaceless EGL context, current on the creating thread.
pub struct Headless {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
}

impl Headless {
    /// Create a GL 4.3 core context and make it current on this thread.
    pub fn new() -> Result<Headless, String> {
        // SAFETY: loads the system libEGL.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|e| format!("cannot load libEGL: {e}"))?;

        // SAFETY: EGL_DEFAULT_DISPLAY with an empty (terminated) attribute list.
        let display = unsafe {
            egl.get_platform_display(
                PLATFORM_SURFACELESS_MESA,
                egl::DEFAULT_DISPLAY,
                &[egl::ATTRIB_NONE],
            )
        }
        .or_else(|_| {
            // SAFETY: default display handle.
            unsafe { egl.get_display(egl::DEFAULT_DISPLAY) }.ok_or(egl::Error::BadDisplay)
        })
        .map_err(|e| format!("no EGL display: {e}"))?;
        egl.initialize(display)
            .map_err(|e| format!("eglInitialize: {e}"))?;
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| format!("eglBindAPI: {e}"))?;

        let attribs = [
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::SURFACE_TYPE,
            egl::PBUFFER_BIT,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attribs)
            .map_err(|e| format!("eglChooseConfig: {e}"))?
            .ok_or("no EGL config")?;
        let ctx_attribs = [
            egl::CONTEXT_MAJOR_VERSION,
            4,
            egl::CONTEXT_MINOR_VERSION,
            3,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let context = egl
            .create_context(display, config, None, &ctx_attribs)
            .map_err(|e| format!("eglCreateContext (GL 4.3 core): {e}"))?;
        // Surfaceless: EGL_KHR_surfaceless_context.
        egl.make_current(display, None, None, Some(context))
            .map_err(|e| format!("eglMakeCurrent: {e}"))?;
        Ok(Headless {
            egl,
            display,
            context,
        })
    }

    /// `glViewport(0, 0, width, height)`. The renderer reads the viewport to
    /// learn the surface size (GTK's `GLArea` sets it before every render), so
    /// headless callers must set it to the surface size.
    pub fn set_viewport(&self, width: i32, height: i32) {
        let p = self.proc_address("glViewport");
        if p.is_null() {
            return;
        }
        // SAFETY: `glViewport` has the C signature `void(GLint, GLint, GLsizei,
        // GLsizei)`; the context is current on this thread.
        unsafe {
            let f: extern "C" fn(i32, i32, i32, i32) = std::mem::transmute(p);
            f(0, 0, width, height);
        }
    }

    /// Resolve a GL function pointer (null if unknown).
    #[must_use]
    pub fn proc_address(&self, name: &str) -> *const c_void {
        self.egl
            .get_proc_address(name)
            .map_or(std::ptr::null(), |f| f as *const c_void)
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        // The display is shared by every context of the process (tests run in
        // parallel threads), so it is deliberately not terminated.
    }
}
