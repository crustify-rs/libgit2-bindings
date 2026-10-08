// A handle borrowed from an `OdbDataBox` cannot outlive the box.
use libgit2::libgit2::raw::OdbDataBox;

#[repr(C)]
pub struct pair_st {
    pub a: u64,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);
// SAFETY: any `a` is valid.
unsafe impl ffibox::CZeroable for Pair {}

fn main() {
    let pair = OdbDataBox::<Pair>::new_zeroed().unwrap();
    let handle = pair.as_ref();
    drop(pair);
    let _ = handle;
}
