//! `git2/sys/errors.h`: the calling thread's last error, set and cleared through the
//! safe wrappers.
//!
//! libgit2 looks the thread's error state up under a TLS key that only exists while
//! it is initialised, so each wrapper is also tried as a process's first contact with
//! libgit2, where `ASan` flags a write through the nonexistent key. Messages are copied
//! into a per-thread buffer: dropping the source, growing the buffer, racing threads
//! and thread exit must neither dangle nor leak.

use std::ffi::{CString, c_int};
use std::sync::Barrier;
use std::thread;

use libgit2::libgit2::src::util::errors;
use libgit2::libgit2::src::util::errors::{ErrorClass, last};

use crate::fixture::isolated;

/// The thread's last error's class and message, through the safe `last`.
fn last_error() -> (c_int, Vec<u8>) {
    let e = last();
    (e.raw_class(), e.message().to_bytes().to_vec())
}

fn class(c: ErrorClass) -> c_int {
    c.raw()
}

fn text(len: usize, seed: u8) -> CString {
    let bytes = (0..len)
        .map(|i| b'a' + i.to_le_bytes()[0].wrapping_add(seed) % 26)
        .collect::<Vec<_>>();
    CString::new(bytes).unwrap()
}

fn oom() -> (c_int, Vec<u8>) {
    (class(ErrorClass::NoMemory), b"Out of memory".to_vec())
}

fn no_error() -> (c_int, Vec<u8>) {
    (class(ErrorClass::None), b"no error".to_vec())
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
            errors::set(ErrorClass::Invalid, c"first");
            assert_eq!(
                last_error(),
                (class(ErrorClass::Invalid), b"first".to_vec())
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
            errors::set_without_message(ErrorClass::Config);
            assert_eq!(last_error(), (class(ErrorClass::Config), Vec::new()));
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_str_works_before_any_init() {
    isolated(
        concat!(module_path!(), "::set_str_works_before_any_init"),
        || {
            assert_eq!(errors::set_str(ErrorClass::Odb, c"first"), Ok(()));
            assert_eq!(last_error(), (class(ErrorClass::Odb), b"first".to_vec()));
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
                            1 => errors::set(ErrorClass::Net, c"raced"),
                            2 => errors::set_without_message(ErrorClass::Net),
                            3 => errors::set_str(ErrorClass::Net, c"raced").unwrap(),
                            _ => errors::set_oom(),
                        }
                        let expected = match t % 5 {
                            0 => no_error(),
                            1 | 3 => (class(ErrorClass::Net), b"raced".to_vec()),
                            2 => (class(ErrorClass::Net), Vec::new()),
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
    errors::set(ErrorClass::Invalid, message);
    assert_eq!(last_error().1, message.to_bytes());
    assert_eq!(errors::set_str(ErrorClass::Invalid, message), Ok(()));
    assert_eq!(last_error().1, message.to_bytes());
    errors::set(ErrorClass::Os, message);
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
        let expected = text(len, u8::try_from(i).unwrap());
        let source = expected.clone();
        errors::set(ErrorClass::Tree, &source);
        drop(source);
        assert_eq!(
            last_error(),
            (class(ErrorClass::Tree), expected.to_bytes().to_vec())
        );

        let source = expected.clone();
        assert_eq!(errors::set_str(ErrorClass::Index, &source), Ok(()));
        drop(source);
        assert_eq!(
            last_error(),
            (class(ErrorClass::Index), expected.to_bytes().to_vec())
        );
    }
    errors::clear();
    assert_eq!(last_error(), no_error());
}

/// Bytes that are not UTF-8, and every class, are kept as given.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn messages_and_classes_are_kept_verbatim() {
    let message = c"\xff\xfe\x80 bytes\n";
    let classes: Vec<ErrorClass> = (0..=36).filter_map(ErrorClass::from_raw).collect();
    assert_eq!(classes.len(), 37);
    for c in classes {
        assert_eq!(errors::set_str(c, message), Ok(()));
        assert_eq!(last_error(), (class(c), message.to_bytes().to_vec()));
        assert_eq!(last().class(), Some(c));
        errors::set(c, message);
        assert_eq!(last().class(), Some(c));
        if c != ErrorClass::Os {
            assert_eq!(last_error(), (class(c), message.to_bytes().to_vec()));
            errors::set_without_message(c);
            assert_eq!(last_error(), (class(c), Vec::new()));
        }
    }
}

/// Every setter and `clear`, in alternation, on threads that check only their own
/// errors; `TSan` flags any state the threads share unsynchronised.
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
                            errors::set(ErrorClass::Net, &own);
                            assert_eq!(
                                last_error(),
                                (class(ErrorClass::Net), own.to_bytes().to_vec())
                            );
                        }
                        1 => {
                            errors::set_str(ErrorClass::Ssl, &own).unwrap();
                            assert_eq!(
                                last_error(),
                                (class(ErrorClass::Ssl), own.to_bytes().to_vec())
                            );
                        }
                        2 => {
                            errors::set_without_message(ErrorClass::Ssh);
                            assert_eq!(last_error(), (class(ErrorClass::Ssh), Vec::new()));
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

/// A thread-local destructor that sets and reads errors while its thread exits: the
/// thread's state is allocated or reused there, and still released with the thread
/// (`LSan` flags a leak, `ASan` a use after free of a state torn down first).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn errors_set_while_a_thread_exits_are_released() {
    struct AtExit(usize);

    impl Drop for AtExit {
        fn drop(&mut self) {
            let message = text(self.0, 7);
            errors::set(ErrorClass::Filesystem, &message);
            assert_eq!(
                last_error(),
                (class(ErrorClass::Filesystem), message.to_bytes().to_vec())
            );
            assert_eq!(errors::set_str(ErrorClass::Worktree, &message), Ok(()));
            assert_eq!(
                last_error(),
                (class(ErrorClass::Worktree), message.to_bytes().to_vec())
            );
            errors::set_oom();
            assert_eq!(last_error(), oom());
            errors::clear();
            assert_eq!(last_error(), no_error());
            errors::set_without_message(ErrorClass::Filesystem);
            assert_eq!(last_error(), (class(ErrorClass::Filesystem), Vec::new()));
        }
    }

    thread_local! {
        static AT_EXIT: std::cell::Cell<Option<AtExit>> = const { std::cell::Cell::new(None) };
    }

    for len in [0, 10, 4096] {
        // A thread whose first contact with libgit2 is its destructor.
        thread::spawn(move || AT_EXIT.with(|c| c.set(Some(AtExit(len)))))
            .join()
            .unwrap();
        // One that already holds an error, of another size, when it exits.
        thread::spawn(move || {
            errors::set(ErrorClass::Object, &text(4096 - len, 3));
            AT_EXIT.with(|c| c.set(Some(AtExit(len))));
        })
        .join()
        .unwrap();
    }
}

/// A thread that exits holding a message releases its error state with it; `LSan`
/// flags a leak, `ASan` a double free.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn thread_exit_releases_the_message() {
    for len in [0, 10, 4096] {
        thread::spawn(move || errors::set(ErrorClass::Object, &text(len, 1)))
            .join()
            .unwrap();
        thread::spawn(move || errors::set_str(ErrorClass::Object, &text(len, 2)))
            .join()
            .unwrap()
            .unwrap();
        thread::spawn(errors::set_oom).join().unwrap();
        thread::spawn(errors::clear).join().unwrap();
    }
}
