// The shared handle only reads: setters live on `IndexerProgressMut`.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::indexer::IndexerProgress;

fn main() {
    let progress = IndexerProgress::zeroed();
    let r = progress.as_ref();
    r.set_received_bytes(1);
}
