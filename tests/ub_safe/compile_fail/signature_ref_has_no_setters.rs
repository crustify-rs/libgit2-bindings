// The shared handle only reads: a borrowed signature (a view's, or one libgit2 lends)
// cannot have its strings replaced.
use libgit2::libgit2::core::types::{SignatureView, Time};
use libgit2::libgit2::raw::OdbDataStrFree;

fn main() {
    let view = SignatureView::new(c"A", c"a@b", Time::new(0, 0));
    view.as_ref().set_name(OdbDataStrFree::dup(c"B").unwrap());
}
