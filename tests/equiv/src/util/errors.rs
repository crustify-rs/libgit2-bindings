//! Equivalence of `git2/errors.h`'s enums with the C values libgit2 declares and
//! produces.
//!
//! [`ErrorCode`] and [`ErrorClass`] must name exactly the values of `git_error_code`
//! and `git_error_t`, each with its C value, and classify the codes and classes C
//! hands out: a function's return value, and `git_error_last()->klass` after it,
//! whether C or the safe setters of `git2/sys/errors.h` set the class.
//!
//! [`last`] must read the thread's last error as `git_error_last` does. Each side
//! runs the same sequence of C calls on a fresh thread of its own, reading the error
//! after every step through its own reader, and the two are compared step by step:
//! the class, the message, and `errno` after the read. The Rust side also checks that
//! its read left the thread's error as C sees it.
//!
//! Documented divergence, outside these comparisons: before `git_libgit2_init`, C
//! reports a static "library has not been initialized" error, while [`last`] pins the
//! runtime first and reports the thread's error in it; both agree from then on
//! (`last_agrees_once_it_pins_the_runtime`), and the module's `unit_tests` cover the
//! corrected behaviour.

mod sys;

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::process::Command;
use std::ptr;

use glibc_bindings::libc::csu::errno_loc::{errno, set_errno};
use glibc_bindings::libc::ffi as libc;
use libgit2::ffi;
use libgit2::libgit2::src::util::errors::{
    ErrorClass, ErrorCode, UnnamedErrorClass, UnnamedErrorCode, clear, last, set_str,
};

/// Every `git_error_code` constant, beside the variant that must carry it.
const CODES: [(ffi::git_error_code, ErrorCode); 35] = [
    (ffi::GIT_OK, ErrorCode::Ok),
    (ffi::GIT_ERROR, ErrorCode::Generic),
    (ffi::GIT_ENOTFOUND, ErrorCode::NotFound),
    (ffi::GIT_EEXISTS, ErrorCode::Exists),
    (ffi::GIT_EAMBIGUOUS, ErrorCode::Ambiguous),
    (ffi::GIT_EBUFS, ErrorCode::BufSize),
    (ffi::GIT_EUSER, ErrorCode::User),
    (ffi::GIT_EBAREREPO, ErrorCode::BareRepo),
    (ffi::GIT_EUNBORNBRANCH, ErrorCode::UnbornBranch),
    (ffi::GIT_EUNMERGED, ErrorCode::Unmerged),
    (ffi::GIT_ENONFASTFORWARD, ErrorCode::NonFastForward),
    (ffi::GIT_EINVALIDSPEC, ErrorCode::InvalidSpec),
    (ffi::GIT_ECONFLICT, ErrorCode::Conflict),
    (ffi::GIT_ELOCKED, ErrorCode::Locked),
    (ffi::GIT_EMODIFIED, ErrorCode::Modified),
    (ffi::GIT_EAUTH, ErrorCode::Auth),
    (ffi::GIT_ECERTIFICATE, ErrorCode::Certificate),
    (ffi::GIT_EAPPLIED, ErrorCode::Applied),
    (ffi::GIT_EPEEL, ErrorCode::Peel),
    (ffi::GIT_EEOF, ErrorCode::Eof),
    (ffi::GIT_EINVALID, ErrorCode::Invalid),
    (ffi::GIT_EUNCOMMITTED, ErrorCode::Uncommitted),
    (ffi::GIT_EDIRECTORY, ErrorCode::Directory),
    (ffi::GIT_EMERGECONFLICT, ErrorCode::MergeConflict),
    (ffi::GIT_PASSTHROUGH, ErrorCode::Passthrough),
    (ffi::GIT_ITEROVER, ErrorCode::IterOver),
    (ffi::GIT_RETRY, ErrorCode::Retry),
    (ffi::GIT_EMISMATCH, ErrorCode::Mismatch),
    (ffi::GIT_EINDEXDIRTY, ErrorCode::IndexDirty),
    (ffi::GIT_EAPPLYFAIL, ErrorCode::ApplyFail),
    (ffi::GIT_EOWNER, ErrorCode::Owner),
    (ffi::GIT_TIMEOUT, ErrorCode::Timeout),
    (ffi::GIT_EUNCHANGED, ErrorCode::Unchanged),
    (ffi::GIT_ENOTSUPPORTED, ErrorCode::NotSupported),
    (ffi::GIT_EREADONLY, ErrorCode::ReadOnly),
];

/// Every `git_error_t` constant, beside the variant that must carry it.
const CLASSES: [(ffi::git_error_t, ErrorClass); 37] = [
    (ffi::GIT_ERROR_NONE, ErrorClass::None),
    (ffi::GIT_ERROR_NOMEMORY, ErrorClass::NoMemory),
    (ffi::GIT_ERROR_OS, ErrorClass::Os),
    (ffi::GIT_ERROR_INVALID, ErrorClass::Invalid),
    (ffi::GIT_ERROR_REFERENCE, ErrorClass::Reference),
    (ffi::GIT_ERROR_ZLIB, ErrorClass::Zlib),
    (ffi::GIT_ERROR_REPOSITORY, ErrorClass::Repository),
    (ffi::GIT_ERROR_CONFIG, ErrorClass::Config),
    (ffi::GIT_ERROR_REGEX, ErrorClass::Regex),
    (ffi::GIT_ERROR_ODB, ErrorClass::Odb),
    (ffi::GIT_ERROR_INDEX, ErrorClass::Index),
    (ffi::GIT_ERROR_OBJECT, ErrorClass::Object),
    (ffi::GIT_ERROR_NET, ErrorClass::Net),
    (ffi::GIT_ERROR_TAG, ErrorClass::Tag),
    (ffi::GIT_ERROR_TREE, ErrorClass::Tree),
    (ffi::GIT_ERROR_INDEXER, ErrorClass::Indexer),
    (ffi::GIT_ERROR_SSL, ErrorClass::Ssl),
    (ffi::GIT_ERROR_SUBMODULE, ErrorClass::Submodule),
    (ffi::GIT_ERROR_THREAD, ErrorClass::Thread),
    (ffi::GIT_ERROR_STASH, ErrorClass::Stash),
    (ffi::GIT_ERROR_CHECKOUT, ErrorClass::Checkout),
    (ffi::GIT_ERROR_FETCHHEAD, ErrorClass::FetchHead),
    (ffi::GIT_ERROR_MERGE, ErrorClass::Merge),
    (ffi::GIT_ERROR_SSH, ErrorClass::Ssh),
    (ffi::GIT_ERROR_FILTER, ErrorClass::Filter),
    (ffi::GIT_ERROR_REVERT, ErrorClass::Revert),
    (ffi::GIT_ERROR_CALLBACK, ErrorClass::Callback),
    (ffi::GIT_ERROR_CHERRYPICK, ErrorClass::CherryPick),
    (ffi::GIT_ERROR_DESCRIBE, ErrorClass::Describe),
    (ffi::GIT_ERROR_REBASE, ErrorClass::Rebase),
    (ffi::GIT_ERROR_FILESYSTEM, ErrorClass::Filesystem),
    (ffi::GIT_ERROR_PATCH, ErrorClass::Patch),
    (ffi::GIT_ERROR_WORKTREE, ErrorClass::Worktree),
    (ffi::GIT_ERROR_SHA, ErrorClass::Sha),
    (ffi::GIT_ERROR_HTTP, ErrorClass::Http),
    (ffi::GIT_ERROR_INTERNAL, ErrorClass::Internal),
    (ffi::GIT_ERROR_GRAFTS, ErrorClass::Grafts),
];

/// Every `int` from well below the lowest code to well above the highest class,
/// plus the extremes.
fn probes() -> impl Iterator<Item = c_int> {
    (-128..=128).chain([c_int::MIN, c_int::MIN + 1, c_int::MAX - 1, c_int::MAX])
}

#[test]
fn codes_carry_their_c_values() {
    for (c, code) in CODES {
        assert_eq!(code.raw(), c, "{code:?}");
        assert_eq!(c_int::from(code), c, "{code:?}");
    }
}

#[test]
fn from_raw_names_exactly_the_c_codes() {
    for raw in probes() {
        let c = CODES.iter().find(|(c, _)| *c == raw).map(|&(_, code)| code);
        assert_eq!(ErrorCode::from_raw(raw), c, "{raw}");
        assert_eq!(
            ErrorCode::try_from(raw).map_err(UnnamedErrorCode::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
fn classes_carry_their_c_values() {
    for (c, class) in CLASSES {
        assert_eq!(class.raw(), c.cast_signed(), "{class:?}");
        assert_eq!(c_int::from(class), c.cast_signed(), "{class:?}");
    }
}

#[test]
fn from_raw_names_exactly_the_c_classes() {
    for raw in probes() {
        let c = CLASSES
            .iter()
            .find(|(c, _)| (*c).cast_signed() == raw)
            .map(|&(_, class)| class);
        assert_eq!(ErrorClass::from_raw(raw), c, "{raw}");
        assert_eq!(
            ErrorClass::try_from(raw).map_err(UnnamedErrorClass::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's settings.
fn isolated(test: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `git_error_last()->klass`, the raw class C reports for this thread.
fn last_klass() -> c_int {
    // SAFETY: `git_error_last` never returns NULL; it points at this thread's error
    // state or at a static, either of which stays valid until the next libgit2 call
    // on this thread, after the read.
    let error_last = unsafe { ffi::git_error_last() };
    // SAFETY: as above.
    unsafe { (*error_last).klass }
}

/// Checks that Rust classifies C's return value and last error class as the C
/// constants do.
fn assert_classified(ret: c_int, code: ErrorCode, klass: ffi::git_error_t, class: ErrorClass) {
    assert_eq!(ret, code.raw());
    assert_eq!(ErrorCode::from_raw(ret), Some(code));
    let raw = last_klass();
    assert_eq!(raw, klass.cast_signed());
    assert_eq!(ErrorClass::from_raw(raw), Some(class));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn classifies_what_libgit2_reports() {
    isolated(
        concat!(module_path!(), "::classifies_what_libgit2_reports"),
        || {
            // SAFETY: no arguments; the first reference initialises the runtime.
            assert!(unsafe { ffi::git_libgit2_init() } >= 0);
            // SAFETY: no arguments; clears this thread's error state.
            unsafe { ffi::git_error_clear() };
            assert_eq!(ErrorClass::from_raw(last_klass()), Some(ErrorClass::None));

            // A successful call: GIT_OK, and the error state stays clear.
            let mut size = 0usize;
            // SAFETY: GIT_OPT_GET_MWINDOW_SIZE takes one `size_t *`, written once.
            let ret = unsafe {
                ffi::git_libgit2_opts(ffi::GIT_OPT_GET_MWINDOW_SIZE.cast_signed(), &raw mut size)
            };
            assert_classified(ret, ErrorCode::Ok, ffi::GIT_ERROR_NONE, ErrorClass::None);

            // An unknown option key: GIT_ERROR, class GIT_ERROR_INVALID.
            // SAFETY: an unknown key reads no variadic argument.
            let ret = unsafe { ffi::git_libgit2_opts(0x7FFF) };
            assert_classified(
                ret,
                ErrorCode::Generic,
                ffi::GIT_ERROR_INVALID,
                ErrorClass::Invalid,
            );

            // SAFETY: no arguments.
            unsafe { ffi::git_error_clear() };
            assert_eq!(ErrorClass::from_raw(last_klass()), Some(ErrorClass::None));

            // A certificate file that does not exist: class GIT_ERROR_SSL.
            let missing = CString::new("/nonexistent/crustify/ca.pem").unwrap();
            // SAFETY: GIT_OPT_SET_SSL_CERT_LOCATIONS takes a file and a directory, either
            // of which may be NULL; `missing` outlives the call.
            let ret = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_SSL_CERT_LOCATIONS.cast_signed(),
                    missing.as_ptr(),
                    ptr::null::<c_char>(),
                )
            };
            assert_classified(ret, ErrorCode::Generic, ffi::GIT_ERROR_SSL, ErrorClass::Ssl);

            // An invalid config level: back to GIT_ERROR_INVALID.
            let mut buf = ffi::git_buf::default();
            // SAFETY: GIT_OPT_GET_SEARCH_PATH takes an `int` level and a `git_buf *`; a
            // zeroed buffer owns nothing, so libgit2 only points it at its static empty
            // string before rejecting the level.
            let ret = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_GET_SEARCH_PATH.cast_signed(),
                    999,
                    &raw mut buf,
                )
            };
            assert_classified(
                ret,
                ErrorCode::Generic,
                ffi::GIT_ERROR_INVALID,
                ErrorClass::Invalid,
            );
            // SAFETY: `buf` is an initialised `git_buf` with `reserved == 0`, which
            // `git_buf_dispose` leaves unfreed.
            unsafe { ffi::git_buf_dispose(&raw mut buf) };
        },
    );
}

/// Codes libgit2 produces besides `GIT_OK` and `GIT_ERROR`, from calls that need no
/// repository: a reference name that is not valid (`GIT_EINVALIDSPEC`), a buffer too
/// short for its normalisation (`GIT_EBUFS`), and a repository that does not exist
/// (`GIT_ENOTFOUND`); each with the class the call leaves. The rest of the codes
/// need repository state to reach.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn classifies_codes_libgit2_produces() {
    // SAFETY: no arguments; the first reference initialises the runtime.
    assert!(unsafe { ffi::git_libgit2_init() } >= 0);
    let mut out = [0; 64];
    let normalize = |out: &mut [c_char], name: &CStr| {
        // SAFETY: `out` is writable for its length, which is passed as the buffer
        // size; `name` is NUL-terminated and read during the call only.
        unsafe {
            ffi::git_reference_normalize_name(
                out.as_mut_ptr(),
                out.len(),
                name.as_ptr(),
                ffi::GIT_REFERENCE_FORMAT_NORMAL,
            )
        }
    };

    let ret = normalize(&mut out, c"refs/heads/main");
    assert_eq!(ErrorCode::from_raw(ret), Some(ErrorCode::Ok));
    // SAFETY: a successful normalisation NUL-terminates `out`.
    assert_eq!(unsafe { CStr::from_ptr(out.as_ptr()) }, c"refs/heads/main");

    let ret = normalize(&mut out, c"refs/heads/bad..name");
    assert_classified(
        ret,
        ErrorCode::InvalidSpec,
        ffi::GIT_ERROR_REFERENCE,
        ErrorClass::Reference,
    );

    let ret = normalize(&mut out[..4], c"refs/heads/main");
    assert_classified(
        ret,
        ErrorCode::BufSize,
        ffi::GIT_ERROR_REFERENCE,
        ErrorClass::Reference,
    );

    let missing = c"/nonexistent/crustify/repository";
    // SAFETY: a NULL `out` only checks for the repository; `missing` is
    // NUL-terminated and read during the call only; no ceiling directories.
    let ret = unsafe {
        ffi::git_repository_open_ext(
            ptr::null_mut(),
            missing.as_ptr(),
            ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
            ptr::null(),
        )
    };
    assert_classified(ret, ErrorCode::NotFound, ffi::GIT_ERROR_OS, ErrorClass::Os);
    assert_eq!(
        last().message(),
        c"failed to resolve path '/nonexistent/crustify/repository': No such file or directory"
    );
}

/// Setting each class through `git_error_set_str` with the C constant, and through
/// the safe `set_str` with the variant, leaves the same class on the thread, which
/// classifies back to the variant.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_classes_round_trip_through_libgit2() {
    // SAFETY: no arguments; initialises the runtime for the raw calls below.
    assert!(unsafe { ffi::git_libgit2_init() } >= 0);
    for (c, class) in CLASSES {
        // SAFETY: a NUL-terminated message, copied during the call; any class is
        // stored as given. The runtime is initialised.
        let ret = unsafe { ffi::git_error_set_str(c.cast_signed(), c"class".as_ptr()) };
        assert_eq!(ret, 0);
        let from_c = last_klass();

        set_str(class, c"class").unwrap();
        let from_rust = last_klass();

        assert_eq!(from_c, from_rust, "{class:?}");
        assert_eq!(ErrorClass::from_raw(from_rust), Some(class));
    }
    // An `int` the header does not name is stored as given, and classifies as none.
    // SAFETY: as above.
    let ret = unsafe { ffi::git_error_set_str(1000, c"class".as_ptr()) };
    assert_eq!(ret, 0);
    assert_eq!(last_klass(), 1000);
    assert_eq!(ErrorClass::from_raw(last_klass()), None);
    clear();
    assert_eq!(ErrorClass::from_raw(last_klass()), Some(ErrorClass::None));
}

// --- `git_error_last` -----------------------------------------------------------

/// A step of a sequence, always taken through C, so the two sides differ only in how
/// they read the error it leaves.
#[derive(Clone, Debug)]
enum Step {
    /// Nothing: the thread's error as it stands.
    Read,
    Clear,
    Str(c_int, Vec<u8>),
    /// `git_error_set` of a `GIT_ERROR_OS` error, with `errno` set first.
    Os(c_int, Vec<u8>),
    Oom,
    /// A libgit2 call that fails: an unknown `git_libgit2_opts` key.
    Fail,
    /// A libgit2 call that succeeds, leaving the previous error in place.
    Succeed,
    /// Sets the thread's `errno`, which reading must keep.
    Errno(c_int),
    /// Refuses the thread's next allocation (allocation-failure child only).
    FailNext,
}

#[derive(Clone, Copy)]
enum Side {
    C,
    Rust,
}

/// The class and message a read reports, and `errno` after it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    class: c_int,
    message: Vec<u8>,
    errno: c_int,
}

/// `git_error_last()`'s class and message, copied out.
fn c_last() -> (c_int, Vec<u8>) {
    {
        // SAFETY: `git_error_last` never returns NULL; its message lives until the next
        // libgit2 call on this thread, and is copied out before one.
        let e = unsafe { ffi::git_error_last() };
        // SAFETY: as above.
        let klass = unsafe { (*e).klass };
        // SAFETY: as above.
        let message2 = unsafe { (*e).message };
        // SAFETY: as above.
        unsafe { (klass, CStr::from_ptr(message2).to_bytes().to_vec()) }
    }
}

fn rust_last() -> (c_int, Vec<u8>) {
    let e = last();
    (e.raw_class(), e.message().to_bytes().to_vec())
}

fn c_string(message: &[u8]) -> CString {
    CString::new(message).expect("test messages hold no NUL")
}

fn take(step: &Step) {
    match step {
        Step::Read => {}
        // SAFETY: no arguments.
        Step::Clear => unsafe { ffi::git_error_clear() },
        Step::Str(class, message) => {
            let message = c_string(message);
            // SAFETY: a NUL-terminated string, copied during the call.
            unsafe { ffi::git_error_set_str(*class, message.as_ptr()) };
        }
        Step::Os(errno, message) => {
            let message = c_string(message);
            set_errno(*errno);
            // SAFETY: a `"%s"` format with the one string argument it names.
            unsafe {
                ffi::git_error_set(
                    ffi::GIT_ERROR_OS.cast_signed(),
                    c"%s".as_ptr(),
                    message.as_ptr(),
                );
            };
        }
        // SAFETY: no arguments.
        Step::Oom => unsafe { ffi::git_error_set_oom() },
        Step::Fail => {
            // SAFETY: an unknown key reads no variadic argument.
            assert_eq!(unsafe { ffi::git_libgit2_opts(0x7FFF) }, -1);
        }
        Step::Succeed => {
            let mut size = 0usize;
            // SAFETY: GIT_OPT_GET_MWINDOW_SIZE takes one `size_t *`, written once.
            let ret = unsafe {
                ffi::git_libgit2_opts(ffi::GIT_OPT_GET_MWINDOW_SIZE.cast_signed(), &raw mut size)
            };
            assert_eq!(ret, 0);
        }
        Step::Errno(value) => set_errno(*value),
        Step::FailNext => refuse_next(),
    }
}

/// Runs `steps` on a fresh thread, reading the last error through `side` after each
/// step but [`Step::FailNext`], whose refusal is meant for the next one.
///
/// A new thread's `errno` is unspecified (a spawned thread occasionally starts with
/// `EAGAIN` left by thread setup), so the sequence starts from 0 on both sides.
fn run(side: Side, steps: &[Step]) -> Vec<Option<Seen>> {
    let steps = steps.to_vec();
    std::thread::spawn(move || {
        set_errno(0);
        let mut refusing = false;
        steps
            .iter()
            .map(|step| {
                take(step);
                if let Step::FailNext = step {
                    refusing = true;
                    return None;
                }
                let (class, message) = match side {
                    Side::C => c_last(),
                    Side::Rust => rust_last(),
                };
                let seen = Seen {
                    class,
                    message,
                    errno: errno(),
                };
                // A refused read leaves the state to allocate, which a check would do.
                if let (Side::Rust, false) = (side, std::mem::take(&mut refusing)) {
                    assert_eq!(c_last(), (seen.class, seen.message.clone()), "{step:?}");
                    set_errno(seen.errno);
                }
                Some(seen)
            })
            .collect()
    })
    .join()
    .unwrap()
}

/// Runs `steps` on both sides and compares every step.
fn compare(steps: &[Step]) -> Vec<Option<Seen>> {
    let (c, rust) = (run(Side::C, steps), run(Side::Rust, steps));
    for (i, step) in steps.iter().enumerate() {
        assert_eq!(c[i], rust[i], "step {i}: {step:?}");
    }
    c
}

fn init() {
    // SAFETY: no arguments; refcounted, never released in this process. The C side
    // needs it; `last` pins its own reference.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

fn text(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| b'a' + i.to_le_bytes()[0].wrapping_add(seed) % 26)
        .collect()
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "the signature matches the callbacks it stands in for"
)]
fn expect(class: ffi::git_error_t, message: &[u8], errno: c_int) -> Option<Seen> {
    Some(Seen {
        class: class.cast_signed(),
        message: message.to_vec(),
        errno,
    })
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn last_reads_as_c_does_after_each_step() {
    use Step::*;
    init();
    let long = text(8_192, 4);
    let seen = compare(&[
        Read,
        Str(ffi::GIT_ERROR_ODB.cast_signed(), b"first".to_vec()),
        Read,
        Succeed,
        Fail,
        Errno(libc::EACCES),
        Oom,
        Clear,
        Str(ffi::GIT_ERROR_INDEX.cast_signed(), long.clone()),
        Str(
            ffi::GIT_ERROR_NONE.cast_signed(),
            b"set, yet class none".to_vec(),
        ),
        Str(1000, b"\xff\xfe not UTF-8".to_vec()),
        Str(-1, b"".to_vec()),
        Os(libc::ENOENT, b"open".to_vec()),
        Errno(libc::EINTR),
        Read,
        Clear,
        Clear,
    ]);
    let none = |errno| expect(ffi::GIT_ERROR_NONE, b"no error", errno);
    assert_eq!(seen[0], none(0));
    assert_eq!(seen[1], expect(ffi::GIT_ERROR_ODB, b"first", 0));
    assert_eq!(seen[3], seen[1], "a success leaves the error in place");
    assert_eq!(
        seen[4],
        expect(ffi::GIT_ERROR_INVALID, b"invalid option key", 0)
    );
    assert_eq!(seen[5].as_ref().unwrap().errno, libc::EACCES);
    assert_eq!(
        seen[6],
        expect(ffi::GIT_ERROR_NOMEMORY, b"Out of memory", libc::EACCES)
    );
    assert_eq!(seen[7], none(0), "clear resets errno");
    assert_eq!(seen[8], expect(ffi::GIT_ERROR_INDEX, &long, 0));
    assert_eq!(
        seen[9],
        expect(ffi::GIT_ERROR_NONE, b"set, yet class none", 0)
    );
    assert_eq!(seen[10].as_ref().unwrap().class, 1000);
    assert_eq!(seen[11].as_ref().unwrap().class, -1);
    assert_eq!(
        seen[12],
        expect(ffi::GIT_ERROR_OS, b"open: No such file or directory", 0)
    );
    assert_eq!(
        seen[14],
        expect(
            ffi::GIT_ERROR_OS,
            b"open: No such file or directory",
            libc::EINTR
        )
    );
    assert_eq!(seen[16], none(0));
}

/// The message buffer grows, shrinks and is reallocated under the reads.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn last_reads_messages_of_every_size() {
    init();
    let mut steps = Vec::new();
    for (i, len) in [0, 1, 7, 8, 9, 63, 64, 65, 1000, 4095, 4097, 3, 0]
        .into_iter()
        .enumerate()
    {
        steps.push(Step::Str(
            ffi::GIT_ERROR_TREE.cast_signed(),
            text(len, u8::try_from(i).unwrap()),
        ));
        steps.push(Step::Os(
            libc::ENOTDIR,
            text(len, u8::try_from(i).unwrap() + 1),
        ));
    }
    compare(&steps);
}

/// Each thread reads only its own error, while other threads set theirs.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn last_reads_each_threads_own_error() {
    init();
    let steps: Vec<Step> = (0..64)
        .flat_map(|i| {
            [
                Step::Str(
                    ffi::GIT_ERROR_NET.cast_signed(),
                    text(i * 5, u8::try_from(i).unwrap()),
                ),
                Step::Oom,
                Step::Clear,
            ]
        })
        .collect();
    let expected = run(Side::C, &steps);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let steps = &steps;
                s.spawn(move || run(if t % 2 == 0 { Side::C } else { Side::Rust }, steps))
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), expected);
        }
    });
}

/// Before `git_libgit2_init` C reports its static "not initialized" error, where `last`
/// pins the runtime and reports the thread's; from then on the two agree.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn last_agrees_once_it_pins_the_runtime() {
    isolated(
        concat!(module_path!(), "::last_agrees_once_it_pins_the_runtime"),
        || {
            assert_eq!(
                c_last(),
                (
                    ffi::GIT_ERROR_INVALID.cast_signed(),
                    b"library has not been initialized".to_vec()
                )
            );
            let rust = rust_last();
            assert_eq!(
                rust,
                (ffi::GIT_ERROR_NONE.cast_signed(), b"no error".to_vec())
            );
            assert_eq!(c_last(), rust);
            let seen = compare(&[
                Step::Read,
                Step::Str(ffi::GIT_ERROR_CONFIG.cast_signed(), b"pinned".to_vec()),
                Step::Fail,
            ]);
            assert_eq!(seen[1], expect(ffi::GIT_ERROR_CONFIG, b"pinned", 0));
        },
    );
}

// --- allocation failures --------------------------------------------------------

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

thread_local! {
    /// Refuses this thread's next allocation when set.
    static REFUSE: Cell<bool> = const { Cell::new(false) };
}

fn refuse_next() {
    REFUSE.with(|r| r.set(true));
}

fn refused() -> bool {
    REFUSE.with(|r| r.replace(false))
}

unsafe extern "C" fn refusing_malloc(n: usize, _: *const c_char, _: c_int) -> *mut c_void {
    if refused() {
        ptr::null_mut()
    } else {
        // SAFETY: plain libc allocation.
        unsafe { malloc(n) }
    }
}

unsafe extern "C" fn refusing_realloc(
    p: *mut c_void,
    n: usize,
    _: *const c_char,
    _: c_int,
) -> *mut c_void {
    if refused() {
        ptr::null_mut()
    } else {
        // SAFETY: `p` came from this allocator, i.e. libc, or is NULL.
        unsafe { realloc(p, n) }
    }
}

unsafe extern "C" fn libc_free(p: *mut c_void) {
    // SAFETY: `p` came from this allocator, i.e. libc, or is NULL.
    unsafe { free(p) }
}

/// A thread whose error state cannot be allocated reads libgit2's static thread-state
/// error, and a thread whose message buffer ran out of memory reads the out-of-memory
/// error from then on.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn last_reads_allocation_failures_as_c_does() {
    use Step::*;
    isolated(
        concat!(module_path!(), "::last_reads_allocation_failures_as_c_does"),
        || {
            let mut allocator = ffi::git_allocator {
                gmalloc: Some(refusing_malloc),
                grealloc: Some(refusing_realloc),
                gfree: Some(libc_free),
            };
            // SAFETY: a `git_allocator *` with every member set, which libgit2 copies;
            // nothing is allocated yet, so no storage changes allocator.
            let rc = unsafe {
                ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), &raw mut allocator)
            };
            assert_eq!(rc, 0);
            init();
            // Pin the Rust side's runtime now, so no step below initialises it.
            rust_last();

            // The fresh thread's first read allocates its state, which is refused.
            let seen = compare(&[FailNext, Read, Read]);
            assert_eq!(
                seen[1],
                expect(
                    ffi::GIT_ERROR_THREAD,
                    b"thread-local data initialization failure",
                    0
                )
            );
            assert_eq!(
                seen[2],
                expect(ffi::GIT_ERROR_NONE, b"no error", 0),
                "the next read allocates the state"
            );

            let seen = compare(&[
                Str(ffi::GIT_ERROR_INVALID.cast_signed(), b"warm".to_vec()),
                FailNext,
                Str(ffi::GIT_ERROR_CONFIG.cast_signed(), text(1000, 1)),
                Str(ffi::GIT_ERROR_CONFIG.cast_signed(), b"short".to_vec()),
                Clear,
                Str(ffi::GIT_ERROR_CONFIG.cast_signed(), b"again".to_vec()),
            ]);
            let oom = expect(ffi::GIT_ERROR_NOMEMORY, b"Out of memory", 0);
            assert_eq!((&seen[2], &seen[3]), (&oom, &oom));
            assert_eq!(seen[5], expect(ffi::GIT_ERROR_NONE, b"no error", 0));
        },
    );
}
