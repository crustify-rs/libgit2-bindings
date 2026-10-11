use libgit2::libgit2::diff::diff::Diff;
use libgit2::libgit2::repository::repository::Repository;
fn main() {
    let repo = Repository::new().unwrap();
    let d = Diff::tree_to_tree(repo.as_ref(), None, None).unwrap();
    drop(repo);
    let _ = d.as_ref().len();
}
