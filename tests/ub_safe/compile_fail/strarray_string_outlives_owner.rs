// A borrowed string cannot outlive the array that owns it.
use libgit2::libgit2::src::libgit2::strarray::StrArray;

fn main() {
    let s = {
        let array = StrArray::new();
        array.as_ref().get(0)
    };
    let _ = s;
}
