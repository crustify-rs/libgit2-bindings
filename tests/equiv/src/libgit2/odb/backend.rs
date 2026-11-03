//! Equivalence of `git2/sys/odb_backend.h`'s backend table with libgit2's.
//!
//! [`OdbBackend::new`] is compared byte for byte with the template
//! `git_odb_init_backend` fills. libgit2's packed and loose backends are each built
//! twice, by their C constructors, over two `objects` directories laid out alike (a
//! pack `git repack` wrote, and loose objects beside it): one is called through its
//! members from C, the other adopted into an [`OdbBackendBox`] and used through the
//! handles. After each step of a sequence the two must agree: the getters against the
//! fields C reads, the ids each walk visits, a walk stopped by its callback (code, calls
//! and error), a rescan, a written `multi-pack-index` and the walk over it, and the
//! failure of a rescan whose pack directory is gone. Dropping the owner releases the
//! backend as calling its `free` member does, which the sanitizer builds check.
//!
//! Documented divergences, outside these comparisons: where a backend lacks a member,
//! C would call through NULL (`foreach`, `writemidx`), and the Rust handle fails with a
//! `GIT_ERROR_ODB` error naming the member instead (`refresh`, which the odb skips,
//! succeeds); the unit tests cover those.
//!
//! The constructors are not wrapped yet (`git_odb_backend_pack` and
//! `git_odb_backend_loose` are scheduled with their options), so the fixture declares
//! them and adopts their result through `from_c`.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io::Write as _;
use std::num::NonZero;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{fs, ptr};

use ffibox::CRefSeam;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::{
    ForeachCallback, OdbBackend, OdbBackendBox, OdbBackendRef, OdbError,
};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidRef};
use libgit2::libgit2::src::util::errors::ErrorClass;

// The fixture's raw constructors; their options are NULL, which both accept.
unsafe extern "C" {
    fn git_odb_backend_pack(
        out: *mut *mut ffi::git_odb_backend,
        objects_dir: *const c_char,
        opts: *const c_void,
    ) -> c_int;
    fn git_odb_backend_loose(
        out: *mut *mut ffi::git_odb_backend,
        objects_dir: *const c_char,
        opts: *mut c_void,
    ) -> c_int;
}

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    let _ = core::mem::ManuallyDrop::new(libgit2::libgit2::src::libgit2::libgit2::init().unwrap());
}

/// The thread's last libgit2 error, read through C: its class and message.
fn c_last() -> (c_int, String) {
    // SAFETY: libgit2 is initialised; the result is a static error or this thread's
    // error state, live until the next libgit2 call on this thread.
    let e = unsafe { ffi::git_error_last() };
    assert!(!e.is_null());
    // SAFETY: `e` is live (above); its fields are copied out before any other call.
    let klass = unsafe { (*e).klass };
    // SAFETY: as above.
    let message = unsafe { (*e).message };
    // SAFETY: as above, a NUL-terminated message.
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    (klass, message)
}

/// A failure as `(code, class, message)`, with `dir` written as `<dir>`.
fn c_fail(rc: c_int, dir: &Path) -> (c_int, c_int, String) {
    let (class, message) = c_last();
    (rc, class, message.replace(dir.to_str().unwrap(), "<dir>"))
}

/// A Rust failure as the triple [`c_fail`] reads.
fn rust_fail(err: &OdbError, dir: &Path) -> (c_int, c_int, String) {
    (
        err.raw_code(),
        err.raw_class(),
        err.message()
            .to_string_lossy()
            .replace(dir.to_str().unwrap(), "<dir>"),
    )
}

/// A directory of its own under the temporary directory, removed on drop, holding a
/// bare repository's `objects`: packed blobs, then loose ones, copied to `c` and `rust`.
struct Fixture(PathBuf);

impl Fixture {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "crustify-odb-backend-equiv-{}-{tag}",
            std::process::id()
        ));
        drop(fs::remove_dir_all(&root));
        fs::create_dir_all(&root).unwrap();
        let repo = root.join("repo");
        let git = |args: &[&str]| {
            let mut cmd = Command::new("git");
            cmd.args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("HOME", &root);
            cmd
        };
        let repo_s = repo.to_str().unwrap();
        assert!(
            git(&["init", "-q", "--bare", repo_s])
                .status()
                .unwrap()
                .success()
        );
        let hash = |blob: &str| {
            let mut child = git(&["-C", repo_s, "hash-object", "-w", "--stdin"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(blob.as_bytes())
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success());
            String::from_utf8(out.stdout).unwrap()
        };
        let packed: String = (0..6).map(|i| hash(&format!("packed {i}\n"))).collect();
        // No reference reaches the blobs, so they are packed by id, then pruned.
        let pack = repo.join("objects/pack/pack");
        let mut child = git(&["-C", repo_s, "pack-objects", "-q", pack.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(packed.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert!(
            git(&["-C", repo_s, "prune-packed", "-q"])
                .status()
                .unwrap()
                .success()
        );
        for i in 0..3 {
            hash(&format!("loose {i}\n"));
        }
        for side in ["c", "rust"] {
            let status = Command::new("cp")
                .arg("-a")
                .arg(repo.join("objects"))
                .arg(root.join(side))
                .status()
                .unwrap();
            assert!(status.success());
        }
        Self(root)
    }

    fn dir(&self, side: &str) -> PathBuf {
        self.0.join(side)
    }

    fn c_dir(&self, side: &str) -> CString {
        CString::new(self.dir(side).to_str().unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// Which libgit2 backend to build.
#[derive(Clone, Copy)]
enum Kind {
    Pack,
    Loose,
}

/// A backend C owns, called through its members.
fn c_backend(kind: Kind, dir: &CStr) -> *mut ffi::git_odb_backend {
    let mut out = ptr::null_mut();
    let rc = match kind {
        // SAFETY: `out` is writable and `dir` live for the call; NULL options are
        // accepted.
        Kind::Pack => unsafe { git_odb_backend_pack(&raw mut out, dir.as_ptr(), ptr::null()) },
        // SAFETY: as above.
        Kind::Loose => unsafe {
            git_odb_backend_loose(&raw mut out, dir.as_ptr(), ptr::null_mut())
        },
    };
    assert_eq!(rc, 0);
    out
}

/// The same backend, adopted by its owner: the documented unsafe fixture.
fn rust_backend(kind: Kind, dir: &CStr) -> OdbBackendBox {
    // SAFETY: a complete backend libgit2's constructor just built, with its `free`
    // member set, which no odb and nothing else holds.
    unsafe { OdbBackendBox::from_c(c_backend(kind, dir)) }.unwrap()
}

/// What the getters read, as C reads the fields: the version, whether an odb holds
/// it, and which members are set.
fn c_getters(c: *mut ffi::git_odb_backend) -> (u32, bool, [bool; 14]) {
    // SAFETY: `c` is a live backend; the fields are copied out.
    let b = unsafe { c.read() };
    (
        b.version,
        !b.odb.is_null(),
        [
            b.read.is_some(),
            b.read_prefix.is_some(),
            b.read_header.is_some(),
            b.write.is_some(),
            b.writestream.is_some(),
            b.readstream.is_some(),
            b.exists.is_some(),
            b.exists_prefix.is_some(),
            b.refresh.is_some(),
            b.foreach.is_some(),
            b.writepack.is_some(),
            b.writemidx.is_some(),
            b.freshen.is_some(),
            b.free.is_some(),
        ],
    )
}

fn rust_getters(r: OdbBackendRef<'_>) -> (u32, bool, [bool; 14]) {
    (
        r.version(),
        r.in_odb(),
        [
            r.has_read(),
            r.has_read_prefix(),
            r.has_read_header(),
            r.has_write(),
            r.has_write_stream(),
            r.has_read_stream(),
            r.has_exists(),
            r.has_exists_prefix(),
            r.has_refresh(),
            r.has_foreach(),
            r.has_writepack(),
            r.has_write_multi_pack_index(),
            r.has_freshen(),
            r.has_free(),
        ],
    )
}

/// A C walk's state: the ids seen, and the call that stops it (`stop_after` calls,
/// with `code`), if any.
struct CWalk {
    seen: Vec<Oid>,
    stop_after: usize,
    code: c_int,
}

unsafe extern "C" fn c_collect(id: *const ffi::git_oid, payload: *mut c_void) -> c_int {
    // SAFETY: `payload` is the walk's live `CWalk`, used by nothing else meanwhile.
    let walk = unsafe { &mut *payload.cast::<CWalk>() };
    // SAFETY: the backend lends a live id for the call.
    walk.seen
        .push(unsafe { OidRef::from_ptr(id.cast_mut()) }.unwrap().to_oid());
    if walk.seen.len() == walk.stop_after {
        walk.code
    } else {
        0
    }
}

/// Walks `c` through its `foreach` member: the return, and the ids seen.
fn c_foreach(c: *mut ffi::git_odb_backend, stop_after: usize, code: c_int) -> (c_int, Vec<Oid>) {
    let mut walk = CWalk {
        seen: Vec::new(),
        stop_after,
        code,
    };
    // SAFETY: `c` is a live backend; its member is copied out.
    let foreach = unsafe { (*c).foreach }.unwrap();
    // SAFETY: `foreach` is the member `c`'s constructor set; the callback and its
    // payload, `walk`, are live for the call.
    let rc = unsafe { foreach(c, Some(c_collect), ptr::from_mut(&mut walk).cast()) };
    (rc, walk.seen)
}

/// Walks `rust` through the handle, stopping as [`c_foreach`] does: the result as C's
/// return, and the ids seen.
fn rust_foreach(rust: &mut OdbBackendBox, stop_after: usize, code: c_int) -> (c_int, Vec<Oid>) {
    let mut seen = Vec::new();
    let mut collect = |id: OidRef<'_>| {
        seen.push(id.to_oid());
        if seen.len() == stop_after {
            ControlFlow::Break(NonZero::new(code).unwrap())
        } else {
            ControlFlow::Continue(())
        }
    };
    let rc = match rust
        .as_mut()
        .foreach(&mut ForeachCallback::new(&mut collect))
    {
        Ok(ControlFlow::Continue(())) => 0,
        Ok(ControlFlow::Break(code)) => code.get(),
        Err(err) => err.raw_code(),
    };
    (rc, seen)
}

/// Calls `c`'s `refresh` or `writemidx` member.
fn c_member(c: *mut ffi::git_odb_backend, midx: bool) -> c_int {
    // SAFETY: `c` is a live backend; it is copied out.
    let backend = unsafe { c.read() };
    let member = if midx {
        backend.writemidx
    } else {
        backend.refresh
    };
    // SAFETY: the member `c`'s constructor set, called alone on the live backend.
    unsafe { member.unwrap()(c) }
}

fn free_c(c: *mut ffi::git_odb_backend) {
    // SAFETY: `c` is a live backend; its member is copied out.
    let free = unsafe { (*c).free }.unwrap();
    // SAFETY: `c` is a live backend C owns, released once, through its own member.
    unsafe { free(c) }
}

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_template_matches_git_odb_init_backend() {
    init();
    let mut c = ffi::git_odb_backend::default();
    // SAFETY: `c` is a writable backend for the call.
    let rc = unsafe { ffi::git_odb_init_backend(&raw mut c, ffi::GIT_ODB_BACKEND_VERSION) };
    assert_eq!(rc, 0);
    let rust = OdbBackend::new();
    let size = size_of::<ffi::git_odb_backend>();
    // SAFETY: `c` is a live local of `size` bytes, read only.
    let c_bytes = unsafe { std::slice::from_raw_parts(ptr::from_ref(&c).cast::<u8>(), size) };
    // SAFETY: `rust` is a live local of the same layout and size, read only.
    let rust_bytes =
        unsafe { std::slice::from_raw_parts(rust.as_ref().as_ptr().cast::<u8>(), size) };
    assert_eq!(c_bytes, rust_bytes);
    assert_eq!(c_getters(&raw mut c), rust_getters(rust.as_ref()));
}

/// The sequence over libgit2's packed backend.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_packed_backend_matches_c() {
    init();
    let fixture = Fixture::new("pack");
    let c = c_backend(Kind::Pack, &fixture.c_dir("c"));
    let mut rust = rust_backend(Kind::Pack, &fixture.c_dir("rust"));
    assert_eq!(c_getters(c), rust_getters(rust.as_ref()));

    // A full walk: every packed id, in the backend's order.
    let (c_rc, c_seen) = c_foreach(c, 0, 0);
    let (r_rc, r_seen) = rust_foreach(&mut rust, 0, 0);
    assert_eq!((c_rc, c_seen.len()), (0, 6));
    assert_eq!((r_rc, &r_seen), (c_rc, &c_seen));

    // A walk the callback stops, with a code that is also an error code.
    assert_eq!(c_foreach(c, 2, -42), rust_foreach(&mut rust, 2, -42));
    assert_eq!(c_foreach(c, 3, 7), rust_foreach(&mut rust, 3, 7));

    // A rescan, then a written multi-pack-index and the walk through it.
    assert_eq!(c_member(c, false), 0);
    rust.as_mut().refresh().unwrap();
    assert_eq!(c_member(c, true), 0);
    rust.as_mut().write_multi_pack_index().unwrap();
    for side in ["c", "rust"] {
        assert!(fixture.dir(side).join("pack/multi-pack-index").is_file());
    }
    let (c_rc, mut c_seen) = c_foreach(c, 0, 0);
    let (r_rc, mut r_seen) = rust_foreach(&mut rust, 0, 0);
    assert_eq!(r_rc, c_rc);
    c_seen.sort();
    r_seen.sort();
    assert_eq!(r_seen, c_seen);
    assert_eq!(c_getters(c), rust_getters(rust.as_ref()));

    // A rescan whose pack directory is gone.
    for side in ["c", "rust"] {
        fs::remove_dir_all(fixture.dir(side).join("pack")).unwrap();
    }
    let c_err = c_fail(c_member(c, false), &fixture.dir("c"));
    let r_err = rust.as_mut().refresh().unwrap_err();
    assert_eq!(rust_fail(&r_err, &fixture.dir("rust")), c_err);
    assert_eq!(c_err.0, ffi::GIT_ENOTFOUND);

    free_c(c);
    drop(rust);
}

/// The sequence over libgit2's loose backend.
#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_loose_backend_matches_c() {
    init();
    let fixture = Fixture::new("loose");
    let c = c_backend(Kind::Loose, &fixture.c_dir("c"));
    let mut rust = rust_backend(Kind::Loose, &fixture.c_dir("rust"));
    let getters = c_getters(c);
    assert_eq!(getters, rust_getters(rust.as_ref()));
    // No `refresh` or `writemidx`: the odb skips it for the one, and fails for the other.
    assert!(!getters.2[8] && !getters.2[11]);

    let (c_rc, mut c_seen) = c_foreach(c, 0, 0);
    let (r_rc, mut r_seen) = rust_foreach(&mut rust, 0, 0);
    assert_eq!((c_rc, c_seen.len()), (0, 3));
    c_seen.sort();
    r_seen.sort();
    assert_eq!((r_rc, r_seen), (c_rc, c_seen));

    // A stopped walk: libgit2's loose backend records the callback's code as an error.
    let (c_rc, c_seen) = c_foreach(c, 1, -42);
    let c_err = c_fail(c_rc, &fixture.dir("c"));
    let r = rust_foreach(&mut rust, 1, -42);
    let r_err = c_fail(r.0, &fixture.dir("rust"));
    assert_eq!((r.0, r.1.len()), (c_rc, c_seen.len()));
    assert_eq!(r_err, c_err);
    assert_eq!(c_err.1, ErrorClass::Callback.raw());

    // A rescan has nothing to do.
    rust.as_mut().refresh().unwrap();

    free_c(c);
    drop(rust);
}
