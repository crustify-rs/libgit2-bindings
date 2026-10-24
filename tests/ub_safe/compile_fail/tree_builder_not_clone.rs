// libgit2 has no copy routine for a tree builder, so the owner does not clone. (That
// neither the owner nor a handle crosses threads is held by the `Send` / `Sync`
// doctests on `TreeBuilder`: their `E0277` chains differ between toolchains.)
use libgit2::libgit2::src::libgit2::tree::TreeBuilder;
use libgit2::libgit2::src::libgit2::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let _copy = builder.clone();
}
