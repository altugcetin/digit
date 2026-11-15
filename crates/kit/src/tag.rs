//! Values kept on a GObject under a name, without `unsafe` at the call site.
//!
//! GLib's object data is untyped: reading a key as a type other than the one
//! stored is undefined behaviour. Every value here goes in as one wrapper type
//! under a namespaced key, and comes out through `Any`, so a mismatch is `None`.

use gtk::glib::{self, prelude::*};
use std::any::Any;

struct Tag(Box<dyn Any>);

fn key(name: &str) -> String {
    format!("digit-tag:{name}")
}

/// Stores `value` on `object` under `name`, dropping what was there.
pub fn set<T: 'static>(object: &impl IsA<glib::Object>, name: &str, value: T) {
    // SAFETY: keys under "digit-tag:" are only ever stored and read as `Tag`.
    unsafe { object.as_ref().set_data(&key(name), Tag(Box::new(value))) }
}

/// A copy of the value under `name`, if one of type `T` is there.
pub fn get<T: Clone + 'static>(object: &impl IsA<glib::Object>, name: &str) -> Option<T> {
    // SAFETY: as in `set`; the pointer is used before anything can replace it.
    unsafe { object.as_ref().data::<Tag>(&key(name)).and_then(|p| p.as_ref().0.downcast_ref::<T>().cloned()) }
}

/// Whether anything is stored under `name`.
pub fn has(object: &impl IsA<glib::Object>, name: &str) -> bool {
    // SAFETY: as in `set`.
    unsafe { object.as_ref().data::<Tag>(&key(name)).is_some() }
}

/// Removes and drops the value under `name`.
pub fn clear(object: &impl IsA<glib::Object>, name: &str) {
    // SAFETY: as in `set`.
    drop(unsafe { object.as_ref().steal_data::<Tag>(&key(name)) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_come_back_only_as_their_own_type() {
        let o = glib::Object::new::<glib::Object>();
        assert!(!has(&o, "n"));
        set(&o, "n", 7u32);
        assert!(has(&o, "n"));
        assert_eq!(get::<u32>(&o, "n"), Some(7));
        assert_eq!(get::<i64>(&o, "n"), None);
        clear(&o, "n");
        assert!(!has(&o, "n"));
    }
}
