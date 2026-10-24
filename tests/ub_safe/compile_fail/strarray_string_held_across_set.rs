// Replacing the strings frees them, so no borrowed string may be held across it.
use libgit2::libgit2::src::libgit2::strarray::StrArray;
use libgit2::libgit2::src::libgit2::odb::OdbDataStrVec;

fn main() {
    let mut array = StrArray::new();
    let s = array.as_ref().get(0);
    array.as_mut().set_strings(OdbDataStrVec::new());
    let _ = s;
}
