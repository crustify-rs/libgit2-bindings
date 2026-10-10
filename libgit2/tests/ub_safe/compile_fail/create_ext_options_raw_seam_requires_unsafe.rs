// The signing callback's raw seam stores a function and payload the borrow checker
// cannot follow: it is `unsafe`.
use libgit2::libgit2::object::commit::CreateExtOptions;

fn main() {
    let mut options = CreateExtOptions::new();
    options.as_mut().set_sign_callback(None, std::ptr::null_mut());
}
