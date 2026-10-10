// Every commit is shared with its repository's cache: no owner lends an exclusive
// handle.
use libgit2::libgit2::object::commit::{CommitArc, CommitMut};

fn exclusive<'a>(commit: &'a mut CommitArc<'static>) -> CommitMut<'a, 'static> {
    commit.as_mut()
}

fn main() {}
