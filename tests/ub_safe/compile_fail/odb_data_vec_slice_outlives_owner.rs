// A slice of an `OdbDataVec` cannot outlive the buffer it borrows.
use libgit2::libgit2::src::libgit2::odb::OdbDataVec;

fn main() {
    let v = OdbDataVec::<u8>::from_slice(b"odb").unwrap();
    let s = v.as_slice();
    drop(v);
    let _ = s[0];
}
