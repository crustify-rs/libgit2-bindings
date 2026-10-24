use libgit2::libgit2::src::libgit2::transaction::Transaction;
use libgit2::libgit2::src::libgit2::repository::Repository;
fn main() {
    let repo = Repository::open(c".").unwrap();
    let mut tx = Transaction::new(repo.as_ref()).unwrap();
    let mut exclusive = tx.as_mut();
    let shared = exclusive.as_ref();
    let _alias = exclusive.as_mut();
    println!("{shared:?}");
}
