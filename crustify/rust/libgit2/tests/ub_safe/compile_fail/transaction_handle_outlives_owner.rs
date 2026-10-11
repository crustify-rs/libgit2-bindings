use libgit2::libgit2::refs::transaction::Transaction;
use libgit2::libgit2::repository::repository::Repository;
fn main() {
    let repo = Repository::open(c".").unwrap();
    let tx = Transaction::new(repo.as_ref()).unwrap();
    let handle = tx.as_ref();
    drop(tx);
    println!("{handle:?}");
}
