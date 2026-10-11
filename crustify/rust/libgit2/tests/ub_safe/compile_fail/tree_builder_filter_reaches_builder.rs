// A filter's closure runs while the builder is borrowed exclusively: it cannot reach
// the builder, nor can an entry the builder lent before survive the filter.
use libgit2::libgit2::object::tree::{TreeBuilderBox, TreeBuilderFilter, TreeEntryRef};

fn reenter(mut builder: TreeBuilderBox<'_>) {
    let mut f = |_: TreeEntryRef<'_>| builder.as_ref().is_empty();
    let mut filter = TreeBuilderFilter::new(&mut f);
    builder.as_mut().remove_if(&mut filter);
}

fn stale(mut builder: TreeBuilderBox<'_>) {
    let entry = builder.as_ref().get(c"README");
    let mut f = |_: TreeEntryRef<'_>| true;
    builder.as_mut().remove_if(&mut TreeBuilderFilter::new(&mut f));
    let _ = entry;
}

fn main() {
    let _ = (reenter, stale);
}
