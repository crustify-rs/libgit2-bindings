//! UB tests for `src/libgit2/indexer.c`: the `IndexerProgress` value
//! (`git_indexer_progress`) and its handles, and the `ProgressCallback`
//! (`git_indexer_progress_cb`).
//!
//! An `IndexerProgress` is a plain 32-byte value without padding: six `unsigned int`
//! counters and a `size_t`. Its handles project each field out of a pointer, so these tests
//! exercise every getter and setter on every holder Rust has for one (a local, a heap slot,
//! an element of a run that moves when it grows, a copy), through reborrows, and across
//! threads under the `Send` / `Sync` the crate asserts (`TSan`). A misaligned or
//! out-of-bounds projection, or a write through the shared handle, would be flagged by Miri
//! and ASan/UBSan. Handles that outlive their value, aliased exclusive handles and setters
//! on the shared handle do not compile (`compile_fail/`). None of them calls libgit2, so all
//! run under Miri; no C routine is given a pointer to these values, so there is no foreign
//! write or retained pointer to race (`BSan`).
//!
//! A `ProgressCallback` is called through its `extern "C"` trampoline, as libgit2 calls it,
//! with progresses lent from every holder, from inside another callback's closure
//! (reentrance), after its closure panicked, and on several threads at once, each over its
//! own closure reading one shared progress (`TSan`). The trampoline is Rust, so these run
//! under Miri too; the unit tests have libgit2 itself call it while indexing a pack. A
//! progress kept past the call, a callback moved to another thread and a closure used while
//! its callback borrows it do not compile (`compile_fail/`).

use std::ops::ControlFlow;
use std::panic::{self, AssertUnwindSafe};
use std::thread;

use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::indexer::{
    IndexerProgress, IndexerProgressMut, IndexerProgressRef, ProgressCallback,
};

type Counters = (u32, u32, u32, u32, u32, u32, usize);

fn read(r: IndexerProgressRef<'_>) -> Counters {
    (
        r.total_objects(),
        r.indexed_objects(),
        r.received_objects(),
        r.local_objects(),
        r.total_deltas(),
        r.indexed_deltas(),
        r.received_bytes(),
    )
}

fn write(mut m: IndexerProgressMut<'_>, c: Counters) {
    m.set_total_objects(c.0);
    m.set_indexed_objects(c.1);
    m.set_received_objects(c.2);
    m.set_local_objects(c.3);
    m.set_total_deltas(c.4);
    m.set_indexed_deltas(c.5);
    m.set_received_bytes(c.6);
}

/// Distinct values in every counter, the extremes included, so a getter reading a
/// neighbouring field shows.
fn counters(i: u32) -> Counters {
    let edge = [0, 1, u32::MAX - 1, u32::MAX][i as usize % 4];
    (
        edge,
        edge ^ 1,
        edge ^ 2,
        edge ^ 3,
        edge ^ 4,
        edge ^ 5,
        [0, 1, usize::MAX - 1, usize::MAX][i as usize % 4] ^ 6,
    )
}

#[test]
fn every_counter_round_trips_on_every_holder() {
    let mut local = IndexerProgress::zeroed();
    let mut boxed = Box::new(IndexerProgress::default());
    let mut run = vec![IndexerProgress::zeroed()];
    for i in 0..8 {
        let c = counters(i);
        write(local.as_mut(), c);
        write(IndexerProgress::as_mut(&mut boxed), c);
        // Growing the run moves its elements: every handle is taken afresh.
        run.push(IndexerProgress::zeroed());
        let last = run.len() - 1;
        write(run[last].as_mut(), c);
        for progress in [&local, &*boxed, &run[last]] {
            assert_eq!(read(progress.as_ref()), c);
            assert_eq!(read(progress.as_ref().to_progress().as_ref()), c);
        }
    }
    // Earlier elements kept what was written to them before the run moved.
    for (i, progress) in (0..).zip(&run[1..]) {
        assert_eq!(read(progress.as_ref()), counters(i));
    }
}

#[test]
fn each_setter_writes_only_its_counter() {
    let setters: [fn(&mut IndexerProgressMut<'_>); 7] = [
        |m| m.set_total_objects(u32::MAX),
        |m| m.set_indexed_objects(u32::MAX),
        |m| m.set_received_objects(u32::MAX),
        |m| m.set_local_objects(u32::MAX),
        |m| m.set_total_deltas(u32::MAX),
        |m| m.set_indexed_deltas(u32::MAX),
        |m| m.set_received_bytes(usize::MAX),
    ];
    for (i, set) in setters.into_iter().enumerate() {
        let mut progress = IndexerProgress::zeroed();
        set(&mut progress.as_mut());
        let c = read(progress.as_ref());
        let got = [
            u64::from(c.0),
            u64::from(c.1),
            u64::from(c.2),
            u64::from(c.3),
            u64::from(c.4),
            u64::from(c.5),
            c.6 as u64,
        ];
        for (j, value) in got.into_iter().enumerate() {
            assert_eq!(value != 0, i == j, "setter {i} touched counter {j}");
        }
    }
}

#[test]
fn reborrows_keep_one_writer() {
    let mut progress = IndexerProgress::zeroed();
    let mut m = progress.as_mut();
    {
        let mut inner = m.as_mut();
        inner.set_received_objects(4);
        assert_eq!(inner.as_ref().received_objects(), 4);
    }
    let shared = m.as_ref();
    let copy = shared;
    assert_eq!(read(copy), (0, 0, 4, 0, 0, 0, 0));
    m.set_received_bytes(512);
    assert_eq!(read(progress.as_ref()), (0, 0, 4, 0, 0, 0, 512));
}

#[test]
fn copies_are_independent() {
    let c = {
        let a = IndexerProgress::from(libgit2::ffi::git_indexer_progress {
            total_objects: 3,
            ..Default::default()
        });
        let mut b = a;
        b.as_mut().set_indexed_objects(3);
        assert_eq!(read(a.as_ref()), (3, 0, 0, 0, 0, 0, 0));
        assert_eq!(read(b.as_ref()), (3, 3, 0, 0, 0, 0, 0));
        a.as_ref().to_progress()
    };
    // The copy outlives the value it was read from.
    assert_eq!(read(c.as_ref()), (3, 0, 0, 0, 0, 0, 0));
}

#[test]
fn shared_handles_read_across_threads() {
    let mut progress = IndexerProgress::zeroed();
    write(progress.as_mut(), counters(2));
    let r = progress.as_ref();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(move || {
                for _ in 0..50 {
                    assert_eq!(read(r), counters(2));
                }
            });
        }
    });
}

#[test]
fn values_and_exclusive_handles_move_across_threads() {
    let mut progress = IndexerProgress::zeroed();
    thread::scope(|s| {
        let m = progress.as_mut();
        s.spawn(move || write(m, counters(3))).join().unwrap();
    });
    assert_eq!(read(progress.as_ref()), counters(3));
    let moved = thread::spawn(move || {
        let mut progress = progress;
        progress.as_mut().set_local_objects(9);
        progress
    })
    .join()
    .unwrap();
    assert_eq!(moved.as_ref().local_objects(), 9);
}

#[test]
fn callback_reads_progress_lent_from_every_holder() {
    let mut local = IndexerProgress::zeroed();
    let mut boxed = Box::new(IndexerProgress::default());
    let mut run = vec![IndexerProgress::zeroed()];
    let mut seen = Vec::new();
    for i in 0..4 {
        let c = counters(i);
        write(local.as_mut(), c);
        write(IndexerProgress::as_mut(&mut boxed), c);
        run.push(IndexerProgress::zeroed());
        let last = run.len() - 1;
        write(run[last].as_mut(), c);
        let mut record = |p: IndexerProgressRef<'_>| {
            seen.push(read(p));
            ControlFlow::Continue(())
        };
        let mut callback = ProgressCallback::new(&mut record);
        for progress in [&local, &*boxed, &run[last]] {
            assert_eq!(callback.call(progress.as_ref()), ControlFlow::Continue(()));
        }
        // A handle reborrowed from an exclusive one lends the same counters.
        let m = local.as_mut();
        assert_eq!(callback.call(m.as_ref()), ControlFlow::Continue(()));
    }
    let expected: Vec<Counters> = (0..4).flat_map(|i| [counters(i); 4]).collect();
    assert_eq!(seen, expected);
}

#[test]
fn break_is_reported_and_the_callback_stays_usable() {
    let progress = IndexerProgress::zeroed();
    let mut calls = 0u32;
    let mut every_other = |_: IndexerProgressRef<'_>| {
        calls += 1;
        if calls.is_multiple_of(2) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut callback = ProgressCallback::new(&mut every_other);
    let flows: Vec<_> = (0..4).map(|_| callback.call(progress.as_ref())).collect();
    drop(callback);
    assert_eq!(
        flows,
        [
            ControlFlow::Continue(()),
            ControlFlow::Break(()),
            ControlFlow::Continue(()),
            ControlFlow::Break(()),
        ]
    );
    assert_eq!(calls, 4);
}

#[test]
fn callbacks_nest_inside_a_closure() {
    let mut outer_progress = IndexerProgress::zeroed();
    outer_progress.as_mut().set_total_objects(2);
    let mut inner_seen = Vec::new();
    let mut outer = |p: IndexerProgressRef<'_>| {
        // The closure lends what it was lent to a second callback, re-entering the
        // trampoline while the first call is live.
        let mut inner = |q: IndexerProgressRef<'_>| {
            inner_seen.push(read(q));
            ControlFlow::Break(())
        };
        let mut nested = ProgressCallback::new(&mut inner);
        let flow = nested.call(p);
        let mut local = p.to_progress();
        local.as_mut().set_indexed_objects(1);
        assert_eq!(nested.call(local.as_ref()), ControlFlow::Break(()));
        flow
    };
    let mut callback = ProgressCallback::new(&mut outer);
    assert_eq!(
        callback.call(outer_progress.as_ref()),
        ControlFlow::Break(())
    );
    drop(callback);
    assert_eq!(inner_seen, [(2, 0, 0, 0, 0, 0, 0), (2, 1, 0, 0, 0, 0, 0)]);
}

#[test]
fn a_panic_resumes_from_call_and_the_closure_runs_again() {
    let progress = IndexerProgress::zeroed();
    let mut calls = 0u32;
    let mut first_panics = |_: IndexerProgressRef<'_>| {
        calls += 1;
        assert!(calls > 1, "first call");
        ControlFlow::Continue(())
    };
    let mut callback = ProgressCallback::new(&mut first_panics);
    let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(progress.as_ref())));
    let payload = caught.unwrap_err();
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"first call"));
    // The panic was taken back to Rust: the next call runs the closure again.
    assert_eq!(
        format!("{callback:?}"),
        "ProgressCallback { panicked: false, .. }"
    );
    assert_eq!(callback.call(progress.as_ref()), ControlFlow::Continue(()));
    drop(callback);
    assert_eq!(calls, 2);
}

#[test]
fn callbacks_run_on_several_threads_over_one_progress() {
    let mut progress = IndexerProgress::zeroed();
    write(progress.as_mut(), counters(1));
    let r = progress.as_ref();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(move || {
                let mut sum = 0u64;
                let mut add = |p: IndexerProgressRef<'_>| {
                    sum += u64::from(p.total_objects());
                    ControlFlow::Continue(())
                };
                let mut callback = ProgressCallback::new(&mut add);
                for _ in 0..50 {
                    assert_eq!(callback.call(r), ControlFlow::Continue(()));
                }
                drop(callback);
                assert_eq!(sum, 50 * u64::from(counters(1).0));
            });
        }
    });
}
