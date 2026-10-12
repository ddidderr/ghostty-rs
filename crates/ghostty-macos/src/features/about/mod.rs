//! `Sources/Features/About` (owner: MA4).
//! MA1 provides the `AboutController` surface `AppDelegate` calls.
//! TODO(MA4): AboutController.swift/About.xib/AboutView/CyclingIconView.

/// `AboutController.shared`.
#[derive(Default)]
pub struct AboutController;

thread_local! {
    static SHARED: std::rc::Rc<AboutController> = std::rc::Rc::default();
}

impl AboutController {
    #[must_use]
    pub fn shared() -> std::rc::Rc<Self> {
        SHARED.with(std::rc::Rc::clone)
    }

    /// `show()`. TODO(MA4)
    pub fn show(&self) {
        log::warn!("TODO(MA4): AboutController.show");
    }

    /// `hide()`. TODO(MA4)
    pub fn hide(&self) {}
}
