//! A repository given an odb becomes its owner, and from then on every odb read and
//! write goes through the repository's object cache: `odb_cache()` in `odb.c` follows
//! `rc.owner` without holding a reference. The `Odb: Sync` proof (and so `OdbRef:
//! Send`) said nothing about that owner, so a shared handle used on another thread
//! raced the repository's release.
//!
//! The repository is given the odb the way a safe `Repository::wrap_odb` wrapper would
//! (one FFI call, adopted into the wave's `RepositoryBox`), and the other threads read
//! through the `Send` handle the way a safe `OdbRef::exists` wrapper would. Under
//! AddressSanitizer the reader hits the freed repository's cache:
//! `heap-use-after-free` in `cache_get` from `git_odb_exists_ext`.
//!
//! With the fix (`Odb<'r>` borrowing the owning repository; `OdbArc<'static>` only for
//! an odb no repository owns), the sound wrapper shapes make this scenario a compile
//! error (`tests/ub_safe/compile_fail/odb_owner_outlives_repository.rs`).
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use libgit2::ffi;
use libgit2::libgit2::odb::odb::{Odb, OdbArc};
use libgit2::libgit2::repository::repository::RepositoryBox;

/// The shape `git_repository_wrap_odb`'s wrapper had to avoid: it leaves the caller an
/// owner that does not borrow the repository now owning the odb.
fn wrap_odb(odb: &OdbArc) -> RepositoryBox {
    let mut raw = ptr::null_mut();
    // SAFETY: a live odb and a writable slot; the repository takes its own reference and
    // becomes the odb's owner.
    assert_eq!(unsafe { ffi::git_repository_wrap_odb(&mut raw, odb.as_c_ptr()) }, 0);
    // SAFETY: the fresh repository the call wrote, owned here alone.
    unsafe { RepositoryBox::from_c(raw) }.unwrap()
}

fn main() {
    let odb = Odb::new().unwrap();
    let id = ffi::git_oid { id: [0x11; 32], type_: ffi::GIT_OID_SHA1 as u8 };
    let done = AtomicBool::new(false);
    // The shared handle is `Send` (`Odb: Sync`), so other threads may read through it.
    let handle = odb.as_ref();
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    // SAFETY: `OdbRef` is `#[repr(transparent)]` over one pointer to the
                    // odb the arc keeps alive (its `as_ptr` is crate-private); what the
                    // wrapper of `git_odb_exists` on `OdbRef` does.
                    let p: *mut ffi::git_odb = unsafe { core::mem::transmute(handle) };
                    // SAFETY: a live odb the handle designates; the routine only reads
                    // it, through its own locks.
                    unsafe { ffi::git_odb_exists(p, &id) };
                }
            });
        }
        for _ in 0..20_000 {
            drop(wrap_odb(&odb));
        }
        done.store(true, Ordering::Relaxed);
    });
    drop(odb);
    println!("no use-after-free observed");
}
