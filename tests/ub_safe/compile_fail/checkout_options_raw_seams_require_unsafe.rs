// The safe setters take Rust callbacks only; a C callback is stored through the raw seam,
// whose pointer only an `unsafe` write goes through.
use ffibox::CMutSeam;
use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;

fn main() {
    let mut options = CheckoutOptions::new();
    let mut m = options.as_mut();
    let p = m.as_mut_ptr();
    (*p).notify_cb = None;
    (*p).progress_payload = std::ptr::null_mut();
}
