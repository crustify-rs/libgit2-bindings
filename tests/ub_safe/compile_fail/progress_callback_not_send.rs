// A `ProgressCallback` stays on the thread of its closure: libgit2 calls it synchronously,
// and the closure need not be `Send`.
use std::ops::ControlFlow;
use std::thread;

use libgit2::libgit2::src::libgit2::indexer::{IndexerProgressRef, ProgressCallback};

fn main() {
    let mut f = |_: IndexerProgressRef<'_>| ControlFlow::Continue(());
    let callback = ProgressCallback::new(&mut f);
    thread::scope(|s| {
        s.spawn(move || drop(callback));
    });
}
