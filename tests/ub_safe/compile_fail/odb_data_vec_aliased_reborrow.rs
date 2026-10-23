// A shared view cannot coexist with the exclusive one.
use libgit2::libgit2::raw::OdbDataVec;

fn main() {
    let mut v = OdbDataVec::<u8>::from_slice(b"odb").unwrap();
    let shared = v.as_slice();
    let exclusive = v.as_mut_slice();
    exclusive[0] = shared[1];
}
