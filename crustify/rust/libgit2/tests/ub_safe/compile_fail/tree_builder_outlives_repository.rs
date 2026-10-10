// A tree builder keeps its repository, which inserting and writing reach, so it cannot
// be dropped after the repository, nor kept while the repository lends an exclusive
// handle.
use libgit2::libgit2::object::tree::TreeBuilder;
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let builder = TreeBuilder::new(repo.as_ref()).unwrap();
    drop(repo);
    drop(builder);

    let mut repo = Repository::new().unwrap();
    let builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let _exclusive = repo.as_mut();
    drop(builder);
}
