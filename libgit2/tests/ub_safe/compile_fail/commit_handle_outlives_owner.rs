// A borrowed commit lives no longer than what keeps it alive: its owner, or the array
// that lends it.
use libgit2::libgit2::object::commit::{CommitArc, CommitArrayVal, CommitRef};

fn from_owner<'a, 'r>(commit: CommitArc<'r>) -> CommitRef<'a, 'r> {
    commit.as_ref()
}

fn from_array<'a, 'r>(array: CommitArrayVal<'r>) -> CommitRef<'a, 'r> {
    array.as_ref().get(0).unwrap()
}

fn main() {}
