// A commit keeps the repository it was looked up in, which its tree and parent lookups
// read: it, and every further reference to it, borrows that repository.
use libgit2::libgit2::object::commit::Commit;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let repo = Repository::open(c".").unwrap();
    let id = Oid::zero(OidType::Sha1);
    let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let copy = commit.as_ref().to_commit();
    drop(commit);
    drop(repo);
    drop(copy);
}
