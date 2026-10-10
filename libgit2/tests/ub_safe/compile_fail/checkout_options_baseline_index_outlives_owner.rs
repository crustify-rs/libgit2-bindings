// The options cannot outlive the index owner their baseline index was borrowed from.
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::workdir::checkout::CheckoutOptions;

fn main() {
    let index = Index::new().unwrap();
    let mut options = CheckoutOptions::new();
    options.as_mut().set_baseline_index(Some(index.as_ref()));
    drop(index);
    let _ = options.as_ref().baseline_index();
}
