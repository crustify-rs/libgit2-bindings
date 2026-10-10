// A commit array carries its commits' repository borrow: it cannot be dropped after the
// repository, nor can what it lends or hands out, and its exclusive handle takes
// commits of that repository's borrow only.
use libgit2::libgit2::object::commit::{CommitArc, CommitArrayMut};
use libgit2::libgit2::repository::repository::Repository;

fn store<'r>(mut array: CommitArrayMut<'_, 'static>, commit: CommitArc<'r>) {
    let _ = array.set_commits(vec![commit]);
}

fn main() {
    let repo = Repository::open(c".").unwrap();
    let parents = repo.as_ref().commit_parents().unwrap();
    drop(repo);
    drop(parents);

    let repo = Repository::open(c".").unwrap();
    let taken = repo.as_ref().commit_parents().unwrap().as_mut().take();
    drop(repo);
    drop(taken);
}
