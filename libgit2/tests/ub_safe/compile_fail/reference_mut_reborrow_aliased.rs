// An exclusive handle is frozen while its reborrow lives.
use libgit2::libgit2::refs::refs::ReferenceMut;

fn alias(mut exclusive: ReferenceMut<'_, '_>) {
    let reborrowed = exclusive.as_mut();
    let shared = exclusive.as_ref();
    drop((reborrowed, shared));
}

fn main() {}
