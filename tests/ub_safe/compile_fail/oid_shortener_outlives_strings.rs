// A shortener cannot outlive a string its exclusive handle was given: the handle
// (what `git_oid_shorten_add` takes) is invariant in the strings' lifetime.
use std::ffi::{CStr, CString};

use {libgit2::libgit2::src::libgit2::oid::OidShortener, libgit2::libgit2::src::libgit2::oid::OidShortenerMut};

fn keep<'s>(_: OidShortenerMut<'_, 's>, _: &'s CStr) {}

fn main() {
    let shortener = {
        let text = CString::new("a94a8fe5ccb19ba61c4c0873d391e987982fbbd3").unwrap();
        let mut shortener = OidShortener::new(0).unwrap();
        keep(shortener.as_mut(), &text);
        shortener
    };
    drop(shortener);
}
