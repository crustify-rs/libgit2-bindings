// A boxed index lends no exclusive handle while options borrow it as their baseline.
use libgit2::libgit2::index::index::Index;
use libgit2::libgit2::workdir::checkout::CheckoutOptions;

fn main() {
    let mut index = Index::new().unwrap();
    let mut options = CheckoutOptions::new();
    options.as_mut().set_baseline_index(Some(index.as_ref()));
    let _exclusive = index.as_mut();
    let _ = options.as_ref().baseline_index();
}
