// While a filter list lives, its repository is shared: the list reads it when applied,
// so nothing may reach the repository exclusively (replacing its namespace, identity
// or working directory) meanwhile.
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::filter::FilterList;
use libgit2::libgit2::src::libgit2::repository::Repository;

fn main() {
    let mut repo = Repository::open(c".").unwrap();
    let list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_WORKTREE, 0).unwrap();
    repo.as_mut().set_namespace(None).unwrap();
    drop(list);
}
