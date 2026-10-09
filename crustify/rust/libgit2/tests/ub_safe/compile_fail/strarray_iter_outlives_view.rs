// A view's iterator borrows the view, and cannot outlive it.
use libgit2::libgit2::core::strarray::StrArrayView;

fn main() {
    let names = [c"a", c"b"];
    let it = {
        let view = StrArrayView::new(&names);
        view.as_ref().iter()
    };
    let _ = it.count();
}
