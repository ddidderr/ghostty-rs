//! A wrapper around a CALayer with a utility method for settings its
//! `contents` to an IOSurface (`metal/IOSurfaceLayer.zig`).
use std::sync::{Arc, Mutex, PoisonError};

use dispatch2::DispatchQueue;
use objc2::{
    AnyThread,
    DefinedClass,
    MainThreadMarker,
    define_class,
    msg_send,
    rc::Retained,
    runtime::AnyObject,
};
use objc2_core_foundation::CFRetained;
use objc2_foundation::{NSNull, NSString};
use objc2_io_surface::IOSurfaceRef;
use objc2_quartz_core::{CALayer, kCAGravityTopLeft};

use super::Error;

/// The callback invoked when the layer needs to `display`.
pub type DisplayCallback = Arc<dyn Fn() + Send + Sync>;

/// Instance variables of the layer subclass (`display_cb` / `display_ctx`).
#[derive(Default)]
pub struct Ivars {
    display_cb: Mutex<Option<DisplayCallback>>,
}

define_class!(
    /// We subclass CALayer with a custom display handler.
    // SAFETY:
    // - The superclass CALayer has no subclassing requirements we violate.
    // - The class does not implement `Drop`.
    #[unsafe(super(CALayer))]
    #[name = "IOSurfaceLayer"]
    #[ivars = Ivars]
    pub struct IOSurfaceLayerClass;

    impl IOSurfaceLayerClass {
        #[unsafe(method(display))]
        fn display(&self) {
            // Clone the callback out so it isn't invoked with the lock held.
            let cb = self
                .ivars()
                .display_cb
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(cb) = cb {
                cb();
            }
        }

        /// Disable all animations for this layer by returning null for all
        /// actions.
        #[unsafe(method_id(actionForKey:))]
        fn action_for_key(&self, _key: &NSString) -> Option<Retained<NSNull>> {
            Some(NSNull::null())
        }
    }
);

impl IOSurfaceLayerClass {
    fn create() -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars::default());
        // SAFETY: `init` is the designated initializer of the superclass.
        unsafe { msg_send![super(this), init] }
    }
}

/// Wrapper to move an Objective-C / CoreFoundation object to another thread.
struct SendWrapper<T>(T);
// SAFETY: used only for the layer and IOSurface, which are reference counted
// thread-safe objects when messaged on the main thread (we only message the
// layer on the main queue).
unsafe impl<T> Send for SendWrapper<T> {}

/// A CALayer which displays IOSurfaces.
#[derive(Clone)]
pub struct IOSurfaceLayer {
    /// The underlying CALayer
    layer: Retained<IOSurfaceLayerClass>,
}

// SAFETY: the layer is only messaged from the render / main threads through
// thread-safe CALayer properties (`contents`, `bounds`, `contentsScale`), the
// same way Ghostty (Zig) shares it between the renderer and the main thread.
unsafe impl Send for IOSurfaceLayer {}
// SAFETY: see above.
unsafe impl Sync for IOSurfaceLayer {}

impl IOSurfaceLayer {
    pub fn new() -> Result<IOSurfaceLayer, Error> {
        let layer = IOSurfaceLayerClass::create();

        // The layer gravity is set to top-left so that the contents aren't
        // stretched during resize operations before a new frame has been drawn.
        //
        // SAFETY: a constant NSString exported by QuartzCore.
        layer.setContentsGravity(unsafe { kCAGravityTopLeft });

        Ok(IOSurfaceLayer { layer })
    }

    /// The underlying CALayer.
    pub fn layer(&self) -> &CALayer {
        &self.layer
    }

    /// The layer's bounds size and contents scale.
    pub fn bounds_and_scale(&self) -> (f64, f64, f64) {
        let bounds = self.layer.bounds();
        (
            bounds.size.width,
            bounds.size.height,
            self.layer.contentsScale(),
        )
    }

    /// Sets the layer's `contents` to the provided IOSurface.
    ///
    /// Makes sure to do so on the main thread to avoid visual artifacts.
    pub fn set_surface(&self, surface: &CFRetained<IOSurfaceRef>) {
        // We retain the surface (and the layer) to make sure they're not
        // released before we can set it as the contents of the layer; both
        // are released when the closure is dropped.
        let layer = SendWrapper(self.layer.clone());
        let surface = SendWrapper(surface.clone());

        // We check if we're on the main thread and run the work directly if so.
        if MainThreadMarker::new().is_some() {
            set_surface_callback(&layer.0, &surface.0);
        } else {
            DispatchQueue::main().exec_async(move || {
                let layer = layer;
                let surface = surface;
                set_surface_callback(&layer.0, &surface.0);
            });
        }
    }

    /// Sets the layer's `contents` to the provided IOSurface.
    ///
    /// Does not ensure this happens on the main thread.
    pub fn set_surface_sync(&self, surface: &CFRetained<IOSurfaceRef>) {
        let contents: &AnyObject = surface_as_object(surface);
        // SAFETY: an IOSurface is a valid value for `contents`.
        unsafe { self.layer.setContents(Some(contents)) };
    }

    /// Set (or clear) the callback invoked from the layer's `display` method.
    pub fn set_display_callback(&self, cb: Option<DisplayCallback>) {
        *self
            .layer
            .ivars()
            .display_cb
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = cb;
    }
}

/// View an IOSurface as an Objective-C object (they are toll-free bridged).
fn surface_as_object(surface: &CFRetained<IOSurfaceRef>) -> &AnyObject {
    let cf: &objc2_core_foundation::CFType = surface;
    cf.as_ref()
}

fn set_surface_callback(layer: &IOSurfaceLayerClass, surface: &CFRetained<IOSurfaceRef>) {
    // We check to see if the surface is the appropriate size for
    // the layer, if it's not then we discard it. This is because
    // asynchronously drawn frames can sometimes finish just after
    // a synchronously drawn frame during a resize, and if we don't
    // discard the improperly sized surface it creates jank.
    let bounds = layer.bounds();
    let scale = layer.contentsScale();
    let width = (bounds.size.width * scale) as usize;
    let height = (bounds.size.height * scale) as usize;
    if width != surface.width() || height != surface.height() {
        log::debug!(
            "setSurfaceCallback(): surface is wrong size for layer, discarding. surface = {}x{}, layer = {}x{}",
            surface.width(),
            surface.height(),
            width,
            height,
        );
        return;
    }

    // SAFETY: an IOSurface is a valid value for `contents`.
    unsafe { layer.setContents(Some(surface_as_object(surface))) };
}
