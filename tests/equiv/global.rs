//! Equivalence of `git2/global.h`'s `init` and `Runtime::shutdown` with
//! `git_libgit2_init` and `git_libgit2_shutdown`.
//!
//! The count is process-global, so each comparison runs in a child process, where
//! nothing has initialised libgit2: the same sequence of initialisations and
//! shutdowns runs through C and then through Rust, from 0 each time, and the counts
//! each step returns must agree, as must a sequence mixing the two. With an allocator
//! refusing every request installed first, the first initialisation fails in both.
//!
//! Documented divergence, outside these comparisons (`global.rs`'s unit tests): after
//! a failed initialisation C answers every later `git_libgit2_init` with success over
//! half-initialised state, while `init` refuses (`InitError::Poisoned`).

use std::ffi::{CStr, c_int};
use std::process::Command;
use std::ptr::NonNull;

use libgit2::ffi;
use libgit2::libgit2::core::common;
use libgit2::libgit2::core::global::{InitError, Runtime, init};
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2.
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

fn c_init() -> c_int {
    // SAFETY: no arguments; counted under libgit2's own mutex.
    unsafe { ffi::git_libgit2_init() }
}

fn c_shutdown() -> c_int {
    // SAFETY: no arguments; every call here releases a reference `c_init` took.
    unsafe { ffi::git_libgit2_shutdown() }
}

/// One step of a sequence: take a reference, or release the most recent one.
#[derive(Clone, Copy, Debug)]
enum Step {
    Init,
    Shutdown,
}

const SEQUENCE: &[Step] = {
    use Step::*;
    &[
        Init, Init, Shutdown, Init, Init, Shutdown, Shutdown, Shutdown, Init, Shutdown,
    ]
};

fn run_c(steps: &[Step]) -> Vec<c_int> {
    steps
        .iter()
        .map(|step| match step {
            Step::Init => c_init(),
            Step::Shutdown => c_shutdown(),
        })
        .collect()
}

fn run_rust(steps: &[Step]) -> Vec<c_int> {
    let mut held: Vec<Runtime> = Vec::new();
    steps
        .iter()
        .map(|step| match step {
            Step::Init => {
                let r = init().unwrap();
                let n = i32::try_from(r.count().get()).unwrap();
                held.push(r);
                n
            }
            Step::Shutdown => i32::try_from(held.pop().unwrap().shutdown().unwrap()).unwrap(),
        })
        .collect()
}

/// Counts agree step by step, each run starting from 0, with full shutdowns between.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn counts_match_c() {
    isolated(concat!(module_path!(), "::counts_match_c"), || {
        let c = run_c(SEQUENCE);
        assert_eq!(c, [1, 2, 1, 2, 3, 2, 1, 0, 1, 0]);
        assert_eq!(run_rust(SEQUENCE), c);
        assert_eq!(run_c(SEQUENCE), c, "Rust left the count at 0");
        // Settings libgit2 sets up at initialisation read the same after each cycle.
        let agent = common::user_agent().unwrap();
        assert_eq!(agent.to_bytes(), b"libgit2 1.9.0");
    });
}

/// References taken through C and Rust count together; each side releases its own.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn mixed_references_count_together() {
    isolated(
        concat!(module_path!(), "::mixed_references_count_together"),
        || {
            assert_eq!(c_init(), 1);
            let r = init().unwrap();
            assert_eq!(r.count().get(), 2);
            assert_eq!(c_init(), 3);
            assert_eq!(c_shutdown(), 2);
            assert_eq!(r.shutdown(), Ok(1));
            let r = init().unwrap();
            assert_eq!(r.count().get(), 2);
            drop(r);
            assert_eq!(c_shutdown(), 0);
        },
    );
}

static REFUSING: Allocator = Allocator::new::<Refusing>();

/// An allocator refusing every request.
struct Refusing;

// SAFETY: it never hands out an allocation, which `Alloc` allows; `free` is never
// reached, since nothing it released was allocated.
unsafe impl Alloc for Refusing {
    fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        None
    }

    unsafe fn realloc(
        _: Option<NonNull<u8>>,
        _: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        None
    }

    unsafe fn free(_: NonNull<u8>) {}
}

/// With nothing allocatable, the first initialisation fails in C ...
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn c_init_fails_without_memory() {
    isolated(
        concat!(module_path!(), "::c_init_fails_without_memory"),
        || {
            let raw = (&raw const REFUSING)
                .cast::<ffi::git_allocator>()
                .cast_mut();
            // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, so `raw`
            // addresses a complete one, which libgit2 copies before anything is allocated.
            let rc =
                unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(), raw) };
            assert_eq!(rc, 0);
            assert_eq!(c_init(), -1);
        },
    );
}

/// ... and through `init`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn rust_init_fails_without_memory() {
    isolated(
        concat!(module_path!(), "::rust_init_fails_without_memory"),
        || {
            // SAFETY: nothing has initialised libgit2 or allocated through it in this
            // child, and no other thread runs.
            unsafe { common::set_allocator(Some(REFUSING.as_ref())) };
            assert_eq!(init().unwrap_err(), InitError::Failed);
        },
    );
}
