// An odb owner a repository handed out borrows that repository: it cannot be dropped,
// nor used on another thread, after the repository, whose object cache the odb reads.
use libgit2::libgit2::odb::odb::OdbArc;
use libgit2::libgit2::repository::repository::{Repository, RepositoryRef};

// What the wrapper of `git_repository_odb` returns.
fn odb_of<'r>(_repo: RepositoryRef<'r>) -> OdbArc<'r> {
    unimplemented!()
}

fn main() {
    let repo = Repository::new().unwrap();
    let odb = odb_of(repo.as_ref());
    drop(repo);
    drop(odb);
}
