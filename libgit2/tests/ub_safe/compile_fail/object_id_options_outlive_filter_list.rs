// Id options cannot outlive the filter list they borrow.
use libgit2::ffi;
use libgit2::libgit2::filter::filter::FilterList;
use libgit2::libgit2::object::object::ObjectIdOptions;
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let mut opts = ObjectIdOptions::new();
    {
        let mut list = FilterList::new(repo.as_ref(), ffi::GIT_FILTER_TO_ODB, 0).unwrap();
        opts.as_mut().set_filters(Some(list.as_mut()));
    }
    println!("{:?}", opts.as_ref().filters());
}
