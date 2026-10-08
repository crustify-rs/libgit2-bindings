//! The suite's fixtures. `unsafe` is confined here, standing in for safe constructors
//! the crate does not have yet; replace each once its safe counterpart lands.
#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::process::Command;

use libgit2::libgit2::raw::OdbDataVec;

const CHILD: &str = "CRUSTIFY_UB_SAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2.
pub fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
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

/// Promotes a run every element of which the test wrote; stands in for the typed
/// constructors that fill a run in one safe call.
pub fn filled<T>(v: OdbDataVec<MaybeUninit<T>>) -> OdbDataVec<T> {
    // SAFETY: the caller wrote every element, which is this fixture's contract.
    unsafe { v.assume_init() }
}

/// A plain C struct to place in an `OdbDataBox`; the crate wraps no C type yet.
///
/// Replace with a wrapped libgit2 type once one is resource-free and `CZeroable`.
#[repr(C)]
pub struct pair_st {
    pub a: u64,
    pub b: u32,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);

// SAFETY: any `a` and `b` are valid, zero included; nothing tears a pair down.
unsafe impl ffibox::CZeroable for Pair {}
// SAFETY: a pair is plain data with no thread affinity; its handles keep Rust's
// borrow rules, so moving or sharing one across threads races nothing.
unsafe impl Send for Pair {}
// SAFETY: as above.
unsafe impl Sync for Pair {}

impl PairRef<'_> {
    pub fn a(&self) -> u64 {
        // SAFETY: a read through the shared handle's pointer to a live `pair_st`.
        unsafe { core::ptr::addr_of!((*self.as_ptr()).a).read() }
    }
    pub fn b(&self) -> u32 {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of!((*self.as_ptr()).b).read() }
    }
}

impl PairMut<'_> {
    pub fn set(&mut self, a: u64, b: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to a live `pair_st`.
        unsafe {
            core::ptr::addr_of_mut!((*p).a).write(a);
            core::ptr::addr_of_mut!((*p).b).write(b);
        }
    }
}
