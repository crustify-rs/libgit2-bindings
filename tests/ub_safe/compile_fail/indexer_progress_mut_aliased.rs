// Two exclusive handles to one `IndexerProgress` cannot coexist, nor a shared one beside them.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::indexer::IndexerProgress;

fn main() {
    let mut progress = IndexerProgress::zeroed();
    let mut a = progress.as_mut();
    let r = progress.as_ref();
    let mut b = progress.as_mut();
    a.set_total_objects(1);
    b.set_total_objects(2);
    let _ = r.total_objects();
}
