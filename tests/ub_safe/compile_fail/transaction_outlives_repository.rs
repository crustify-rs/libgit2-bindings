use libgit2::libgit2::src::libgit2::transaction::Transaction;
use libgit2::libgit2::src::libgit2::repository::Repository;
fn main() {
    let repo = Repository::open(c".").unwrap();
    let tx = Transaction::new(repo.as_ref()).unwrap();
    drop(repo);
    drop(tx);
}
