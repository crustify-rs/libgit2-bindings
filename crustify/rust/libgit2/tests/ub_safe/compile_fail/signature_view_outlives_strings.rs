// A view borrows its strings, so it cannot outlive them.
use std::ffi::CString;

use libgit2::libgit2::core::types::{SignatureView, Time};

fn main() {
    let view = {
        let name = CString::new("A").unwrap();
        SignatureView::new(&name, c"a@b", Time::new(0, 0))
    };
    let _ = view.as_ref().name();
}
