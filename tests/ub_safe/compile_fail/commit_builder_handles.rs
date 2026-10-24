// A commit builder lives on the stack of the routine writing the commit, for its signing
// callback alone: nothing outside the crate can make a handle to one, and the exclusive
// handle does not alias. (That neither handle is `Send` is a doctest on `CommitBuilder`:
// the toolchains word that error differently.)
use libgit2::libgit2::src::libgit2::commit::CommitBuilderMut;

fn alias(mut builder: CommitBuilderMut<'_>) {
    let shared = builder.as_ref();
    let exclusive = builder.as_mut();
    drop(exclusive);
    let _ = shared;
}

fn main() {
    let mut slot = 0u64;
    let _ = unsafe { CommitBuilderMut::from_ptr(core::ptr::from_mut(&mut slot).cast()) };
}
