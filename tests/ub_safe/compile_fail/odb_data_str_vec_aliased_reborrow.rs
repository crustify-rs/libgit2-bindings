// An `OdbDataStrVec`'s shared and mutable views cannot coexist.
use {libgit2::libgit2::src::libgit2::odb::OdbDataStrFree, libgit2::libgit2::src::libgit2::odb::OdbDataStrVec};

fn main() {
    let mut v = OdbDataStrVec::dup_all(&[c"a"]).unwrap();
    let shared = v.as_slice();
    v.as_mut_slice()[0] = OdbDataStrFree::dup(c"b").unwrap();
    let _ = shared[0].as_bytes();
}
