//! UB tests for the safe API of `git2/checkout.h`: [`CheckoutOptions`], its handles,
//! and what its setters borrow.
//!
//! The options store pointers to the strings and path arrays their setters borrow, and
//! read them back through the getters; every source here is an allocation of exactly
//! its length (`CString`, `StrArrayView`'s pointer array, `StrArray::from_strs`'
//! copies), so `ASan` flags a read past one, and every borrowed source is released after
//! the options, exactly once (`ASan`, `LSan`). The handles reborrow while the options keep
//! those pointers (`BSan`). Options are built on several threads while another
//! initialises libgit2 for the first time: `git_checkout_options_init` touches no
//! global state, so nothing races (`TSan`). Illegal borrows are in `compile_fail/`, and
//! in the type's doctests.
//!
//! The tests that run real checkouts over a borrowed baseline index and tree need the
//! checkout routines (`git_checkout_head`, `_index`, `_tree`), which are not wrapped yet:
//! they come with those wrappers.
//!
//! Every test builds its options through `git_checkout_options_init`, so Miri skips
//! them; the inline unit tests run the accessors under Miri over a Rust-built value.
//!
//! [`CheckoutPerfdata`] holds three plain counters, which Rust values are built, copied
//! and read through without C: its handles reborrow and are read and written in turn
//! (Miri, `BSan`), copies cross threads and a shared value is read from several at once
//! (`TSan`, for its `Send` and `Sync`). A handle outliving its value, or a shared handle
//! beside the exclusive one, is in `compile_fail/`.

use std::ffi::{CStr, CString};
use std::sync::Barrier;
use std::thread;

use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::checkout::{
    CheckoutOptions, CheckoutOptionsMut, CheckoutOptionsRef, CheckoutPerfdata, CheckoutPerfdataMut,
    CheckoutPerfdataRef,
};
use libgit2::libgit2::src::libgit2::libgit2 as global;
use libgit2::libgit2::src::libgit2::strarray::{StrArray, StrArrayView};

use crate::fixture::isolated;

/// `n` distinct strings of lengths 1, 2, 3, ...
fn strings(n: usize) -> Vec<CString> {
    (1..=n)
        .map(|i| CString::new(vec![b'a' + u8::try_from(i % 26).unwrap(); i]).unwrap())
        .collect()
}

/// Every byte the getters reach.
fn read_all(o: CheckoutOptionsRef<'_, '_>) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = o.paths().iter().map(|s| s.to_bytes().to_vec()).collect();
    for s in [
        o.target_directory(),
        o.ancestor_label(),
        o.our_label(),
        o.their_label(),
    ] {
        out.push(s.map_or(b"-".to_vec(), |s| s.to_bytes().to_vec()));
    }
    out.push(format!("{o:?}").into_bytes());
    out
}

fn set_strings<'o>(mut m: CheckoutOptionsMut<'_, 'o>, s: &'o [CString]) {
    m.set_target_directory(Some(&s[0]));
    m.set_ancestor_label(Some(&s[1]));
    m.set_our_label(Some(&s[2]));
    m.set_their_label(Some(&s[3]));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn defaults_then_every_setter() {
    let mut options = CheckoutOptions::new();
    let o = options.as_ref();
    assert_eq!(o.version(), 1);
    assert_eq!(
        (
            o.strategy(),
            o.dir_mode(),
            o.file_mode(),
            o.file_open_flags()
        ),
        (0, 0, 0, 0)
    );
    assert!(!o.disable_filters() && o.notify_flags() == 0 && o.paths().is_empty());
    assert!(o.baseline().is_none() && o.baseline_index().is_none());
    assert!(o.notify_callback().0.is_none() && o.progress_callback().0.is_none());
    assert!(o.perfdata_callback().0.is_none());
    assert_eq!((o.target_directory(), o.our_label()), (None, None));

    for round in 0..64u32 {
        let mut m = options.as_mut();
        m.set_strategy(round.rotate_left(round));
        m.set_disable_filters(round % 2 == 0);
        m.set_dir_mode(round * 7);
        m.set_file_mode(u32::MAX - round);
        m.set_file_open_flags(i32::MIN + round.cast_signed());
        m.set_notify_flags(!round);
        let o = m.as_ref();
        assert_eq!(o.strategy(), round.rotate_left(round));
        assert_eq!(o.disable_filters(), round % 2 == 0);
        assert_eq!((o.dir_mode(), o.file_mode()), (round * 7, u32::MAX - round));
        assert_eq!(
            (o.file_open_flags(), o.notify_flags()),
            (i32::MIN + round.cast_signed(), !round)
        );
        assert_eq!(o.version(), 1);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn borrowed_strings_and_paths_from_every_source() {
    let owned_strs = strings(5);
    let owned = StrArray::from_strs(&owned_strs).unwrap();
    let view_strs = strings(3);
    let view = StrArrayView::new(&view_strs);
    let empty = StrArrayView::default();
    let labels = strings(8);
    let mut options = CheckoutOptions::new();
    for round in 0..16 {
        let mut m = options.as_mut();
        match round % 4 {
            0 => m.set_paths(owned.as_ref()),
            1 => m.set_paths(view.as_ref()),
            2 => m.set_paths(empty.as_ref()),
            _ => m.clear_paths(),
        }
        set_strings(m.as_mut(), &labels[round % 4..]);
        if round % 3 == 0 {
            m.set_our_label(Some(c"static"));
            m.set_target_directory(None);
        }
        let got = read_all(options.as_ref());
        let paths: Vec<_> = options
            .as_ref()
            .paths()
            .iter()
            .map(CStr::to_bytes)
            .collect();
        match round % 4 {
            0 => assert_eq!(
                paths,
                owned_strs.iter().map(CString::as_bytes).collect::<Vec<_>>()
            ),
            1 => assert_eq!(
                paths,
                view_strs.iter().map(CString::as_bytes).collect::<Vec<_>>()
            ),
            _ => assert!(paths.is_empty(), "{paths:?}"),
        }
        assert!(got.len() >= 5);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn borrowed_sources_are_released_after_the_options() {
    let strs = strings(4);
    let mut owned = StrArray::from_strs(&strs).unwrap();
    let label = CString::new("mine").unwrap();
    {
        let mut options = CheckoutOptions::new();
        options.as_mut().set_paths(owned.as_ref());
        options.as_mut().set_our_label(Some(&label));
        assert_eq!(options.as_ref().paths().len(), 4);
    }
    // The options are gone: the array may change, and every source be released, once.
    owned.as_mut().truncate(1);
    assert_eq!(owned.as_ref().len(), 1);
    drop(owned);
    drop(label);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn getters_outlive_the_handles_and_the_options() {
    let strs = strings(4);
    let view = StrArrayView::new(&strs);
    let (label, paths) = {
        let mut options = CheckoutOptions::new();
        set_strings(options.as_mut(), &strs);
        options.as_mut().set_paths(view.as_ref());
        let label = options.as_ref().their_label();
        // The array header lives in the options, so the strings are read here.
        let paths: Vec<Vec<u8>> = options
            .as_ref()
            .paths()
            .iter()
            .map(|s| s.to_bytes().to_vec())
            .collect();
        (label, paths)
    };
    assert_eq!(label, Some(strs[3].as_c_str()));
    assert_eq!(paths.len(), 4);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reborrows_keep_the_retained_pointers_valid() {
    let strs = strings(6);
    let view = StrArrayView::new(&strs[..3]);
    let mut options = CheckoutOptions::new();
    let mut outer = options.as_mut();
    outer.set_paths(view.as_ref());
    set_strings(outer.as_mut(), &strs[2..]);
    for _ in 0..8 {
        let mut inner = outer.as_mut();
        let before = read_all(inner.as_ref());
        // Rust reads the sources the options point at, then writes through a reborrow.
        assert_eq!(strs[2].as_bytes(), before[3]);
        inner.as_mut().set_strategy(1);
        assert_eq!(read_all(inner.as_ref())[..7], before[..7]);
        let shared = outer.as_ref();
        let again = shared;
        assert_eq!(read_all(again), read_all(shared));
    }
    assert_eq!(options.as_ref().paths().len(), 3);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn options_on_threads_while_libgit2_initialises() {
    isolated(
        concat!(
            module_path!(),
            "::options_on_threads_while_libgit2_initialises"
        ),
        || {
            const THREADS: usize = 4;
            let shared = strings(4);
            let start = Barrier::new(THREADS + 1);
            thread::scope(|s| {
                s.spawn(|| {
                    start.wait();
                    let runtime = global::init().unwrap();
                    let owned = StrArray::from_strs(&shared).unwrap();
                    assert_eq!(owned.as_ref().len(), 4);
                    drop(owned);
                    drop(runtime);
                });
                for t in 0..THREADS {
                    let (shared, start) = (&shared, &start);
                    s.spawn(move || {
                        start.wait();
                        let view = StrArrayView::new(shared);
                        for round in 0..32 {
                            let mut options = CheckoutOptions::new();
                            let mut m = options.as_mut();
                            m.set_strategy(u32::try_from(t * round).unwrap());
                            m.set_paths(view.as_ref());
                            set_strings(m.as_mut(), shared);
                            assert_eq!(read_all(options.as_ref())[..4].len(), 4);
                        }
                    });
                }
            });
        },
    );
}

fn perfdata_counts(r: CheckoutPerfdataRef<'_>) -> (usize, usize, usize) {
    (r.mkdir_calls(), r.stat_calls(), r.chmod_calls())
}

fn bump(mut m: CheckoutPerfdataMut<'_>) {
    let r = m.as_ref();
    let (mkdir, stat, chmod) = perfdata_counts(r);
    m.set_mkdir_calls(mkdir + 1);
    m.set_stat_calls(stat + 1);
    m.set_chmod_calls(chmod + 1);
}

#[test]
fn perfdata_handles_reborrow_and_read_their_writes() {
    let mut d = CheckoutPerfdata::zeroed();
    let mut m = d.as_mut();
    bump(m.as_mut());
    assert_eq!(perfdata_counts(m.as_ref()), (1, 1, 1));
    bump(m.as_mut());
    m.set_stat_calls(usize::MAX);
    let shared = m.as_ref();
    let again = shared;
    assert_eq!(perfdata_counts(again), (2, usize::MAX, 2));
    assert_eq!(perfdata_counts(d.as_ref()), (2, usize::MAX, 2));
    let copy = d.as_ref().copied();
    d.as_mut().set_stat_calls(5);
    bump(d.as_mut());
    assert_eq!(perfdata_counts(copy.as_ref()), (2, usize::MAX, 2));
    assert_eq!(perfdata_counts(d.as_ref()), (3, 6, 3));
}

#[test]
fn perfdata_copies_cross_threads_and_shared_values_are_read_concurrently() {
    let mut d = CheckoutPerfdata::default();
    d.as_mut().set_mkdir_calls(7);
    d.as_mut().set_stat_calls(11);
    let shared = &d;
    let handle = d.as_ref();
    let barrier = Barrier::new(4);
    thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| {
                barrier.wait();
                for _ in 0..100 {
                    assert_eq!(perfdata_counts(shared.as_ref()), (7, 11, 0));
                }
            });
            s.spawn(|| {
                barrier.wait();
                for _ in 0..100 {
                    assert_eq!(perfdata_counts(handle), (7, 11, 0));
                }
            });
        }
    });
    let moved = thread::spawn(move || {
        let mut d = d;
        bump(d.as_mut());
        d
    })
    .join()
    .unwrap();
    assert_eq!(perfdata_counts(moved.as_ref()), (8, 12, 1));
    assert_eq!(
        perfdata_counts(d.as_ref()),
        (7, 11, 0),
        "the thread moved a copy"
    );
    let mut exclusive = moved;
    let m = exclusive.as_mut();
    thread::scope(|s| {
        s.spawn(move || bump(m));
    });
    assert_eq!(perfdata_counts(exclusive.as_ref()), (9, 13, 2));
}

/// The `ProgressCallback` tests: it is called through its `extern "C"` trampoline, as
/// libgit2 calls it, and lends its closure each path for the call only. The paths come
/// from allocations of exactly their length, released right after the call (`ASan`), and
/// the closures copy what they keep. A panic never crosses the trampoline: it resumes from
/// `call`, after which the callback runs its closure again. Callbacks run on several
/// threads at once, each over its own closure and sharing only a lock (`TSan`). Escaping
/// paths, a closure read while lent, and a callback sent to another thread are in
/// `compile_fail/`. None of this reaches C, so Miri runs it.
mod progress_callback {
    use std::cell::Cell;
    use std::ffi::{CStr, CString};
    use std::panic::{self, AssertUnwindSafe};
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::thread;

    use libgit2::libgit2::src::libgit2::checkout::ProgressCallback;

    type Seen = Vec<(Option<CString>, usize, usize)>;

    /// A type-erased progress closure.
    type DynProgress<'a> = dyn FnMut(Option<&CStr>, usize, usize) + 'a;

    fn record(seen: &mut Seen) -> impl FnMut(Option<&CStr>, usize, usize) + '_ {
        |path, completed, total| seen.push((path.map(CStr::to_owned), completed, total))
    }

    #[test]
    fn paths_from_every_source_are_lent_for_the_call() {
        let mut seen = Seen::new();
        let mut f = record(&mut seen);
        let mut callback = ProgressCallback::new(&mut f);
        callback.call(None, 0, 4);
        // A heap string of exactly its length, freed right after the call.
        let owned = CString::new("dir/a.txt").unwrap();
        callback.call(Some(&owned), 1, 4);
        drop(owned);
        // A string inside a larger buffer, and a static one.
        let buffer = b"x/b.txt\0trailing".to_vec();
        callback.call(Some(CStr::from_bytes_until_nul(&buffer).unwrap()), 2, 4);
        drop(buffer);
        callback.call(Some(c""), 3, 4);
        callback.call(Some(c"\xff\xfe"), 4, 4);
        drop(callback);
        drop(f);
        assert_eq!(seen.len(), 5);
        assert_eq!(seen[1].0.as_deref(), Some(c"dir/a.txt"));
        assert_eq!(seen[2].0.as_deref(), Some(c"x/b.txt"));
        assert_eq!(seen[4], (Some(c"\xff\xfe".to_owned()), 4, 4));
    }

    #[test]
    fn every_closure_holder() {
        fn plain(_: Option<&CStr>, completed: usize, total: usize) {
            assert!(completed <= total);
        }
        let mut item = plain;
        ProgressCallback::new(&mut item).call(None, 0, 1);

        let mut count = 0;
        let mut boxed: Box<DynProgress<'_>> = Box::new(|_, _, _| count += 1);
        let mut callback = ProgressCallback::new(&mut boxed);
        callback.call(None, 0, 2);
        callback.call(Some(c"a"), 1, 2);
        drop(callback);
        drop(boxed);
        assert_eq!(count, 2);

        // A closure owning its state, lent to the callback and called again after it.
        let len = Rc::new(Cell::new(0));
        let mut owned = {
            let len = Rc::clone(&len);
            let mut paths = Vec::new();
            move |path: Option<&CStr>, _, _| {
                paths.extend(path.map(CStr::to_owned));
                len.set(paths.len());
            }
        };
        let mut callback = ProgressCallback::new(&mut owned);
        callback.call(Some(c"one"), 1, 1);
        drop(callback);
        owned(Some(c"two"), 2, 2);
        drop(owned);
        assert_eq!((len.get(), Rc::strong_count(&len)), (2, 1));
    }

    #[test]
    fn a_closure_calls_another_callback_reentrantly() {
        let mut inner_seen = Seen::new();
        let mut outer_calls = 0;
        {
            let mut inner = record(&mut inner_seen);
            let mut outer = |path: Option<&CStr>, completed: usize, total: usize| {
                outer_calls += 1;
                // A callback within a callback, over the outer call's lent path.
                let mut nested = ProgressCallback::new(&mut inner);
                nested.call(path, completed * 10, total * 10);
            };
            let mut callback = ProgressCallback::new(&mut outer);
            callback.call(None, 0, 1);
            let path = CString::new("nested").unwrap();
            callback.call(Some(&path), 1, 1);
        }
        assert_eq!(outer_calls, 2);
        assert_eq!(
            inner_seen,
            [(None, 0, 10), (Some(c"nested".to_owned()), 10, 10)]
        );
    }

    #[test]
    fn a_panic_resumes_from_call_and_the_callback_runs_again() {
        let mut calls = Vec::new();
        let mut f = |path: Option<&CStr>, completed: usize, _: usize| {
            calls.push(completed);
            assert_ne!(completed, 1, "at {:?}", path.map(CStr::to_owned));
        };
        let mut callback = ProgressCallback::new(&mut f);
        callback.call(None, 0, 2);
        let path = CString::new("p").unwrap();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(Some(&path), 1, 2)));
        drop(path);
        // The closure's own panic payload, its message included, comes back unchanged.
        let message = caught.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("at Some(\"p\")"), "{message}");
        assert_eq!(
            format!("{callback:?}"),
            "ProgressCallback { panicked: false, .. }"
        );
        callback.call(None, 2, 2);
        drop(callback);
        assert_eq!(calls, [0, 1, 2]);
    }

    #[test]
    fn callbacks_on_threads_share_only_a_lock() {
        let shared = Mutex::new(Vec::new());
        thread::scope(|s| {
            for t in 0..4_usize {
                let shared = &shared;
                s.spawn(move || {
                    let mut local = 0;
                    let mut f = |path: Option<&CStr>, completed: usize, _: usize| {
                        local += completed;
                        let path = path.map(CStr::to_owned);
                        shared.lock().unwrap().push((t, path, completed));
                    };
                    let mut callback = ProgressCallback::new(&mut f);
                    for step in 0..50 {
                        let path = CString::new(format!("t{t}/{step}")).unwrap();
                        callback.call(Some(&path), step, 50);
                    }
                    drop(callback);
                    assert_eq!(local, (0..50).sum::<usize>());
                });
            }
        });
        let shared = shared.into_inner().unwrap();
        assert_eq!(shared.len(), 200);
        assert!(
            shared
                .iter()
                .all(|(t, p, c)| p.as_deref().unwrap().to_str().unwrap() == format!("t{t}/{c}"))
        );
    }
}

/// The `NotifyCallback` tests: it is called through its `extern "C"` trampoline, as
/// libgit2 calls it, and lends its closure each notification for the call only. The sides
/// are diff files libgit2 parsed from a patch, freed right after the calls, and the paths
/// allocations of exactly their length, freed after theirs (`ASan`); the closures copy
/// what they keep. A break comes back as a cancel, and a panic never crosses the
/// trampoline: it resumes from `call`, after which the callback runs its closure again.
/// Callbacks run on several threads at once, each over its own closure and sharing only a
/// lock (`TSan`). Escaping notifications, a closure read while lent, and a callback sent
/// to another thread are in `compile_fail/`. Only the parsed sides reach C, so Miri runs
/// the rest.
mod notify_callback {
    use std::ffi::{CStr, CString};
    use std::ops::ControlFlow;
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::Mutex;
    use std::thread;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::checkout::{
        CheckoutNotification, CheckoutPerfdata, CheckoutPerfdataRef, NotifyCallback,
        PerfdataCallback,
    };
    use libgit2::libgit2::src::libgit2::diff::{Diff, DiffFileRef};

    const CONFLICT: u32 = ffi::GIT_CHECKOUT_NOTIFY_CONFLICT;
    const UPDATED: u32 = ffi::GIT_CHECKOUT_NOTIFY_UPDATED;

    /// A modification of `file`, then a rename of `old` to `new`.
    const PATCH: &[u8] = b"diff --git a/file b/file\nindex 1111111..2222222 100644\n\
        --- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n\
        diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";

    /// A notification as owned data: the reason, the path and each side's path.
    type Seen = (u32, Option<CString>, [Option<CString>; 3]);

    fn seen(n: CheckoutNotification<'_>) -> Seen {
        let side = |f: Option<DiffFileRef<'_>>| f.and_then(|f| f.path()).map(CStr::to_owned);
        let sides = [side(n.baseline), side(n.target), side(n.workdir)];
        (n.why, n.path.map(CStr::to_owned), sides)
    }

    fn bare(why: u32, path: Option<&CStr>) -> CheckoutNotification<'_> {
        CheckoutNotification {
            why,
            path,
            baseline: None,
            target: None,
            workdir: None,
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn sides_libgit2_owns_are_lent_for_the_call() {
        let mut log = Vec::new();
        let mut record = |n: CheckoutNotification<'_>| {
            log.push(seen(n));
            ControlFlow::Continue(())
        };
        let mut callback = NotifyCallback::new(&mut record);
        let input = PATCH.to_vec();
        let diff = Diff::from_buffer(&input).unwrap();
        drop(input);
        for i in 0..diff.as_ref().len() {
            let delta = diff.as_ref().get(i).unwrap();
            let (old, new) = (delta.old_file(), delta.new_file());
            // As libgit2 lends them: the baseline and target sides, the old file again
            // as the workdir side, then no side at all.
            let full = CheckoutNotification {
                why: CONFLICT,
                path: old.path(),
                baseline: Some(old),
                target: Some(new),
                workdir: Some(old),
            };
            assert_eq!(callback.call(full), ControlFlow::Continue(()));
            assert_eq!(
                callback.call(bare(UPDATED, new.path())),
                ControlFlow::Continue(())
            );
        }
        drop(diff);
        drop(callback);
        let some = |s: &CStr| Some(s.to_owned());
        assert_eq!(
            log,
            [
                (
                    CONFLICT,
                    some(c"file"),
                    [some(c"file"), some(c"file"), some(c"file")]
                ),
                (UPDATED, some(c"file"), [None, None, None]),
                (
                    CONFLICT,
                    some(c"old"),
                    [some(c"old"), some(c"new"), some(c"old")]
                ),
                (UPDATED, some(c"new"), [None, None, None]),
            ]
        );
    }

    #[test]
    fn paths_from_every_source_are_lent_for_the_call() {
        let mut log = Vec::new();
        let mut record = |n: CheckoutNotification<'_>| {
            log.push(seen(n));
            ControlFlow::Continue(())
        };
        let mut callback = NotifyCallback::new(&mut record);
        // A heap string of exactly its length, freed right after the call.
        let owned = CString::new("dir/a.txt").unwrap();
        assert!(callback.call(bare(UPDATED, Some(&owned))).is_continue());
        drop(owned);
        // A string inside a larger buffer, a static one, and none.
        let buffer = b"x/b.txt\0trailing".to_vec();
        let inner = CStr::from_bytes_until_nul(&buffer).unwrap();
        assert!(callback.call(bare(UPDATED, Some(inner))).is_continue());
        drop(buffer);
        assert!(callback.call(bare(0, Some(c"\xff\xfe"))).is_continue());
        assert!(callback.call(bare(u32::MAX, None)).is_continue());
        drop(callback);
        let paths: Vec<_> = log.into_iter().map(|(why, path, _)| (why, path)).collect();
        assert_eq!(
            paths,
            [
                (UPDATED, Some(c"dir/a.txt".to_owned())),
                (UPDATED, Some(c"x/b.txt".to_owned())),
                (0, Some(c"\xff\xfe".to_owned())),
                (u32::MAX, None),
            ]
        );
    }

    #[test]
    fn every_closure_holder_and_its_verdict() {
        fn refuse_conflicts(n: CheckoutNotification<'_>) -> ControlFlow<()> {
            if n.why == CONFLICT {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
        let mut item = refuse_conflicts;
        let mut callback = NotifyCallback::new(&mut item);
        assert!(callback.call(bare(CONFLICT, None)).is_break());
        assert!(callback.call(bare(UPDATED, None)).is_continue());

        let mut count = 0;
        let mut boxed: Box<dyn FnMut(CheckoutNotification<'_>) -> ControlFlow<()> + '_> =
            Box::new(|_| {
                count += 1;
                ControlFlow::Break(())
            });
        let mut callback = NotifyCallback::new(&mut boxed);
        assert!(callback.call(bare(UPDATED, None)).is_break());
        // A break is no panic: the next call runs the closure again.
        assert!(callback.call(bare(UPDATED, None)).is_break());
        drop(callback);
        drop(boxed);
        assert_eq!(count, 2);

        // A closure owning its state, lent to the callback and called again after it.
        let mut owned = {
            let mut paths = Vec::new();
            move |n: CheckoutNotification<'_>| {
                paths.extend(n.path.map(CStr::to_owned));
                if paths.len() < 2 {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(())
                }
            }
        };
        let mut callback = NotifyCallback::new(&mut owned);
        assert!(callback.call(bare(UPDATED, Some(c"one"))).is_continue());
        drop(callback);
        assert!(owned(bare(UPDATED, Some(c"two"))).is_break());
    }

    #[test]
    fn a_closure_calls_other_callbacks_reentrantly() {
        let mut inner_log = Vec::new();
        let mut counts = 0;
        {
            let mut inner = |n: CheckoutNotification<'_>| {
                inner_log.push(seen(n));
                ControlFlow::Break(())
            };
            let mut perfdata = |_: CheckoutPerfdataRef<'_>| {
                counts += 1;
            };
            let mut outer = |n: CheckoutNotification<'_>| {
                // A callback within a callback, over the outer call's lent notification.
                let verdict = NotifyCallback::new(&mut inner).call(n);
                let zero = CheckoutPerfdata::default();
                PerfdataCallback::new(&mut perfdata).call(zero.as_ref());
                verdict
            };
            let mut callback = NotifyCallback::new(&mut outer);
            let path = CString::new("nested").unwrap();
            assert!(callback.call(bare(CONFLICT, Some(&path))).is_break());
        }
        assert_eq!(
            inner_log,
            [(CONFLICT, Some(c"nested".to_owned()), [None, None, None])]
        );
        assert_eq!(counts, 1);
    }

    #[test]
    fn a_panic_resumes_from_call_and_the_callback_runs_again() {
        let mut calls = Vec::new();
        let mut f = |n: CheckoutNotification<'_>| {
            calls.push(n.why);
            assert_ne!(n.why, CONFLICT, "at {:?}", n.path.map(CStr::to_owned));
            ControlFlow::Continue(())
        };
        let mut callback = NotifyCallback::new(&mut f);
        assert!(callback.call(bare(UPDATED, None)).is_continue());
        let path = CString::new("p").unwrap();
        let caught = panic::catch_unwind(AssertUnwindSafe(|| {
            callback.call(bare(CONFLICT, Some(&path)))
        }));
        drop(path);
        // The closure's own panic payload, its message included, comes back unchanged.
        let message = caught.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("at Some(\"p\")"), "{message}");
        assert_eq!(
            format!("{callback:?}"),
            "NotifyCallback { panicked: false, .. }"
        );
        assert!(callback.call(bare(UPDATED, None)).is_continue());
        drop(callback);
        assert_eq!(calls, [UPDATED, CONFLICT, UPDATED]);
    }

    #[test]
    fn callbacks_on_threads_share_only_a_lock() {
        let shared = Mutex::new(Vec::new());
        thread::scope(|s| {
            for t in 0..4_u32 {
                let shared = &shared;
                s.spawn(move || {
                    let mut breaks = 0;
                    let mut f = |n: CheckoutNotification<'_>| {
                        let path = n.path.map(CStr::to_owned);
                        shared.lock().unwrap().push((t, path, n.why));
                        if n.why.is_multiple_of(2) {
                            ControlFlow::Continue(())
                        } else {
                            breaks += 1;
                            ControlFlow::Break(())
                        }
                    };
                    let mut callback = NotifyCallback::new(&mut f);
                    for why in 0..50 {
                        let path = CString::new(format!("t{t}/{why}")).unwrap();
                        let verdict = callback.call(bare(why, Some(&path)));
                        assert_eq!(verdict.is_break(), !why.is_multiple_of(2));
                    }
                    drop(callback);
                    assert_eq!(breaks, 25);
                });
            }
        });
        let shared = shared.into_inner().unwrap();
        assert_eq!(shared.len(), 200);
        assert!(
            shared
                .iter()
                .all(|(t, p, w)| p.as_deref().unwrap().to_str().unwrap() == format!("t{t}/{w}"))
        );
    }
}

/// The `PerfdataCallback` tests: it is called through its `extern "C"` trampoline, as
/// libgit2 calls it, and lends its closure the counters for the call only: those of Rust
/// values, of copies and of exclusive handles (Miri, `BSan`). A panic never crosses the
/// trampoline and resumes from `call`. Callbacks run on several threads at once (`TSan`).
/// Escaping counters and a callback sent to another thread are in `compile_fail/`. None of
/// this reaches C, so Miri runs it.
mod perfdata_callback {
    use std::panic::{self, AssertUnwindSafe};
    use std::sync::Mutex;
    use std::thread;

    use libgit2::libgit2::src::libgit2::checkout::{
        CheckoutPerfdata, CheckoutPerfdataRef, PerfdataCallback,
    };

    fn counts(mkdir: usize, stat: usize, chmod: usize) -> CheckoutPerfdata {
        let mut counts = CheckoutPerfdata::default();
        let mut m = counts.as_mut();
        m.set_mkdir_calls(mkdir);
        m.set_stat_calls(stat);
        m.set_chmod_calls(chmod);
        counts
    }

    #[test]
    fn counters_from_every_holder_are_lent_for_the_call() {
        let mut seen = Vec::new();
        let mut record = |d: CheckoutPerfdataRef<'_>| seen.push(d.copied());
        let mut callback = PerfdataCallback::new(&mut record);
        let mut value = counts(1, 2, 3);
        callback.call(value.as_ref());
        // Through an exclusive handle's shared reborrow, then a write, then again.
        {
            let mut exclusive = value.as_mut();
            callback.call(exclusive.as_ref());
            exclusive.set_stat_calls(usize::MAX);
            callback.call(exclusive.as_ref());
        }
        // A boxed copy, freed right after the call.
        let boxed = Box::new(value.as_ref().copied());
        callback.call(boxed.as_ref().as_ref());
        drop(boxed);
        drop(callback);
        assert_eq!(
            seen,
            [
                counts(1, 2, 3),
                counts(1, 2, 3),
                counts(1, usize::MAX, 3),
                counts(1, usize::MAX, 3)
            ]
        );
    }

    #[test]
    fn every_closure_holder() {
        fn check(d: CheckoutPerfdataRef<'_>) {
            assert!(d.mkdir_calls() <= d.stat_calls());
        }
        let mut item = check;
        PerfdataCallback::new(&mut item).call(counts(1, 2, 0).as_ref());

        let mut total = 0;
        let mut boxed: Box<dyn FnMut(CheckoutPerfdataRef<'_>) + '_> =
            Box::new(|d| total += d.stat_calls());
        let mut callback = PerfdataCallback::new(&mut boxed);
        callback.call(counts(0, 5, 0).as_ref());
        callback.call(counts(0, 7, 0).as_ref());
        drop(callback);
        drop(boxed);
        assert_eq!(total, 12);
    }

    #[test]
    fn a_panic_resumes_from_call_and_the_callback_runs_again() {
        let mut calls = 0;
        let mut f = |d: CheckoutPerfdataRef<'_>| {
            calls += 1;
            assert_ne!(d.chmod_calls(), 1, "chmod");
        };
        let mut callback = PerfdataCallback::new(&mut f);
        let one = counts(0, 0, 1);
        let caught = panic::catch_unwind(AssertUnwindSafe(|| callback.call(one.as_ref())));
        let message = caught.unwrap_err().downcast::<String>().unwrap();
        assert!(message.contains("chmod"), "{message}");
        assert_eq!(
            format!("{callback:?}"),
            "PerfdataCallback { panicked: false, .. }"
        );
        callback.call(counts(0, 0, 2).as_ref());
        drop(callback);
        assert_eq!(calls, 2);
    }

    #[test]
    fn callbacks_on_threads_share_only_a_lock() {
        let shared = Mutex::new(0_usize);
        let value = counts(1, 2, 3);
        thread::scope(|s| {
            for _ in 0..4 {
                let (shared, value) = (&shared, &value);
                s.spawn(move || {
                    let mut f = |d: CheckoutPerfdataRef<'_>| {
                        *shared.lock().unwrap() += d.stat_calls();
                    };
                    let mut callback = PerfdataCallback::new(&mut f);
                    for _ in 0..50 {
                        // A value shared by every thread, lent to each call.
                        callback.call(value.as_ref());
                    }
                });
            }
        });
        assert_eq!(shared.into_inner().unwrap(), 4 * 50 * 2);
    }
}
