// The options cannot outlive the path array their setter borrowed.
use libgit2::libgit2::src::libgit2::strarray::StrArrayView;
use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;

fn main() {
    let mut options = CheckoutOptions::new();
    {
        let paths = [c"*.txt"];
        let view = StrArrayView::new(&paths);
        options.as_mut().set_paths(view.as_ref());
    }
    let _ = options.as_ref().paths().len();
}
