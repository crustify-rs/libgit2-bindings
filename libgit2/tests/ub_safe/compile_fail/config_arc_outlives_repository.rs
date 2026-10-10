// A config a repository handed out borrows that repository: its file backends keep the
// repository for their `includeIf` conditions, evaluated on every reparse, so it cannot
// be dropped after the repository, nor installed in a repository that may outlive it.
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let repo = Repository::open(c".").unwrap();
    let config = repo.as_ref().config().unwrap();
    drop(repo);
    drop(config);

    let mut target = Repository::new().unwrap();
    let source = Repository::open(c".").unwrap();
    let lent = source.as_ref().config().unwrap();
    target.as_mut().set_config(&lent);
}
