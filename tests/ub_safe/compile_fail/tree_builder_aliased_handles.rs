// A tree builder lends one exclusive handle at a time, and no shared handle beside it.
use libgit2::libgit2::src::libgit2::tree::TreeBuilder;
use libgit2::libgit2::src::libgit2::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let mut first = builder.as_mut();
    let mut second = builder.as_mut();
    first.clear();
    second.clear();

    let mut exclusive = builder.as_mut();
    let shared = builder.as_ref();
    exclusive.clear();
    let _ = shared;

    let mut outer = builder.as_mut();
    let mut inner = outer.as_mut();
    outer.clear();
    inner.clear();
}
