// The options cannot outlive a string their setter borrowed.
use std::ffi::CString;

use libgit2::libgit2::workdir::checkout::CheckoutOptions;

fn main() {
    let mut options = CheckoutOptions::new();
    {
        let label = CString::new("mine").unwrap();
        options.as_mut().set_our_label(Some(&label));
    }
    let _ = options.as_ref().our_label();
}
