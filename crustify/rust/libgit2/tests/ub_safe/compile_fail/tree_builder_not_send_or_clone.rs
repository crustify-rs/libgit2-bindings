// A tree builder reaches its repository, which is not `Sync`, so neither the owner nor a
// handle crosses threads; libgit2 has no copy routine, so the owner does not clone.
use libgit2::libgit2::object::tree::{TreeBuilder, TreeBuilderRef};
use libgit2::libgit2::repository::repository::Repository;

fn send<T: Send>(_: T) {}
fn sync<T: Sync>(_: &T) {}

fn main() {
    let repo = Repository::new().unwrap();
    let builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let shared: TreeBuilderRef<'_, '_> = builder.as_ref();
    sync(&shared);
    let _copy = builder.clone();
    send(builder);
}
