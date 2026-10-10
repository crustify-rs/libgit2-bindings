// A tree builder's handles borrow its owner, so neither outlives it.
use libgit2::libgit2::object::tree::TreeBuilder;
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let shared = builder.as_ref();
    drop(builder);
    let _ = shared;

    let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let exclusive = builder.as_mut();
    drop(builder);
    drop(exclusive);
}
