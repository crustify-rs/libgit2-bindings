// A trailer cannot outlive the strings it borrows.
use libgit2::libgit2::object::message::MessageTrailer;
use std::ffi::CString;

fn main() {
    let trailer = {
        let key = CString::new("Signed-off-by").unwrap();
        MessageTrailer::new(&key, c"A U Thor")
    };
    println!("{:?}", trailer.as_ref().key());
}
