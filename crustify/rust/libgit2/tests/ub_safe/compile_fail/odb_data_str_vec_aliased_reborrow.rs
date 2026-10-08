// An `OdbDataStrVec`'s shared and mutable views cannot coexist.
use libgit2::libgit2::raw::{OdbDataStrFree, OdbDataStrVec};

fn main() {
    let mut v = OdbDataStrVec::dup_all(&[c"a"]).unwrap();
    let shared = v.as_slice();
    v.as_mut_slice()[0] = OdbDataStrFree::dup(c"b").unwrap();
    let _ = shared[0].as_bytes();
}
