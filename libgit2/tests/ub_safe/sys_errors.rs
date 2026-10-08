//! `git2/sys/errors.h`: the calling thread's last error, set and cleared through the
//! safe wrappers.
//!
//! libgit2 looks the thread's error state up under a TLS key that only exists while
//! it is initialised, so each wrapper is also tried as a process's first contact with
//! libgit2, where ASan flags a write through the nonexistent key. Messages are copied
//! into a per-thread buffer: dropping the source, growing the buffer, racing threads
//! and thread exit must neither dangle nor leak.

use std::ffi::{CString, c_int};
use std::sync::Barrier;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::util::sys::errors;

use crate::fixture::{isolated, last_error};

fn class(c: ffi::git_error_t) -> c_int {
    c as c_int
}

fn text(len: usize, seed: u8) -> CString {
    let bytes = (0..len)
        .map(|i| b'a' + (i as u8).wrapping_add(seed) % 26)
        .collect::<Vec<_>>();
    CString::new(bytes).unwrap()
}

fn oom() -> (c_int, Vec<u8>) {
    (class(ffi::GIT_ERROR_NOMEMORY), b"Out of memory".to_vec())
}

fn no_error() -> (c_int, Vec<u8>) {
    (class(ffi::GIT_ERROR_NONE), b"no error".to_vec())
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn clear_works_before_any_init() {
    isolated(
        concat!(module_path!(), "::clear_works_before_any_init"),
        || {
            errors::clear();
            assert_eq!(last_error(), no_error());
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_works_before_any_init() {
    isolated(
        concat!(module_path!(), "::set_works_before_any_init"),
        || {
            errors::set(ffi::GIT_ERROR_INVALID, c"first");
            assert_eq!(
                last_error(),
                (class(ffi::GIT_ERROR_INVALID), b"first".to_vec())
            );
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_without_message_works_before_any_init() {
    isolated(
        concat!(
            module_path!(),
            "::set_without_message_works_before_any_init"
        ),
        || {
            errors::set_without_message(ffi::GIT_ERROR_CONFIG);
            assert_eq!(last_error(), (class(ffi::GIT_ERROR_CONFIG), Vec::new()));
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_str_works_before_any_init() {
    isolated(
        concat!(module_path!(), "::set_str_works_before_any_init"),
        || {
            assert_eq!(errors::set_str(ffi::GIT_ERROR_ODB, c"first"), Ok(()));
            assert_eq!(last_error(), (class(ffi::GIT_ERROR_ODB), b"first".to_vec()));
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_oom_works_before_any_init() {
    isolated(
        concat!(module_path!(), "::set_oom_works_before_any_init"),
        || {
            errors::set_oom();
            assert_eq!(last_error(), oom());
        },
    );
}

/// Threads whose first libgit2 contact races the runtime's pinning.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_contacts_race_to_pin_the_runtime() {
    isolated(
        concat!(module_path!(), "::first_contacts_race_to_pin_the_runtime"),
        || {
            let barrier = Barrier::new(10);
            thread::scope(|s| {
                for t in 0..10 {
                    let barrier = &barrier;
                    s.spawn(move || {
                        barrier.wait();
                        match t % 5 {
                            0 => errors::clear(),
                            1 => errors::set(ffi::GIT_ERROR_NET, c"raced"),
                            2 => errors::set_without_message(ffi::GIT_ERROR_NET),
                            3 => errors::set_str(ffi::GIT_ERROR_NET, c"raced").unwrap(),
                            _ => errors::set_oom(),
                        }
                        let expected = match t % 5 {
                            0 => no_error(),
                            1 | 3 => (class(ffi::GIT_ERROR_NET), b"raced".to_vec()),
                            2 => (class(ffi::GIT_ERROR_NET), Vec::new()),
                            _ => oom(),
                        };
                        assert_eq!(last_error(), expected);
                    });
                }
            });
        },
    );
}

/// `printf` directives in a message are text: interpreted, `%s` and `%n` would read
/// and write through arguments that do not exist.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn format_directives_stay_literal() {
    let message = c"%s%s%s%s %n%n %p %99999999d %";
    errors::set(ffi::GIT_ERROR_INVALID, message);
    assert_eq!(last_error().1, message.to_bytes());
    assert_eq!(errors::set_str(ffi::GIT_ERROR_INVALID, message), Ok(()));
    assert_eq!(last_error().1, message.to_bytes());
    errors::set(ffi::GIT_ERROR_OS, message);
    assert!(last_error().1.starts_with(message.to_bytes()));
}

/// The message is copied: its source may go right after the call, and the buffer
/// grows and shrinks under the messages of every size.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn messages_are_copied_at_every_size() {
    for (i, len) in [0, 1, 15, 16, 17, 255, 4095, 4097, 2, 0]
        .into_iter()
        .enumerate()
    {
        let expected = text(len, i as u8);
        let source = expected.clone();
        errors::set(ffi::GIT_ERROR_TREE, &source);
        drop(source);
        assert_eq!(
            last_error(),
            (class(ffi::GIT_ERROR_TREE), expected.to_bytes().to_vec())
        );

        let source = expected.clone();
        assert_eq!(errors::set_str(ffi::GIT_ERROR_INDEX, &source), Ok(()));
        drop(source);
        assert_eq!(
            last_error(),
            (class(ffi::GIT_ERROR_INDEX), expected.to_bytes().to_vec())
        );
    }
    errors::clear();
    assert_eq!(last_error(), no_error());
}

/// Bytes that are not UTF-8, and classes outside `git_error_t`, are kept as given.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn messages_and_classes_are_kept_verbatim() {
    let message = c"\xff\xfe\x80 bytes\n";
    assert_eq!(errors::set_str(999, message), Ok(()));
    assert_eq!(last_error(), (999, message.to_bytes().to_vec()));
    errors::set(u32::MAX, message);
    assert_eq!(last_error(), (-1, message.to_bytes().to_vec()));
    errors::set_without_message(u32::MAX - 1);
    assert_eq!(last_error(), (-2, Vec::new()));
}

/// Every setter and `clear`, in alternation, on threads that check only their own
/// errors; TSan flags any state the threads share unsynchronised.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_set_and_clear_concurrently() {
    let barrier = Barrier::new(8);
    thread::scope(|s| {
        for t in 0..8u8 {
            let barrier = &barrier;
            s.spawn(move || {
                barrier.wait();
                for i in 0..200usize {
                    let own = text(i % 97 + usize::from(t), t);
                    match i % 5 {
                        0 => {
                            errors::set(ffi::GIT_ERROR_NET, &own);
                            assert_eq!(
                                last_error(),
                                (class(ffi::GIT_ERROR_NET), own.to_bytes().to_vec())
                            );
                        }
                        1 => {
                            errors::set_str(ffi::GIT_ERROR_SSL, &own).unwrap();
                            assert_eq!(
                                last_error(),
                                (class(ffi::GIT_ERROR_SSL), own.to_bytes().to_vec())
                            );
                        }
                        2 => {
                            errors::set_without_message(ffi::GIT_ERROR_SSH);
                            assert_eq!(last_error(), (class(ffi::GIT_ERROR_SSH), Vec::new()));
                        }
                        3 => {
                            errors::set_oom();
                            assert_eq!(last_error(), oom());
                        }
                        _ => {
                            errors::clear();
                            assert_eq!(last_error(), no_error());
                        }
                    }
                }
            });
        }
    });
}

/// A thread that exits holding a message releases its error state with it; LSan
/// flags a leak, ASan a double free.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn thread_exit_releases_the_message() {
    for len in [0, 10, 4096] {
        thread::spawn(move || errors::set(ffi::GIT_ERROR_OBJECT, &text(len, 1)))
            .join()
            .unwrap();
        thread::spawn(move || errors::set_str(ffi::GIT_ERROR_OBJECT, &text(len, 2)))
            .join()
            .unwrap()
            .unwrap();
        thread::spawn(errors::set_oom).join().unwrap();
        thread::spawn(errors::clear).join().unwrap();
    }
}
