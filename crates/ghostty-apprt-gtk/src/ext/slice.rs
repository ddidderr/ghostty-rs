//! Port of `apprt/gtk/ext/slice.zig`: a boxed list of strings.

use gtk::glib;

/// A boxed list of strings (GType `GhosttyStringList`). Copy/free semantics
/// are provided by `Clone`/`Drop`.
#[derive(Debug, Clone, Default, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "GhosttyStringList")]
pub struct StringList {
    pub strings: Vec<String>,
}

impl StringList {
    pub fn create<S: AsRef<str>>(strings: &[S]) -> Self {
        Self {
            strings: strings.iter().map(|s| s.as_ref().to_owned()).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::glib::prelude::*;

    #[test]
    fn string_list_create_and_destroy() {
        let list = StringList::create(&["hello", "world"]);
        assert_eq!(list.strings.len(), 2);
        assert_eq!(list.strings[0], "hello");
        assert_eq!(list.strings[1], "world");
    }

    #[test]
    fn string_list_create_empty_list() {
        let list = StringList::create::<&str>(&[]);
        assert_eq!(list.strings.len(), 0);
    }

    #[test]
    fn string_list_boxed_copy_and_free() {
        let original = StringList::create(&["foo", "bar", "baz"]);
        let value = original.to_value();
        let copied = value.get::<StringList>().expect("boxed value");
        assert_eq!(copied.strings, ["foo", "bar", "baz"]);
        assert_ne!(original.strings.as_ptr(), copied.strings.as_ptr());
    }
}
