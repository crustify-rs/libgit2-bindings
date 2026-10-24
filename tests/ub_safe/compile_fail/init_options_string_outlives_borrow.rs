// The init options cannot outlive a string their setter borrowed.
use std::ffi::CString;

use libgit2::libgit2::src::libgit2::repository::InitOptions;

fn main() {
    let mut options = InitOptions::new();
    {
        let url = CString::new("https://example.com/r.git").unwrap();
        options.as_mut().set_origin_url(Some(&url));
    }
    let _ = options.as_ref().origin_url();
}
