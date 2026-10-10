// The raw seams store pointers the borrow checker cannot follow: each is `unsafe`.
use libgit2::libgit2::workdir::checkout::CheckoutOptions;

fn main() {
    let mut options = CheckoutOptions::new();
    let mut m = options.as_mut();
    m.set_baseline(std::ptr::null_mut());
    m.set_notify_callback(None, std::ptr::null_mut());
    m.set_progress_callback(None, std::ptr::null_mut());
    m.set_perfdata_callback(None, std::ptr::null_mut());
}
