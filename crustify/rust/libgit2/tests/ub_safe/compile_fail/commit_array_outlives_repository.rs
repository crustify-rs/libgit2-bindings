// A commit array carries its commits' repository borrow: it cannot be dropped after the
// repository, nor can what it lends or hands out. (That its exclusive handle takes
// commits of that borrow only is `CommitArrayMut::set_commits`' `compile_fail,E0521`
// doctest: the note rustc attaches to that error is worded differently across the
// stable and nightly toolchains the suites run under.)
use libgit2::libgit2::repository::repository::Repository;

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
