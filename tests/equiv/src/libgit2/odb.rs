//! Equivalence of `git2/odb.h`'s object database constructors and owner with libgit2's.
//!
//! Each Rust constructor ([`Odb::new`], [`Odb::with_oid_type`], [`Odb::open`],
//! [`Odb::open_with_oid_type`]) runs beside the C routine it wraps (`git_odb_new`,
//! `git_odb_new_ext`, `git_odb_open`, `git_odb_open_ext`) on independent inputs: two
//! `objects` directories laid out alike. A failure must report libgit2's error; a
//! success must build an odb that behaves like C's through a sequence of steps, each
//! compared as it runs: the backend count, writes (the id each blob gets, or the
//! failure), header reads and existence checks. The Rust odb has no safe readers yet,
//! so the steps call libgit2 on both odbs alike.
//!
//! The owner's lifecycle is compared too: an [`OdbArc`] given to a repository
//! (`git_repository_wrap_odb`) and then dropped must leave the repository the same odb
//! a C caller's `git_odb_free` leaves it, and an odb must outlive a repository freed
//! first in both.

mod sys;

use std::ffi::{CStr, CString, c_int};
use std::path::{Path, PathBuf};
use std::{fs, ptr};

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::{Odb, OdbArc};
use libgit2::libgit2::src::libgit2::oid::OidType;
use libgit2::libgit2::src::util::errors::ErrorBuf;

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
    // SAFETY: `e` is live (above), with a NUL-terminated message; both are copied out
    // before any other libgit2 call.
    let klass = unsafe { (*e).klass };
    // SAFETY: as above.
    let message2 = unsafe { (*e).message };
    // SAFETY: as above.
    unsafe {
        (
            klass,
            CStr::from_ptr(message2).to_string_lossy().into_owned(),
        )
    }
}

/// A Rust failure as the pair [`c_last`] reads.
fn rust_err(err: &ErrorBuf) -> (c_int, String) {
    (
        err.raw_class(),
        err.message().to_string_lossy().into_owned(),
    )
}

/// A directory of its own under the temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("crustify-odb-equiv-{}-{tag}", std::process::id()));
        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// `name` under the directory, as a C string.
    fn c_path(&self, name: &str) -> CString {
        c_path(&self.0.join(name))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

fn c_path(path: &Path) -> CString {
    CString::new(path.to_str().unwrap()).unwrap()
}

/// One reference to an odb a C constructor built, released on drop.
struct COdb(*mut ffi::git_odb);

impl Drop for COdb {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the reference the constructor returned, released once.
        unsafe { ffi::git_odb_free(self.0) };
    }
}

/// A C constructor's result, as the Rust constructors report theirs.
fn c_built(rc: c_int, out: *mut ffi::git_odb) -> Result<COdb, (c_int, String)> {
    if rc < 0 {
        assert!(out.is_null(), "a failed constructor left an odb");
        Err(c_last())
    } else {
        assert_eq!(rc, 0);
        assert!(!out.is_null());
        Ok(COdb(out))
    }
}

fn c_new() -> Result<COdb, (c_int, String)> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a writable slot, filled with a fresh reference on success.
    let rc = unsafe { ffi::git_odb_new(&raw mut out) };
    c_built(rc, out)
}

fn c_options(oid_type: ffi::git_oid_t) -> ffi::git_odb_options {
    ffi::git_odb_options {
        version: ffi::GIT_ODB_OPTIONS_VERSION,
        oid_type,
    }
}

fn c_new_ext(opts: Option<&ffi::git_odb_options>) -> Result<COdb, (c_int, String)> {
    let mut out = ptr::null_mut();
    let opts = opts.map_or(ptr::null(), ptr::from_ref);
    // SAFETY: `out` is a writable slot; `opts` is NULL or a live options struct, read
    // for the call only.
    let rc = unsafe { ffi::git_odb_new_ext(&raw mut out, opts) };
    c_built(rc, out)
}

fn c_open(dir: &CStr) -> Result<COdb, (c_int, String)> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is a writable slot; `dir` is NUL-terminated, read for the call.
    let rc = unsafe { ffi::git_odb_open(&raw mut out, dir.as_ptr()) };
    c_built(rc, out)
}

fn c_open_ext(dir: &CStr, opts: Option<&ffi::git_odb_options>) -> Result<COdb, (c_int, String)> {
    let mut out = ptr::null_mut();
    let opts = opts.map_or(ptr::null(), ptr::from_ref);
    // SAFETY: as `c_open`; `opts` is NULL or a live options struct, read for the call.
    let rc = unsafe { ffi::git_odb_open_ext(&raw mut out, dir.as_ptr(), opts) };
    c_built(rc, out)
}

/// A `git_oid`'s type byte and id bytes.
type Id = (u8, [u8; 32]);

/// One step over an odb.
#[derive(Clone, Copy, Debug)]
enum Op<'a> {
    Backends,
    Write(&'a [u8]),
    Header(Id),
    Exists(Id),
}

/// What one step observed: a count, the id a write produced, a header, an existence
/// answer, or the return and the thread's last error after a failure.
#[derive(Debug, PartialEq)]
enum Seen {
    Backends(usize),
    Written(Id),
    Header(ffi::git_object_t, usize),
    Exists(c_int),
    Failed(c_int, (c_int, String)),
}

/// The thread's last error with the random suffix of a loose backend's temporary
/// file name (`tmp_object_git2_<hex>`) blanked, so two writes' failures compare.
#[expect(
    clippy::items_after_statements,
    reason = "each helper item sits beside the steps that use it"
)]
fn c_last_normalized() -> (c_int, String) {
    let (class, mut message) = c_last();
    const TMP: &str = "tmp_object_git2_";
    if let Some(at) = message.find(TMP) {
        let start = at + TMP.len();
        let end = message[start..]
            .find(|c: char| !c.is_ascii_alphanumeric())
            .map_or(message.len(), |n| start + n);
        message.replace_range(start..end, "*");
    }
    (class, message)
}

/// Runs `op` on the live odb `odb`, through C.
fn step(odb: *mut ffi::git_odb, op: Op<'_>) -> Seen {
    match op {
        Op::Backends => {
            // SAFETY: `odb` is live; the routine reads its backend count under its lock.
            Seen::Backends(unsafe { ffi::git_odb_num_backends(odb) })
        }
        Op::Write(data) => {
            let mut id = ffi::git_oid::default();
            // SAFETY: `id` is a writable `git_oid`; `data` is live for its length,
            // read for the call only.
            let rc = unsafe {
                ffi::git_odb_write(
                    &raw mut id,
                    odb,
                    data.as_ptr().cast(),
                    data.len(),
                    ffi::GIT_OBJECT_BLOB,
                )
            };
            if rc == 0 {
                Seen::Written((id.type_, id.id))
            } else {
                Seen::Failed(rc, c_last_normalized())
            }
        }
        Op::Header((type_, id)) => {
            let id = ffi::git_oid { type_, id };
            let (mut len, mut kind) = (usize::MAX, ffi::git_object_t::MAX);
            // SAFETY: `len` and `kind` are writable slots; `id` is a live `git_oid`.
            let rc = unsafe {
                ffi::git_odb_read_header(&raw mut len, &raw mut kind, odb, &raw const id)
            };
            if rc == 0 {
                Seen::Header(kind, len)
            } else {
                Seen::Failed(rc, c_last())
            }
        }
        Op::Exists((type_, id)) => {
            let id = ffi::git_oid { type_, id };
            // SAFETY: `id` is a live `git_oid`, read for the call only.
            Seen::Exists(unsafe { ffi::git_odb_exists(odb, &raw const id) })
        }
    }
}

/// `seen` with every mention of the directory `dir` a failure names blanked.
fn blank(seen: Seen, dir: Option<&CStr>) -> Seen {
    match (seen, dir) {
        (Seen::Failed(rc, (class, message)), Some(dir)) => {
            let dir = dir.to_str().unwrap();
            Seen::Failed(rc, (class, message.replace(dir, "<dir>")))
        }
        (seen, _) => seen,
    }
}

/// Runs the same steps on both odbs, comparing each pair as it is observed; the ids
/// the writes produce feed the later steps. `dirs` names the directories each side
/// opened, which failures blank.
fn compare_in(
    c: *mut ffi::git_odb,
    rust: &OdbArc<'_>,
    dirs: Option<(&CStr, &CStr)>,
    data: &[&[u8]],
) {
    let mut ids = vec![(0u8, [0u8; 32])];
    let both = |op: Op<'_>| {
        let seen = blank(step(c, op), dirs.map(|d| d.0));
        assert_eq!(
            blank(step(rust.as_c_ptr(), op), dirs.map(|d| d.1)),
            seen,
            "{op:?}"
        );
        seen
    };
    both(Op::Backends);
    for &bytes in data {
        if let Seen::Written(id) = both(Op::Write(bytes)) {
            ids.push(id);
        }
    }
    for &id in &ids {
        both(Op::Header(id));
        both(Op::Exists(id));
    }
    // A second write of what is stored is a no-op that reports the same id.
    for &bytes in data {
        both(Op::Write(bytes));
    }
    both(Op::Backends);
}

/// Blobs to write: empty, small, binary, and larger than a loose object's zlib window.
fn blobs() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"hello, odb\n".to_vec(),
        (0..=255u8).collect(),
        (0..70_000u32).map(|i| (i * 7 % 251) as u8).collect(),
    ]
}

fn compare_blobs(c: *mut ffi::git_odb, rust: &OdbArc<'_>) {
    compare_blobs_in(c, rust, None);
}

fn compare_blobs_in(c: *mut ffi::git_odb, rust: &OdbArc<'_>, dirs: Option<(&CStr, &CStr)>) {
    let blobs = blobs();
    let data: Vec<&[u8]> = blobs.iter().map(Vec::as_slice).collect();
    compare_in(c, rust, dirs, &data);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_builds_what_git_odb_new_builds() {
    init();
    let c = c_new().unwrap();
    let rust = Odb::new().unwrap();
    // No backend: writes fail, and nothing is found.
    compare_blobs(c.0, &rust);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn with_oid_type_builds_what_git_odb_new_ext_builds() {
    init();
    for kind in [OidType::Sha1, OidType::Sha256] {
        let c = c_new_ext(Some(&c_options(kind.raw()))).unwrap();
        let rust = Odb::with_oid_type(kind).unwrap();
        compare_blobs(c.0, &rust);
    }
    // `new` is `new_ext` with the default options, which a NULL options pointer and a
    // zeroed type also select.
    let rust = Odb::with_oid_type(OidType::default()).unwrap();
    for c in [
        c_new_ext(None).unwrap(),
        c_new_ext(Some(&c_options(0))).unwrap(),
    ] {
        compare_blobs(c.0, &rust);
    }
}

/// Two `objects` directories laid out alike, one for each side.
struct Pair {
    _dir: TempDir,
    c: CString,
    rust: CString,
}

impl Pair {
    /// `objects` directories under `tag`, each prepared by `lay_out`.
    fn new(tag: &str, lay_out: impl Fn(&Path)) -> Self {
        let dir = TempDir::new(tag);
        for side in ["c", "rust"] {
            let objects = dir.0.join(side).join("objects");
            fs::create_dir_all(&objects).unwrap();
            lay_out(&objects);
        }
        Self {
            c: dir.c_path("c/objects"),
            rust: dir.c_path("rust/objects"),
            _dir: dir,
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_builds_what_git_odb_open_builds() {
    init();
    let dirs = Pair::new("open", |_| {});
    let c = c_open(&dirs.c).unwrap();
    let rust = Odb::open(&dirs.rust).unwrap();
    // The loose and the packed backend: writes land as loose objects, read back.
    compare_blobs(c.0, &rust);
    // Reopened, each directory serves what was written to it.
    drop((c, rust));
    let c = c_open(&dirs.c).unwrap();
    let rust = Odb::open(&dirs.rust).unwrap();
    compare_blobs(c.0, &rust);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_with_oid_type_builds_what_git_odb_open_ext_builds() {
    init();
    for kind in [OidType::Sha1, OidType::Sha256] {
        let dirs = Pair::new(&format!("open-ext-{}", kind.raw()), |_| {});
        let c = c_open_ext(&dirs.c, Some(&c_options(kind.raw()))).unwrap();
        let rust = Odb::open_with_oid_type(&dirs.rust, kind).unwrap();
        compare_blobs(c.0, &rust);
        let Seen::Written((type_, _)) = step(rust.as_c_ptr(), Op::Write(b"typed")) else {
            panic!("an on-disk odb writes")
        };
        assert_eq!(OidType::from_raw(type_.into()), Some(kind));
    }
    let dirs = Pair::new("open-ext-default", |_| {});
    let c = c_open_ext(&dirs.c, None).unwrap();
    let rust = Odb::open_with_oid_type(&dirs.rust, OidType::default()).unwrap();
    compare_blobs(c.0, &rust);
}

/// Every way the open fails reports what C reports, and nothing is built.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_fails_as_git_odb_open_fails() {
    init();
    let dir = TempDir::new("open-fail");
    for file in ["c-file", "rust-file"] {
        fs::write(dir.0.join(file), b"not a directory").unwrap();
    }
    fs::create_dir_all(dir.0.join("unreadable-alternates/info/alternates")).unwrap();
    let missing = [
        dir.c_path("missing"),
        dir.c_path("missing/objects"),
        CString::default(),
        // An alternates "file" that is a directory cannot be read.
        dir.c_path("unreadable-alternates"),
    ];
    for path in &missing {
        let c = c_open(path).map(drop);
        let rust = Odb::open(path).map(drop);
        assert!(c.is_err(), "{path:?}");
        assert_eq!(rust.map_err(|e| rust_err(&e)), c, "{path:?}");
        for kind in [OidType::Sha1, OidType::Sha256] {
            let c = c_open_ext(path, Some(&c_options(kind.raw()))).map(drop);
            let rust = Odb::open_with_oid_type(path, kind).map(drop);
            assert_eq!(rust.map_err(|e| rust_err(&e)), c, "{path:?}");
        }
    }
    // A regular file opens: its backends find nothing and cannot write.
    let files = (dir.c_path("c-file"), dir.c_path("rust-file"));
    let c = c_open(&files.0).unwrap();
    let rust = Odb::open(&files.1).unwrap();
    compare_blobs_in(c.0, &rust, Some((&files.0, &files.1)));
}

/// Alternates: another directory's objects are read through it; missing ones are
/// skipped; a directory naming itself, or a cycle, is opened once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_follows_alternates_as_git_odb_open_does() {
    init();
    let shared = TempDir::new("alternates-shared");
    let shared_objects = shared.0.join("objects");
    fs::create_dir_all(&shared_objects).unwrap();
    // Seed the shared directory with a blob, through C.
    let seed = c_open(&c_path(&shared_objects)).unwrap();
    let Seen::Written(seeded) = step(seed.0, Op::Write(b"from the alternate")) else {
        panic!("the shared directory takes a write")
    };
    drop(seed);
    let lines = format!(
        "# a comment\n\n{}\n/nonexistent/objects\n.\n../objects\n",
        shared_objects.display()
    );
    let dirs = Pair::new("alternates", |objects| {
        fs::create_dir_all(objects.join("info")).unwrap();
        fs::write(objects.join("info/alternates"), &lines).unwrap();
    });
    let c = c_open(&dirs.c).unwrap();
    let rust = Odb::open(&dirs.rust).unwrap();
    assert_eq!(step(rust.as_c_ptr(), Op::Backends), Seen::Backends(4));
    compare_blobs(c.0, &rust);
    for op in [Op::Header(seeded), Op::Exists(seeded)] {
        assert_eq!(step(rust.as_c_ptr(), op), step(c.0, op), "{op:?}");
    }
    assert_eq!(step(rust.as_c_ptr(), Op::Exists(seeded)), Seen::Exists(1));
}

/// One repository over an odb, freed on drop.
struct Repo(*mut ffi::git_repository);

impl Repo {
    /// `git_repository_wrap_odb` over a live odb, which the repository takes a
    /// reference to.
    fn wrap(odb: *mut ffi::git_odb) -> Self {
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a writable slot; `odb` is live.
        let ret = unsafe { ffi::git_repository_wrap_odb(&raw mut repo, odb) };
        assert_eq!(ret, 0);
        Self(repo)
    }

    /// A new reference to the repository's odb, as C holds it.
    fn odb(&self) -> COdb {
        let mut odb = ptr::null_mut();
        // SAFETY: `self.0` is live; the result is a new reference to its odb.
        assert_eq!(unsafe { ffi::git_repository_odb(&raw mut odb, self.0) }, 0);
        COdb(odb)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the live repository `wrap` made, freed once.
        unsafe { ffi::git_repository_free(self.0) };
    }
}

/// Dropping an `OdbArc` a repository shares leaves the repository the odb, as
/// `git_odb_free` does; and the odb outlives a repository freed first. In C the
/// caller's reference then works on, through the odb's own cache; Rust keeps the
/// reference tied to the repository (an `OdbArc<'r>`), since a thread still reading
/// through it would race the free (`odb_cache()` follows the owner pointer without a
/// reference), so the Rust side releases its reference before the repository.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_shared_odb_lives_as_long_as_in_c() {
    init();
    let dirs = Pair::new("shared", |_| {});
    // The caller's reference goes first.
    let c = c_open(&dirs.c).unwrap();
    let c_raw = c.0;
    let c_repo = Repo::wrap(c_raw);
    drop(c);
    let rust = Odb::open(&dirs.rust).unwrap();
    let rust_raw = rust.as_c_ptr();
    let rust_repo = Repo::wrap(rust_raw);
    drop(rust);
    let c = c_repo.odb();
    let rust = rust_repo.odb();
    assert_eq!(
        (c.0, rust.0),
        (c_raw, rust_raw),
        "the repository kept the same odb"
    );
    let blobs = blobs();
    for bytes in &blobs {
        assert_eq!(step(rust.0, Op::Write(bytes)), step(c.0, Op::Write(bytes)));
    }
    drop((c, rust));
    // Then the repository goes first: C's reference keeps the odb, and reads through
    // its own cache from then on. The Rust owner borrows the repository, so it is
    // released first, and the odb is reopened from disk to compare what C still reads.
    let c = c_repo.odb();
    let rust = {
        let raw = rust_repo.odb();
        let adopted = raw.0;
        let _ = core::mem::ManuallyDrop::new(raw);
        // The test's own reference, handed to the Rust owner the wrapper of
        // `git_repository_odb` will return, borrowing `rust_repo`.
        // SAFETY: `adopted` is the one reference `git_repository_odb` just returned, to
        // an odb `rust_repo` owns, which the result's borrow of `rust_repo` keeps alive.
        unsafe { OdbArc::<'_>::from_c(adopted) }.unwrap()
    };
    compare_blobs(c.0, &rust);
    drop(rust);
    drop((c_repo, rust_repo));
    let rust = Odb::open(&dirs.rust).unwrap();
    compare_blobs(c.0, &rust);
}
