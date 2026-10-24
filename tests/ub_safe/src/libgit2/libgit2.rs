//! UB tests for `git2/global.h`: `init`, and the `Runtime` reference it returns.
//!
//! libgit2's initialisation count is process-global, so every test that depends on
//! it reruns alone in a child process, where nothing has initialised libgit2. A
//! `Runtime` releases exactly the reference it took, from any thread, so the count
//! never goes negative, and no safe wrapper's state is torn down under it: wrappers
//! that need libgit2 pin a reference of their own for the rest of the process. The
//! tests take and release references across threads (`TSan`), drop references while
//! pinned objects are live (`ASan`), and cycle libgit2 through full shutdowns.

use std::thread;

use libgit2::libgit2::src::libgit2::libgit2::{Runtime, init};
use libgit2::libgit2::src::libgit2::settings;
use libgit2::libgit2::src::util::errors::last;

use crate::fixture::isolated;

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn runtime_is_send_and_sync() {
    assert_send_sync::<Runtime>();
}

/// References count up from the first, and down to 0, which tears libgit2 down; the
/// next one sets it up again.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn references_count_and_cycle() {
    isolated(
        concat!(module_path!(), "::references_count_and_cycle"),
        || {
            for _ in 0..3 {
                let a = init().unwrap();
                assert_eq!(a.count().get(), 1);
                let b = init().unwrap();
                assert_eq!(b.count().get(), 2);
                assert_eq!(b.shutdown(), Ok(1));
                assert_eq!(a.shutdown(), Ok(0));
            }
            let a = init().unwrap();
            drop(a);
            assert_eq!(init().unwrap().count().get(), 1, "dropping released it");
        },
    );
}

/// A reference taken on one thread is released on another, while other threads take
/// and release their own.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn references_cross_threads() {
    isolated(
        concat!(module_path!(), "::references_cross_threads"),
        || {
            let held: Vec<Runtime> = (0..4).map(|_| init().unwrap()).collect();
            thread::scope(|s| {
                for r in held {
                    s.spawn(move || drop(r));
                }
                for _ in 0..4 {
                    s.spawn(|| {
                        for _ in 0..50 {
                            let r = init().unwrap();
                            assert!(r.count().get() >= 1);
                            let shared = &r;
                            thread::scope(|t| {
                                t.spawn(move || assert!(shared.count().get() >= 1));
                            });
                            drop(r);
                        }
                    });
                }
            });
            assert_eq!(
                init().unwrap().count().get(),
                1,
                "every reference was released"
            );
        },
    );
}

/// Releasing a reference while objects that pinned libgit2 are live leaves them
/// usable: the pin keeps libgit2's state, so the last `Runtime` never tears it down.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn dropping_the_runtime_keeps_pinned_state() {
    isolated(
        concat!(module_path!(), "::dropping_the_runtime_keeps_pinned_state"),
        || {
            let r = init().unwrap();
            assert_eq!(r.count().get(), 1);
            let extensions = settings::extensions().unwrap();
            let agent = settings::user_agent().unwrap();
            assert_eq!(r.shutdown(), Ok(1), "the pin holds the other reference");
            assert!(extensions.as_ref().iter().any(|e| e == c"noop"));
            assert!(agent.to_bytes().starts_with(b"libgit2 "));
            drop(extensions);
            drop(last());
            for _ in 0..3 {
                let r = init().unwrap();
                assert_eq!(r.count().get(), 2);
            }
            assert_eq!(
                settings::template_path().unwrap(),
                settings::template_path().unwrap()
            );
        },
    );
}

/// Threads cycling references race threads pinning libgit2 and reading settings.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn cycling_races_pinning() {
    isolated(concat!(module_path!(), "::cycling_races_pinning"), || {
        thread::scope(|s| {
            for _ in 0..3 {
                s.spawn(|| {
                    for _ in 0..30 {
                        drop(init().unwrap());
                    }
                });
            }
            s.spawn(|| {
                for _ in 0..30 {
                    let _ = settings::mwindow_size();
                    let _ = libgit2::libgit2::src::libgit2::libgit2::version();
                    let _ = libgit2::libgit2::src::libgit2::libgit2::features();
                }
            });
            s.spawn(|| {
                for _ in 0..10 {
                    assert!(settings::search_path(settings::SearchPathLevel::Global).is_ok());
                    drop(last());
                }
            });
        });
    });
}
