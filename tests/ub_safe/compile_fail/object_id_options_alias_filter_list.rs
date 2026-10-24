// A filter list lent to id options cannot be reached exclusively meanwhile.
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::filter::FilterList;
use libgit2::libgit2::src::libgit2::object::ObjectIdOptions;
use libgit2::libgit2::src::libgit2::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let mut list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
    let mut opts = ObjectIdOptions::new();
    opts.as_mut().set_filters(Some(list.as_mut()));
    let other = list.as_mut();
    println!("{:?} {other:?}", opts.as_ref().filters());
}
