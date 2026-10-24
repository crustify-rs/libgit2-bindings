// An odb owner a repository handed out cannot be used on another thread while the
// repository, whose object cache the odb reads, is dropped on its own thread.
use libgit2::libgit2::src::libgit2::odb::OdbArc;
use {libgit2::libgit2::src::libgit2::repository::RepositoryBox, libgit2::libgit2::src::libgit2::repository::RepositoryRef};

// What the wrapper of `git_repository_odb` returns.
fn odb_of<'r>(_repo: RepositoryRef<'r>) -> OdbArc<'r> {
    unimplemented!()
}

fn escape(repo: RepositoryBox) {
    let odb = odb_of(repo.as_ref());
    std::thread::scope(|s| {
        s.spawn(|| drop(odb));
        drop(repo);
    });
}

fn main() {
    let _ = escape;
}
