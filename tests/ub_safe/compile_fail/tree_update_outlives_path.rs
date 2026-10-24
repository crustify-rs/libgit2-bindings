// An update borrows its path: it cannot be read after the path is dropped.
use libgit2::libgit2::src::libgit2::tree::TreeUpdate;
use std::ffi::CString;

fn main() {
    let path = CString::new("dir/file.txt").unwrap();
    let update = TreeUpdate::remove(&path);
    drop(path);
    let _ = update.as_ref().path();
}
