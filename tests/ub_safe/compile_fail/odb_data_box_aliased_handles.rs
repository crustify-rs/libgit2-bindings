// A shared handle cannot coexist with the exclusive one of the same box.
use libgit2::libgit2::src::libgit2::odb::OdbDataBox;

#[repr(C)]
pub struct pair_st {
    pub a: u64,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);
// SAFETY: any `a` is valid.
unsafe impl ffibox::CZeroable for Pair {}

fn main() {
    let mut pair = OdbDataBox::<Pair>::new_zeroed().unwrap();
    let shared = pair.as_ref();
    let exclusive = pair.as_mut();
    let _ = (shared, exclusive);
}
