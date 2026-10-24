//! Equivalence of `git2/sys/errors.h`'s safe wrappers with the C routines they call:
//! `git_error_clear`, `git_error_set`, `git_error_set_str` and `git_error_set_oom`.
//!
//! libgit2 keeps the last error per thread, so each side runs a sequence on a fresh
//! thread of its own, and the two are compared after every step: the return value,
//! `errno`, and the class and message `git_error_last` reports. The C side passes
//! `git_error_set` the message as its format, `%` doubled, as C callers write it; the
//! Rust side passes the message itself.
//!
//! Allocation failures are compared in a child process that installs an allocator
//! refusing requests on demand, per thread, before `git_libgit2_init`.
//!
//! Documented divergence, outside these comparisons: before `git_libgit2_init` C
//! looks the thread state up under a nonexistent TLS key, and the wrappers pin the
//! runtime first; the corrected behaviour is tested in the module's `unit_tests` and
//! in `ub_safe`.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::process::Command;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::src::util::errors;
use libgit2::libgit2::src::util::errors::ErrorClass;

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's globals.
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

// --- the sequences --------------------------------------------------------------

#[derive(Clone, Debug)]
enum Step {
    Clear,
    /// `git_error_set` with a message; `errors::set`.
    Set(ErrorClass, Vec<u8>),
    /// `git_error_set` with a NULL format; `errors::set_without_message`.
    Bare(ErrorClass),
    Str(ErrorClass, Vec<u8>),
    Oom,
    /// Sets the thread's `errno`.
    Errno(c_int),
    /// Refuses the thread's next allocation (allocation-failure child only).
    FailNext,
}

/// What a step leaves observable.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    ret: Option<c_int>,
    errno: c_int,
    class: c_int,
    message: Vec<u8>,
}

#[derive(Clone, Copy)]
enum Side {
    C,
    Rust,
}

fn set_errno(value: c_int) {
    // SAFETY: the calling thread's `errno`, valid for writes.
    let value2 = unsafe { libc::__errno_location() };
    // SAFETY: as above.
    unsafe { value2.write(value) }
}

fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap()
}

/// `git_error_last()`'s class and message.
fn last_error() -> (c_int, Vec<u8>) {
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

fn c_string(message: &[u8]) -> CString {
    CString::new(message).expect("test messages hold no NUL")
}

fn c_step(step: &Step) -> Option<c_int> {
    match step {
        // SAFETY: no arguments.
        Clear => unsafe { ffi::git_error_clear() },
        Set(class, message) => {
            let mut format = Vec::new();
            for &b in message {
                format.extend_from_slice(if b == b'%' {
                    b"%%"
                } else {
                    core::slice::from_ref(&b)
                });
            }
            let format = c_string(&format);
            // SAFETY: a format naming no argument, as every `%` is doubled.
            unsafe { ffi::git_error_set(class.raw(), format.as_ptr()) }
        }
        // SAFETY: a NULL format reads no argument.
        Bare(class) => unsafe { ffi::git_error_set(class.raw(), ptr::null()) },
        Str(class, message) => {
            let message = c_string(message);
            // SAFETY: a NUL-terminated string, copied during the call.
            return Some(unsafe { ffi::git_error_set_str(class.raw(), message.as_ptr()) });
        }
        // SAFETY: no arguments.
        Oom => unsafe { ffi::git_error_set_oom() },
        Errno(value) => set_errno(*value),
        FailNext => refuse_next(),
    }
    None
}

fn rust_step(step: &Step) -> Option<c_int> {
    match step {
        Clear => errors::clear(),
        Set(class, message) => errors::set(*class, &c_string(message)),
        Bare(class) => errors::set_without_message(*class),
        Str(class, message) => {
            return Some(match errors::set_str(*class, &c_string(message)) {
                Ok(()) => 0,
                Err(errors::MessageNotStored) => -1,
            });
        }
        Oom => errors::set_oom(),
        Errno(value) => set_errno(*value),
        FailNext => refuse_next(),
    }
    None
}

/// Runs `steps` on a fresh thread, observing `errno` after each, and libgit2's last
/// error after each libgit2 call.
fn run(side: Side, steps: &[Step]) -> Vec<Seen> {
    let steps = steps.to_vec();
    std::thread::spawn(move || {
        // A fresh thread can start with `errno` already `EAGAIN`: std's thread start
        // waits on a contended lock before running this closure.
        set_errno(0);
        steps
            .iter()
            .map(|step| {
                let ret = match side {
                    Side::C => c_step(step),
                    Side::Rust => rust_step(step),
                };
                let errno = errno();
                // Observing allocates the thread's state, and would spend a refusal
                // meant for the next step.
                let (class, message) = match step {
                    Errno(_) | FailNext => (0, Vec::new()),
                    _ => last_error(),
                };
                Seen {
                    ret,
                    errno,
                    class,
                    message,
                }
            })
            .collect()
    })
    .join()
    .unwrap()
}

/// Runs `steps` on both sides and compares every step.
fn compare(steps: &[Step]) -> Vec<Seen> {
    let (c, rust) = (run(Side::C, steps), run(Side::Rust, steps));
    for (i, step) in steps.iter().enumerate() {
        assert_eq!(c[i], rust[i], "step {i}: {step:?}");
    }
    c
}

fn init() {
    // SAFETY: no arguments; refcounted, never released in this process. The C side
    // needs it; the Rust side pins its own reference.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
}

fn text(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| b'a' + i.to_le_bytes()[0].wrapping_mul(7).wrapping_add(seed) % 26)
        .collect()
}

fn class(c: ErrorClass) -> c_int {
    c.raw()
}

// --- the comparisons ------------------------------------------------------------

use Step::*;

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn setters_and_clear_in_sequence() {
    init();
    let long = text(4_000, 3);
    let seen = compare(&[
        Clear,
        Str(ErrorClass::Invalid, b"first".to_vec()),
        Set(ErrorClass::Reference, b"second".to_vec()),
        Set(ErrorClass::Config, b"100% sure: %s %d %n %".to_vec()),
        Str(ErrorClass::Config, b"100% %s verbatim".to_vec()),
        Oom,
        Set(ErrorClass::Odb, b"".to_vec()),
        Clear,
        Clear,
        Errno(libc::ENOENT),
        Set(ErrorClass::Os, b"open".to_vec()),
        Errno(0),
        Set(ErrorClass::Os, b"quiet".to_vec()),
        Errno(libc::EACCES),
        Bare(ErrorClass::Os),
        Bare(ErrorClass::Os),
        Bare(ErrorClass::Invalid),
        Errno(libc::ENOENT),
        Str(ErrorClass::Os, b"verbatim".to_vec()),
        Errno(libc::EPERM),
        Set(ErrorClass::Invalid, b"not an OS error".to_vec()),
        Clear,
        Str(ErrorClass::Grafts, long.clone()),
        Set(ErrorClass::Internal, text(3_000, 9)),
        Bare(ErrorClass::Grafts),
        Errno(libc::EINTR),
        Oom,
        Str(ErrorClass::None, b"\xff\xfe not UTF-8".to_vec()),
        Set(ErrorClass::None, b"line\nbreak".to_vec()),
        Clear,
    ]);
    // Spot checks that the sequence reached what it means to.
    assert_eq!(seen[0].message, b"no error");
    assert_eq!(seen[3].message, b"100% sure: %s %d %n %");
    assert_eq!(
        (seen[5].class, &seen[5].message[..]),
        (class(ErrorClass::NoMemory), &b"Out of memory"[..])
    );
    assert_eq!(
        (seen[10].message.as_slice(), seen[10].errno),
        (&b"open: No such file or directory"[..], 0)
    );
    assert_eq!(seen[12].message, b"quiet: ");
    assert_eq!(seen[14].message, b"Permission denied");
    assert_eq!(
        (seen[15].message.as_slice(), seen[16].message.as_slice()),
        (&b""[..], &b""[..])
    );
    assert_eq!(
        (seen[18].errno, seen[20].errno, seen[21].errno),
        (libc::ENOENT, libc::EPERM, 0)
    );
    assert_eq!(
        (seen[22].class, &seen[22].message),
        (class(ErrorClass::Grafts), &long)
    );
    assert_eq!(seen[23].class, class(ErrorClass::Internal));
    assert_eq!(seen[26].errno, libc::EINTR);
    assert!(seen.iter().all(|s| s.ret.is_none_or(|r| r == 0)));
}

/// The buffer grows and shrinks across messages of every size.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn messages_grow_and_shrink() {
    init();
    let mut steps = Vec::new();
    for (i, len) in [0, 1, 7, 8, 9, 63, 64, 65, 1000, 4095, 4097, 3, 0]
        .into_iter()
        .enumerate()
    {
        let seed = u8::try_from(i).unwrap();
        steps.push(Str(ErrorClass::Index, text(len, seed)));
        steps.push(Set(ErrorClass::Tree, text(len, seed + 1)));
        steps.push(Errno(libc::ENOTDIR));
        steps.push(Set(ErrorClass::Os, text(len, seed + 2)));
    }
    compare(&steps);
}

/// Each thread keeps its own error: a sequence interleaved with another thread's
/// sees only its own steps.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_keep_their_own_errors() {
    init();
    let steps: Vec<Step> = (0..64)
        .flat_map(|i| {
            [
                Str(ErrorClass::Net, text(i * 5, u8::try_from(i).unwrap())),
                Oom,
                Clear,
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

/// Every setter when the thread's state cannot be allocated, and when its message
/// buffer cannot grow, after which the buffer stays out of memory.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn allocation_failures() {
    isolated(concat!(module_path!(), "::allocation_failures"), || {
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
        errors::set_oom();

        let none = (class(ErrorClass::None), b"no error".to_vec());
        let oom = (class(ErrorClass::NoMemory), b"Out of memory".to_vec());
        let error = |s: &Seen| (s.class, s.message.clone());

        // The thread's state cannot be allocated: nothing is set, `clear` keeps
        // `errno`, and `set_str` fails. Observing allocates it afterwards.
        let seen = compare(&[Errno(libc::EPERM), FailNext, Clear]);
        assert_eq!(
            (seen[2].errno, error(&seen[2])),
            (libc::EPERM, none.clone())
        );
        let seen = compare(&[FailNext, Str(ErrorClass::Invalid, b"x".to_vec())]);
        assert_eq!((seen[1].ret, error(&seen[1])), (Some(-1), none.clone()));
        let seen = compare(&[FailNext, Set(ErrorClass::Invalid, b"x".to_vec())]);
        assert_eq!(error(&seen[1]), none);
        let seen = compare(&[FailNext, Errno(libc::ENOENT), Bare(ErrorClass::Os)]);
        assert_eq!(
            (seen[2].errno, error(&seen[2])),
            (libc::ENOENT, none.clone())
        );
        let seen = compare(&[FailNext, Oom]);
        assert_eq!(error(&seen[1]), none);

        // The message cannot be stored, after which the buffer refuses every message.
        let seen = compare(&[
            Str(ErrorClass::Invalid, b"warm".to_vec()),
            FailNext,
            Str(ErrorClass::Config, text(1000, 1)),
            Str(ErrorClass::Config, b"short".to_vec()),
            Set(ErrorClass::Reference, b"x".to_vec()),
            Bare(ErrorClass::Invalid),
            Errno(libc::ENOENT),
            Bare(ErrorClass::Os),
            Clear,
            Str(ErrorClass::Config, b"again".to_vec()),
            Oom,
        ]);
        assert_eq!((seen[2].ret, error(&seen[2])), (Some(-1), oom.clone()));
        assert_eq!((seen[3].ret, error(&seen[3])), (Some(-1), oom.clone()));
        assert!([4, 5, 7].iter().all(|&i| error(&seen[i]) == oom));
        assert_eq!(
            seen[7].errno, 0,
            "consumed although the message was dropped"
        );
        assert_eq!(error(&seen[8]), none);
        assert_eq!((seen[9].ret, error(&seen[9])), (Some(-1), none.clone()));
        assert_eq!(error(&seen[10]), oom);

        // The same through `git_error_set`'s formatting.
        let seen = compare(&[
            Set(ErrorClass::Invalid, b"warm".to_vec()),
            FailNext,
            Set(ErrorClass::Config, text(1000, 2)),
            Str(ErrorClass::Config, b"short".to_vec()),
        ]);
        assert_eq!(error(&seen[2]), oom);
        assert_eq!((seen[3].ret, error(&seen[3])), (Some(-1), oom));
    });
}
