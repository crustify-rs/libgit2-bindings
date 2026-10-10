// An owned path array cannot be changed (its strings freed) while options borrow it.
use libgit2::libgit2::core::strarray::StrArray;
use libgit2::libgit2::workdir::checkout::CheckoutOptions;

fn main() {
    let mut owned = StrArray::from_strs(&[c"a.txt"]).unwrap();
    let mut options = CheckoutOptions::new();
    options.as_mut().set_paths(owned.as_ref());
    owned.as_mut().truncate(0);
    let _ = options.as_ref().paths().len();
}
