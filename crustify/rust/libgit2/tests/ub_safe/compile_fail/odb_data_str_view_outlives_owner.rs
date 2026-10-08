// A string's `CStr` view cannot outlive the string it borrows.
use libgit2::libgit2::raw::OdbDataStrFree;

fn main() {
    let s = OdbDataStrFree::dup(c"odb").unwrap();
    let view = s.as_c_str();
    drop(s);
    let _ = view.to_bytes();
}
