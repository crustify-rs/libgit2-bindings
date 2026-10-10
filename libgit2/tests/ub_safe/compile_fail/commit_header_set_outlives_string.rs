// A setter stores a borrow for the header's whole lifetime: a shorter-lived string
// cannot be stored in a header that is used after the string is gone.
use libgit2::libgit2::object::commit::Header;
use std::ffi::CString;

fn main() {
    let mut header = Header::new(c"field", c"value");
    {
        let value = CString::new("short-lived").unwrap();
        header.as_mut().set_value(&value);
    }
    let _ = header.as_ref().value();
}
