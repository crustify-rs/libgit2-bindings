//! Dropping an empty `OidArrayVal` while another thread initialises libgit2 for the
//! first time is a data race (ThreadSanitizer).
//!
//! `OidArray::new()` allocates nothing and never touches libgit2, so nothing orders
//! its drop after `git_libgit2_init`; the drop still calls `git_oidarray_dispose`,
//! whose `git__free(NULL)` reads `git__allocator.gfree`. The first initialisation,
//! which any allocating constructor on another thread performs through
//! `runtime_pinned()`, writes that member (`git_stdalloc_init_allocator`). A
//! non-empty array cannot race: its own allocation pinned the runtime first.
//!
//! Build and run against the `tsan` variant, from this directory:
//!
//! ```sh
//! CRUSTIFY_BUILD_VARIANT=tsan \
//! CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zsanitizer=thread \
//! TSAN_OPTIONS=halt_on_error=1 \
//! LD_LIBRARY_PATH=/target/crustify/builds/tsan \
//! cargo +nightly run -Zbuild-std --target x86_64-unknown-linux-gnu
//! ```
//!
//! Exits non-zero with `WARNING: ThreadSanitizer: data race` (read in `git__free`
//! under `git_oidarray_dispose`, previous write in `git_stdalloc_init_allocator`) on
//! the affected commit; exits 0 once the policy skips the C call for an empty array.
#![forbid(unsafe_code)]

use std::sync::{Arc, Barrier};
use std::thread;

use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::oid::oidarray::OidArray;

fn main() {
    let start = Arc::new(Barrier::new(2));

    // Drops empty arrays: no allocation, no initialisation, a C dispose each.
    let disposer = thread::spawn({
        let start = Arc::clone(&start);
        move || {
            start.wait();
            for _ in 0..64 {
                drop(OidArray::new());
            }
        }
    });

    // The process's first libgit2 initialisation, through the first allocation.
    let allocator = thread::spawn(move || {
        start.wait();
        let array = OidArray::from_slice(&[Oid::zero(OidType::Sha1)]).unwrap();
        assert_eq!(array.as_ref().len(), 1);
    });

    disposer.join().unwrap();
    allocator.join().unwrap();
    println!("both threads finished; ThreadSanitizer reports the race, if any, above");
}
