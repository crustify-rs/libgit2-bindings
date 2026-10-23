// A view borrows its strings, and cannot outlive them.
use std::ffi::CString;

use libgit2::libgit2::core::strarray::StrArrayView;

fn main() {
    let view = {
        let owned = vec![CString::new("x").unwrap()];
        StrArrayView::new(&owned)
    };
    let _ = view.as_ref().len();
}
