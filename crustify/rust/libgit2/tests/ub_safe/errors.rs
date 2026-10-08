//! UB tests for `git2/errors.h`: the enums `ErrorCode` and `ErrorClass`, and `last`.
//!
//! Both enums are fieldless enums over C `int`s. The only safe way in from an `int`
//! is `from_raw` / `try_from`, which must reject every value without a variant rather
//! than produce an invalid discriminant (which Miri and UBSan would flag on the next
//! read); an `as` cast to either enum does not compile (`compile_fail/`). Neither
//! calls libgit2, so those tests also run under Miri.
//!
//! `last` reads the thread's error state, which only exists while libgit2 is
//! initialised, so it is also tried as a process's first contact with libgit2, alone
//! and racing. The message it reads lives in a per-thread buffer that later errors
//! rewrite and reallocate and thread exit frees: the copy it returns must outlive all
//! of that, where ASan flags one aliasing the buffer, and thread states allocated only
//! to be read must not leak (LSan). A message borrowed from the copy cannot outlive
//! it (`compile_fail/`).

use std::collections::HashSet;
use std::ffi::{CString, c_int};
use std::hint::black_box;
use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use libgit2::libgit2::util::sys::errors::{clear, set, set_oom, set_str};

use crate::fixture::isolated;

/// Every `int` around the declared ranges, then a coarse stride across the whole type;
/// BorrowSanitizer instruments each comparison, so the probe set stays small.
fn probes() -> impl Iterator<Item = c_int> {
    (-64..=64)
        .chain((c_int::MIN..=c_int::MAX).step_by(1 << 26))
        .chain([c_int::MIN, c_int::MIN + 1, c_int::MAX - 1, c_int::MAX])
}

#[test]
fn from_raw_yields_only_valid_codes() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(code) = ErrorCode::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(code.raw(), raw);
            assert!(!format!("{code:?}").is_empty());
            seen.insert(code);
        } else {
            assert_eq!(ErrorCode::try_from(raw), Err(raw));
        }
    }
    assert_eq!(seen.len(), 35);
}

#[test]
fn from_raw_yields_only_valid_classes() {
    let mut seen = HashSet::new();
    for raw in probes() {
        if let Some(class) = ErrorClass::from_raw(black_box(raw)) {
            assert_eq!(class.raw(), raw);
            assert!(!format!("{class:?}").is_empty());
            seen.insert(class);
        } else {
            assert_eq!(ErrorClass::try_from(raw), Err(raw));
        }
    }
    assert_eq!(seen.len(), 37);
}

#[test]
fn option_niche_holds_for_every_value() {
    // `Option<ErrorCode>` stores `None` in a value no variant uses: a rejected value
    // must never alias a variant, nor a variant read as `None`.
    for raw in probes() {
        let code = black_box(ErrorCode::from_raw(raw));
        let class = black_box(ErrorClass::from_raw(raw));
        assert_eq!(code.map(ErrorCode::raw).unwrap_or(raw), raw);
        assert_eq!(class.map(ErrorClass::raw).unwrap_or(raw), raw);
    }
}

#[test]
fn enums_cross_threads() {
    // Plain `Copy` data: shared and sent between threads without synchronisation.
    let codes: Vec<ErrorCode> = (-40..=0).filter_map(ErrorCode::from_raw).collect();
    let classes: Vec<ErrorClass> = (0..=36).filter_map(ErrorClass::from_raw).collect();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                for code in &codes {
                    assert_eq!(ErrorCode::from_raw(code.raw()), Some(*code));
                }
                for class in &classes {
                    assert_eq!(ErrorClass::from_raw(class.raw()), Some(*class));
                }
            });
        }
    });
    let moved = thread::spawn(move || (codes.len(), classes.len()))
        .join()
        .unwrap();
    assert_eq!(moved, (35, 37));
}

// --- `last` ---------------------------------------------------------------------

fn text(len: usize, seed: u8) -> CString {
    let bytes = (0..len)
        .map(|i| b'a' + (i as u8).wrapping_add(seed) % 26)
        .collect::<Vec<_>>();
    CString::new(bytes).unwrap()
}

fn is_no_error(e: &ErrorBuf) -> bool {
    e.class() == Some(ErrorClass::None) && e.message() == c"no error"
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn last_works_before_any_init() {
    isolated(
        concat!(module_path!(), "::last_works_before_any_init"),
        || {
            assert!(is_no_error(&last()));
            set_str(ErrorClass::Odb.into(), c"after").unwrap();
            let e = last();
            assert_eq!((e.class(), e.message()), (Some(ErrorClass::Odb), c"after"));
        },
    );
}

/// Threads whose first libgit2 contact is `last` race the runtime's pinning.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_reads_race_to_pin_the_runtime() {
    isolated(
        concat!(module_path!(), "::first_reads_race_to_pin_the_runtime"),
        || {
            let barrier = Barrier::new(10);
            thread::scope(|s| {
                for _ in 0..10 {
                    let barrier = &barrier;
                    s.spawn(move || {
                        barrier.wait();
                        assert!(is_no_error(&last()));
                    });
                }
            });
        },
    );
}

/// Each copy survives the thread's buffer being rewritten, grown, shrunk and
/// reallocated by the errors after it.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_outlive_the_next_errors() {
    let lens = [0, 1, 15, 16, 17, 255, 4095, 4097, 2, 8192, 0];
    let copies: Vec<(CString, ErrorBuf)> = lens
        .iter()
        .enumerate()
        .map(|(i, &len)| {
            let message = text(len, i as u8);
            if i % 2 == 0 {
                set(ErrorClass::Tree.into(), &message);
            } else {
                set_str(ErrorClass::Index.into(), &message).unwrap();
            }
            (message, last())
        })
        .collect();
    set_oom();
    let oom = last();
    clear();
    assert!(is_no_error(&last()));
    for (i, (message, copy)) in copies.iter().enumerate() {
        let class = if i % 2 == 0 {
            ErrorClass::Tree
        } else {
            ErrorClass::Index
        };
        assert_eq!(copy.class(), Some(class));
        assert_eq!(copy.message(), message.as_c_str());
    }
    assert_eq!(
        (oom.class(), oom.message()),
        (Some(ErrorClass::NoMemory), c"Out of memory")
    );
}

/// A copy outlives the thread whose error it is, whose exit frees the buffer.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_outlive_their_thread() {
    for len in [0, 10, 4096] {
        let copy = thread::spawn(move || {
            set_str(ErrorClass::Net.into(), &text(len, 3)).unwrap();
            last()
        })
        .join()
        .unwrap();
        assert_eq!(copy.message(), text(len, 3).as_c_str());
        assert_eq!(copy.class(), Some(ErrorClass::Net));
    }
}

/// A thread that only reads allocates its error state, and frees it on exit; LSan
/// flags a leak, ASan a double free.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn reading_threads_release_their_state() {
    for _ in 0..16 {
        let copy = thread::spawn(last).join().unwrap();
        assert!(is_no_error(&copy));
    }
}

/// Copies are shared and sent between threads that keep setting and reading their own
/// errors; TSan flags any state they share unsynchronised.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn copies_cross_threads_while_they_read() {
    set_str(ErrorClass::Config.into(), c"shared").unwrap();
    let shared = last();
    let barrier = Barrier::new(8);
    let sent: Vec<ErrorBuf> = thread::scope(|s| {
        let handles: Vec<_> = (0..8u8)
            .map(|t| {
                let (shared, barrier) = (&shared, &barrier);
                s.spawn(move || {
                    barrier.wait();
                    for i in 0..200usize {
                        assert_eq!(shared.message(), c"shared");
                        let message = text(i % 97 + usize::from(t), t);
                        match i % 3 {
                            0 => set_str(ErrorClass::Ssl.into(), &message).unwrap(),
                            1 => set_oom(),
                            _ => clear(),
                        }
                        let own = last();
                        match i % 3 {
                            0 => assert_eq!(own.message(), message.as_c_str()),
                            1 => assert_eq!(own.class(), Some(ErrorClass::NoMemory)),
                            _ => assert!(is_no_error(&own)),
                        }
                    }
                    clear();
                    last()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(sent.iter().all(is_no_error));
    assert_eq!(last().message(), c"shared");
}
