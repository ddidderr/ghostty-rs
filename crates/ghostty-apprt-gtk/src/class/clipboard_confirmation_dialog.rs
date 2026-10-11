//! Port of apprt/gtk/class/clipboard_confirmation_dialog.zig
//! (`GhosttyClipboardConfirmationDialog`).
//!
//! Properties: `can-remember`, `request` (boxed [`ClipboardRequestBoxed`]),
//! `clipboard-contents` (GtkTextBuffer), `blur`. Signals `confirm(bool)` and
//! `deny(bool)` (bool = "remember choice").

use adw::prelude::*;
use adw::subclass::prelude::*;
use ghostty_apprt::structs::ClipboardRequest;
use ghostty_os::i18n::tr;
use gtk::glib;
use gtk::glib::subclass::Signal;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

use super::dialog::{Dialog, DialogImpl};

/// GBoxed wrapper for [`ClipboardRequest`] (Zig: boxed `apprt.ClipboardRequest`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "GhosttyClipboardRequest", nullable)]
pub struct ClipboardRequestBoxed(pub ClipboardRequest);

mod imp {
    use super::{
        AdwAlertDialogImpl, AdwDialogImpl, Cell, ClipboardRequest, ClipboardRequestBoxed, Dialog,
        DialogImpl, ObjectImpl, ObjectSubclass, OnceLock, RefCell, Signal, WidgetImpl, tr,
    };
    use adw::prelude::*;
    use gtk::glib;
    use gtk::subclass::prelude::*;

    #[derive(Default, glib::Properties, gtk::CompositeTemplate)]
    #[properties(wrapper_type = super::ClipboardConfirmationDialog)]
    #[template(resource = "/com/mitchellh/ghostty/ui/1.4/clipboard-confirmation-dialog.ui")]
    pub struct ClipboardConfirmationDialog {
        /// The request that this dialog is for.
        #[property(get, set, nullable)]
        pub request: RefCell<Option<ClipboardRequestBoxed>>,

        /// The clipboard contents being read/written.
        #[property(get, set, nullable, name = "clipboard-contents")]
        pub clipboard_contents: RefCell<Option<gtk::TextBuffer>>,

        /// Whether the contents should be blurred.
        #[property(get, set)]
        pub blur: Cell<bool>,

        /// Whether the user can remember the choice.
        #[property(get, set, name = "can-remember")]
        pub can_remember: Cell<bool>,

        #[template_child]
        pub text_view_scroll: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub text_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub reveal_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub hide_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub remember_choice: TemplateChild<adw::SwitchRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ClipboardConfirmationDialog {
        const NAME: &'static str = "GhosttyClipboardConfirmationDialog";
        type Type = super::ClipboardConfirmationDialog;
        type ParentType = Dialog;

        fn class_init(klass: &mut Self::Class) {
            Dialog::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl ClipboardConfirmationDialog {
        fn set_blurred(&self, blurred: bool, reveal_visible: bool, hide_visible: bool) {
            self.text_view_scroll.set_sensitive(!blurred);
            if blurred {
                self.text_view.add_css_class("blurred");
            } else {
                self.text_view.remove_css_class("blurred");
            }
            self.reveal_button.set_visible(reveal_visible);
            self.hide_button.set_visible(hide_visible);
        }

        #[template_callback]
        fn notify_blur(&self, _pspec: &glib::ParamSpec) {
            if self.blur.get() {
                self.set_blurred(true, true, false);
            } else {
                self.set_blurred(false, false, false);
            }
        }

        #[template_callback]
        fn notify_request(&self, _pspec: &glib::ParamSpec) {
            self.update_request();
        }

        pub fn update_request(&self) {
            let Some(ClipboardRequestBoxed(req)) = *self.request.borrow() else {
                return;
            };
            let obj = self.obj();
            match req {
                ClipboardRequest::Osc52Write(_) => {
                    obj.set_heading(Some(&tr("Authorize Clipboard Access")));
                    obj.set_body(&tr("An application is attempting to write to the clipboard. The current clipboard contents are shown below."));
                }
                ClipboardRequest::Osc52Read(_) => {
                    obj.set_heading(Some(&tr("Authorize Clipboard Access")));
                    obj.set_body(&tr("An application is attempting to read from the clipboard. The current clipboard contents are shown below."));
                }
                ClipboardRequest::Paste => {
                    obj.set_heading(Some(&tr("Warning: Potentially Unsafe Paste")));
                    obj.set_body(&tr("Pasting this text into the terminal may be dangerous as it looks like some commands may be executed."));
                }
            }
        }

        #[template_callback]
        fn reveal_clicked(&self, _button: &gtk::Button) {
            self.set_blurred(false, false, true);
        }

        #[template_callback]
        fn hide_clicked(&self, _button: &gtk::Button) {
            self.set_blurred(true, true, false);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ClipboardConfirmationDialog {
        fn constructed(&self) {
            self.parent_constructed();
            // Mirrors `self.propBlur(..); self.propRequest(..)` in init.
            self.notify_blur(&Self::type_pspec_dummy());
            self.update_request();
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder("deny")
                        .param_types([bool::static_type()])
                        .build(),
                    Signal::builder("confirm")
                        .param_types([bool::static_type()])
                        .build(),
                ]
            })
        }

        fn dispose(&self) {
            self.clipboard_contents.replace(None);
            self.dispose_template();
        }
    }

    impl ClipboardConfirmationDialog {
        fn type_pspec_dummy() -> glib::ParamSpec {
            glib::ParamSpecBoolean::builder("blur").build()
        }
    }

    impl WidgetImpl for ClipboardConfirmationDialog {}
    impl AdwDialogImpl for ClipboardConfirmationDialog {}
    impl AdwAlertDialogImpl for ClipboardConfirmationDialog {
        fn response(&self, response: &str) {
            let remember = self.remember_choice.is_active();
            let obj = self.obj();
            if response == "cancel" {
                obj.emit_by_name::<()>("deny", &[&remember]);
            } else if response == "ok" {
                obj.emit_by_name::<()>("confirm", &[&remember]);
            }
        }
    }
    impl DialogImpl for ClipboardConfirmationDialog {}
}

glib::wrapper! {
    pub struct ClipboardConfirmationDialog(ObjectSubclass<imp::ClipboardConfirmationDialog>)
        @extends Dialog, adw::AlertDialog, adw::Dialog, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl ClipboardConfirmationDialog {
    pub fn new() -> Self {
        crate::register_resources();
        glib::Object::new()
    }

    pub fn present(&self, parent: Option<&impl IsA<gtk::Widget>>) {
        self.upcast_ref::<Dialog>().present(parent);
    }

    /// Get the clipboard request (`getRequest`).
    pub fn get_request(&self) -> Option<ClipboardRequest> {
        self.imp().request.borrow().map(|b| b.0)
    }

    /// Get the clipboard contents (`getClipboardContents`).
    pub fn get_clipboard_contents(&self) -> Option<gtk::TextBuffer> {
        self.imp().clipboard_contents.borrow().clone()
    }

    pub fn set_request_value(&self, req: ClipboardRequest) {
        self.set_request(Some(ClipboardRequestBoxed(req)));
    }

    /// `confirm(remember)`.
    pub fn connect_confirm<F: Fn(&Self, bool) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "confirm",
            false,
            glib::closure_local!(move |obj: &Self, remember: bool| f(obj, remember)),
        )
    }

    /// `deny(remember)`.
    pub fn connect_deny<F: Fn(&Self, bool) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "deny",
            false,
            glib::closure_local!(move |obj: &Self, remember: bool| f(obj, remember)),
        )
    }
}

impl Default for ClipboardConfirmationDialog {
    fn default() -> Self {
        Self::new()
    }
}
