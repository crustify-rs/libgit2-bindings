// A handle cannot outlive the `IndexerProgress` value it borrows.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::indexer::IndexerProgress;

fn main() {
    let handle = {
        let progress = IndexerProgress::zeroed();
        progress.as_ref()
    };
    let _ = handle.total_objects();
}
