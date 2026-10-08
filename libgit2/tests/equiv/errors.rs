//! Equivalence of `git2/errors.h`'s enums with the C values libgit2 declares and
//! produces.
//!
//! [`ErrorCode`] and [`ErrorClass`] must name exactly the values of `git_error_code`
//! and `git_error_t`, each with its C value, and classify the codes and classes C
//! hands out: a function's return value, and `git_error_last()->klass` after it,
//! whether C or the safe setters of `git2/sys/errors.h` set the class.

use std::ffi::{CString, c_int};
use std::process::Command;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::util::errors::{ErrorClass, ErrorCode};
use libgit2::libgit2::util::sys::errors::{clear, set_str};

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
        assert_eq!(ErrorCode::try_from(raw), c.ok_or(raw), "{raw}");
    }
}

#[test]
fn classes_carry_their_c_values() {
    for (c, class) in CLASSES {
        assert_eq!(class.raw(), c as c_int, "{class:?}");
        assert_eq!(c_int::from(class), c as c_int, "{class:?}");
    }
}

#[test]
fn from_raw_names_exactly_the_c_classes() {
    for raw in probes() {
        let c = CLASSES
            .iter()
            .find(|(c, _)| *c as c_int == raw)
            .map(|&(_, class)| class);
        assert_eq!(ErrorClass::from_raw(raw), c, "{raw}");
        assert_eq!(ErrorClass::try_from(raw), c.ok_or(raw), "{raw}");
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
    unsafe { (*ffi::git_error_last()).klass }
}

/// Checks that Rust classifies C's return value and last error class as the C
/// constants do.
fn assert_classified(ret: c_int, code: ErrorCode, klass: ffi::git_error_t, class: ErrorClass) {
    assert_eq!(ret, code.raw());
    assert_eq!(ErrorCode::from_raw(ret), Some(code));
    let raw = last_klass();
    assert_eq!(raw, klass as c_int);
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
                ffi::git_libgit2_opts(ffi::GIT_OPT_GET_MWINDOW_SIZE as c_int, &raw mut size)
            };
            assert_classified(ret, ErrorCode::Ok, ffi::GIT_ERROR_NONE, ErrorClass::None);

            // An unknown option key: GIT_ERROR, class GIT_ERROR_INVALID.
            // SAFETY: an unknown key reads no variadic argument.
            let ret = unsafe { ffi::git_libgit2_opts(0x7fff) };
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
                    ffi::GIT_OPT_SET_SSL_CERT_LOCATIONS as c_int,
                    missing.as_ptr(),
                    ptr::null::<std::ffi::c_char>(),
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
                    ffi::GIT_OPT_GET_SEARCH_PATH as c_int,
                    999 as c_int,
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
        let ret = unsafe { ffi::git_error_set_str(c as c_int, c"class".as_ptr()) };
        assert_eq!(ret, 0);
        let from_c = last_klass();

        set_str(class.into(), c"class").unwrap();
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
