// A shared view cannot coexist with the exclusive one.
use libgit2::libgit2::src::libgit2::odb::OdbDataVec;

fn main() {
    let mut v = OdbDataVec::<u8>::from_slice(b"odb").unwrap();
    let shared = v.as_slice();
    let exclusive = v.as_mut_slice();
    exclusive[0] = shared[1];
}
