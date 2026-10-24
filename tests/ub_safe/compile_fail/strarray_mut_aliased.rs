// Two exclusive handles over one array cannot coexist.
use libgit2::libgit2::src::libgit2::strarray::StrArray;

fn main() {
    let mut array = StrArray::new();
    let mut a = array.as_mut();
    let b = array.as_mut();
    a.truncate(0);
    drop(b);
}
