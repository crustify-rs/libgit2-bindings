// A setter stores a borrow for the update's whole lifetime: a shorter-lived path
// cannot be stored in an update that is used after the path is gone.
use libgit2::libgit2::object::tree::TreeUpdate;
use std::ffi::CString;

fn main() {
    let mut update = TreeUpdate::remove(c"README");
    {
        let path = CString::new("short-lived").unwrap();
        update.as_mut().set_path(&path);
    }
    let _ = update.as_ref().path();
}
