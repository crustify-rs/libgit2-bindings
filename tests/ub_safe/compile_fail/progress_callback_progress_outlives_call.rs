// A progress lent to a `ProgressCallback`'s closure cannot be kept past the call.
use std::ops::ControlFlow;

use libgit2::libgit2::src::libgit2::indexer::{IndexerProgressRef, ProgressCallback};

fn keep<'k>(kept: &mut Option<IndexerProgressRef<'k>>) {
    let mut f = |progress: IndexerProgressRef<'_>| {
        *kept = Some(progress);
        ControlFlow::Continue(())
    };
    let _ = ProgressCallback::new(&mut f);
}

fn main() {
    keep(&mut None);
}
