// A header borrows its strings: it cannot be read after one is dropped.
use libgit2::libgit2::object::commit::Header;
use std::ffi::CString;

fn main() {
    let field = CString::new("line_one").unwrap();
    let header = Header::new(&field, c"value");
    drop(field);
    let _ = header.as_ref().field();
}
