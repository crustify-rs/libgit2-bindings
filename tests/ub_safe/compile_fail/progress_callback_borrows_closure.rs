// A `ProgressCallback` borrows its closure exclusively: what the closure captures cannot be
// read while the callback can still run it.
use std::ops::ControlFlow;

use libgit2::libgit2::src::libgit2::indexer::{IndexerProgress, IndexerProgressRef, ProgressCallback};

fn main() {
    let mut seen = Vec::new();
    let mut f = |progress: IndexerProgressRef<'_>| {
        seen.push(progress.to_progress());
        ControlFlow::Continue(())
    };
    let mut callback = ProgressCallback::new(&mut f);
    let progress = IndexerProgress::default();
    let _ = callback.call(progress.as_ref());
    assert!(seen.is_empty());
    let _ = callback.call(progress.as_ref());
}
