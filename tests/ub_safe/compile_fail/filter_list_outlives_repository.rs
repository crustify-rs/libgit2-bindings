// A filter list keeps a bare pointer to the repository it was built for, which its
// filters read when it is applied: it cannot be dropped after the repository, whichever
// constructor built it, nor escape the repository's borrow.
use libgit2::ffi;
use {libgit2::libgit2::src::libgit2::filter::FilterList, libgit2::libgit2::src::libgit2::filter::FilterListBox};
use libgit2::libgit2::src::libgit2::repository::Repository;

fn escape() -> FilterListBox<'static> {
    let repo = Repository::new().unwrap();
    FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap()
}

fn main() {
    let repo = Repository::open(c".").unwrap();
    let list = FilterList::load(repo.as_ref(), c"a.txt", ffi::GIT_FILTER_TO_ODB, 0).unwrap();
    drop(repo);
    drop(list);

    let repo = Repository::open(c".").unwrap();
    let list = FilterList::load_ext(repo.as_ref(), c"a.txt", ffi::GIT_FILTER_TO_ODB, None);
    drop(repo);
    drop(list);
    let _ = escape();
}
