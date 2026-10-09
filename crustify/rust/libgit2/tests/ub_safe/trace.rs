//! UB tests for `git2/trace.h`: the enum `TraceLevel`.
//!
//! `TraceLevel` is a fieldless enum over C's `git_trace_level_t`. The only safe way in
//! from a raw value is `from_raw` / `try_from`, which must reject every value without
//! a variant rather than produce an invalid discriminant (which Miri and UBSan would
//! flag on the next read); an `as` cast to it does not compile (`compile_fail/`).
//! None of them calls libgit2, so they also run under Miri.
//!
//! Then the callback, `TraceCallback`, over a `Tracer`. Its message is borrowed for
//! the call, so a tracer that keeps it does not compile (a `compile_fail,E0521`
//! doctest on `Tracer::trace`, whose wording differs between toolchains), and one
//! that copies it leaves nothing behind once the caller frees it (ASan). Calls from
//! many threads at once, and from libgit2's own threads, reach the tracer with nothing
//! but Rust's own synchronisation (TSan); a tracer that calls back into the callback
//! nests; and a panicking one aborts the process instead of unwinding into C. The
//! calls that stay in Rust also run under Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::core::trace::TraceLevel;

/// Every value around the declared ones, then a coarse stride across the whole type;
/// BorrowSanitizer instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = ffi::git_trace_level_t> {
    (0..=64)
        .chain((0..=ffi::git_trace_level_t::MAX).step_by(1 << 26))
        .chain([ffi::git_trace_level_t::MAX - 1, ffi::git_trace_level_t::MAX])
}

#[test]
fn from_raw_yields_only_valid_levels() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(level) = TraceLevel::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(level.raw(), raw);
            assert!(!format!("{level:?}").is_empty());
            seen.insert(level);
        } else {
            assert_eq!(TraceLevel::try_from(raw).map_err(|e| e.raw()), Err(raw));
        }
    }
    assert_eq!(seen.len(), 7);
}

#[test]
fn levels_cross_threads() {
    let levels: Vec<TraceLevel> = (0..=6).filter_map(TraceLevel::from_raw).collect();
    let back = thread::spawn(move || {
        let mut sorted = levels;
        sorted.sort();
        sorted
    })
    .join()
    .unwrap();
    assert_eq!(back.first(), Some(&TraceLevel::None));
    assert_eq!(back.last(), Some(&TraceLevel::Trace));
}

mod callback {
    use std::cell::Cell;
    use std::ffi::{CStr, CString};
    use std::process::Command;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::thread;

    use libgit2::libgit2::core::trace::{TraceCallback, TraceLevel, Tracer};

    use crate::fixture::{connect, isolated, serve_404, set_trace};

    /// Copies every message into one shared log.
    struct Copying;

    static COPIES: Mutex<Vec<(TraceLevel, CString)>> = Mutex::new(Vec::new());

    impl Tracer for Copying {
        fn trace(level: TraceLevel, message: &CStr) {
            COPIES.lock().unwrap().push((level, message.to_owned()));
        }
    }

    /// Counts its calls and the bytes it read, without a lock.
    struct Counting;

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static BYTES: AtomicUsize = AtomicUsize::new(0);

    impl Tracer for Counting {
        fn trace(_: TraceLevel, message: &CStr) {
            CALLS.fetch_add(1, SeqCst);
            BYTES.fetch_add(
                message.to_bytes().iter().map(|&b| usize::from(b > 0)).sum(),
                SeqCst,
            );
        }
    }

    #[test]
    fn freed_messages_leave_copies_intact() {
        let cb = TraceCallback::new::<Copying>();
        let mut want = Vec::new();
        for n in [0usize, 1, 7, 64, 4096] {
            let message = CString::new(vec![b'm'; n]).unwrap();
            cb.call(TraceLevel::Debug, &message);
            drop(message);
            want.push(n);
        }
        let mut log = COPIES.lock().unwrap();
        let mine: Vec<usize> = log
            .iter()
            .filter(|(level, m)| {
                *level == TraceLevel::Debug && m.as_bytes().iter().all(|&b| b == b'm')
            })
            .map(|(_, m)| m.as_bytes().len())
            .collect();
        assert_eq!(mine, want);
        log.clear();
    }

    #[test]
    fn calls_from_many_threads_race_nothing() {
        let cb = TraceCallback::new::<Counting>();
        let threads = if cfg!(any(miri, bsan)) { 3 } else { 8 };
        let per = if cfg!(any(miri, bsan)) { 4 } else { 200 };
        let before = CALLS.load(SeqCst);
        thread::scope(|s| {
            for t in 0..threads {
                s.spawn(move || {
                    for i in 0..per {
                        let message = CString::new(format!("thread {t} message {i}")).unwrap();
                        cb.call(TraceLevel::Trace, &message);
                    }
                });
            }
        });
        assert!(CALLS.load(SeqCst) - before >= threads * per);
        assert!(BYTES.load(SeqCst) > 0);
    }

    thread_local! {
        static DEPTH: Cell<u32> = const { Cell::new(0) };
    }

    /// Calls its own callback again from inside, three levels deep, each time with a
    /// message of its own that dies before the outer one is read again.
    struct Reentrant;

    static NESTED: Mutex<Vec<(u32, CString)>> = Mutex::new(Vec::new());

    impl Tracer for Reentrant {
        fn trace(level: TraceLevel, message: &CStr) {
            let depth = DEPTH.get();
            if depth < 3 {
                DEPTH.set(depth + 1);
                let inner = CString::new(format!("inner {depth}")).unwrap();
                TraceCallback::new::<Self>().call(level, &inner);
                drop(inner);
                DEPTH.set(depth);
            }
            NESTED.lock().unwrap().push((depth, message.to_owned()));
        }
    }

    #[test]
    fn reentrant_calls_nest() {
        let outer = CString::new("outer").unwrap();
        TraceCallback::new::<Reentrant>().call(TraceLevel::Warn, &outer);
        let nested = std::mem::take(&mut *NESTED.lock().unwrap());
        assert_eq!(
            nested,
            [
                (3, c"inner 2".to_owned()),
                (2, c"inner 1".to_owned()),
                (1, c"inner 0".to_owned()),
                (0, outer),
            ]
        );
    }

    /// libgit2 calls the tracer on every thread that connects, at once.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn libgit2_traces_from_several_threads() {
        isolated(
            concat!(module_path!(), "::libgit2_traces_from_several_threads"),
            || {
                let url = serve_404();
                // Untraced: sets up the HTTP client's lazy statics before the threads
                // race to (see `connect`).
                assert!(connect(&url) < 0);
                set_trace(TraceLevel::Trace, Some(TraceCallback::new::<Copying>()));
                let threads = if cfg!(bsan) { 2 } else { 4 };
                thread::scope(|s| {
                    for _ in 0..threads {
                        s.spawn(|| {
                            for _ in 0..3 {
                                assert!(connect(&url) < 0, "the server answers 404");
                            }
                        });
                    }
                });
                set_trace(TraceLevel::None, None);
                let log = std::mem::take(&mut *COPIES.lock().unwrap());
                let connecting = log
                    .iter()
                    .filter(|(level, m)| {
                        *level == TraceLevel::Debug
                            && m.to_bytes().starts_with(b"Connecting to remote 127.0.0.1")
                    })
                    .count();
                assert_eq!(connecting, threads * 3);
                assert!(log.iter().any(|(level, _)| *level == TraceLevel::Trace));
                // Cleared: libgit2 calls nothing any more.
                assert!(connect(&url) < 0);
                assert!(COPIES.lock().unwrap().is_empty());
            },
        );
    }

    struct Panicking;

    impl Tracer for Panicking {
        fn trace(_: TraceLevel, message: &CStr) {
            panic!("tracer refused {message:?}");
        }
    }

    const ABORT_CHILD: &str = "CRUSTIFY_UB_SAFE_ABORT_CHILD";

    /// A panic in the tracer cannot unwind through libgit2's frame: the process aborts.
    #[test]
    #[cfg_attr(miri, ignore = "spawns a child process")]
    #[cfg_attr(bsan, ignore = "the child's abort path spins under BorrowSanitizer")]
    fn panicking_tracer_aborts() {
        let name = concat!(module_path!(), "::panicking_tracer_aborts");
        let name = name.split_once("::").map_or(name, |(_, rest)| rest);
        if std::env::var(ABORT_CHILD).is_ok() {
            let url = serve_404();
            set_trace(TraceLevel::Debug, Some(TraceCallback::new::<Panicking>()));
            connect(&url);
            unreachable!("the trampoline must not return from a panic");
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(ABORT_CHILD, "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{stderr}");
        assert!(stderr.contains("tracer refused"), "{stderr}");
        assert!(!stderr.contains("must not return"), "{stderr}");
    }
}
