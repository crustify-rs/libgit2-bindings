// A commit builder lives on the stack of the routine writing the commit, for its signing
// callback alone: safe code outside the crate cannot make a handle to one, even with the
// seam trait in scope, and the exclusive handle does not alias. (That neither handle is
// `Send` is a doctest on `CommitBuilder`: the toolchains word that error differently.)
use ffibox::CMutSeam;
use libgit2::libgit2::src::libgit2::commit::CommitBuilderMut;

fn alias(mut builder: CommitBuilderMut<'_>) {
    let shared = builder.as_ref();
    let exclusive = builder.as_mut();
    drop(exclusive);
    let _ = shared;
}

fn main() {
    let mut slot = 0u64;
    let _ = CommitBuilderMut::from_ptr(core::ptr::from_mut(&mut slot).cast());
}
