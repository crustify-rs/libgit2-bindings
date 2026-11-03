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

mod stream;
mod sys;

use std::ffi::{CStr, CString, c_int};
use std::path::{Path, PathBuf};
use std::{fs, ptr};

use ffibox::CZeroable;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::odb::{Odb, OdbArc, OdbExpandId, OdbOptions};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
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

/// `git_odb_options_init` into a fresh `git_odb_options`: the return and what it wrote.
fn c_options_init(version: u32) -> (c_int, ffi::git_odb_options) {
    let mut opts = ffi::git_odb_options {
        version: 0xDEAD,
        oid_type: 0xBEEF,
    };
    // SAFETY: `opts` is a live `git_odb_options` the routine may write, not retained.
    let rc = unsafe { ffi::git_odb_options_init(&raw mut opts, version) };
    (rc, opts)
}

/// [`OdbOptions::new`] holds what `git_odb_options_init` writes for this version, which
/// is `GIT_ODB_OPTIONS_INIT`; and options a Rust value carries build, or fail to build,
/// the odb the same C options do.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn odb_options_are_what_c_initialises_and_reads() {
    init();
    let (rc, c) = c_options_init(ffi::GIT_ODB_OPTIONS_VERSION);
    assert_eq!(rc, 0);
    let rust = ffi::git_odb_options::from(OdbOptions::new());
    assert_eq!((rust.version, rust.oid_type), (c.version, c.oid_type));
    assert_eq!(
        (rust.version, rust.oid_type),
        (1, 0),
        "GIT_ODB_OPTIONS_INIT"
    );
    assert_eq!(OdbOptions::default(), OdbOptions::new());

    // Every type the setter takes, then every version, each step compared through the
    // odb `git_odb_new_ext` builds from the C options and from the Rust value.
    let mut opts = OdbOptions::new();
    let kinds = [None, Some(OidType::Sha1), Some(OidType::Sha256)];
    for kind in kinds {
        opts.as_mut().set_oid_type(kind);
        let c = c_new_ext(Some(&c_options(kind.map_or(0, OidType::raw)))).unwrap();
        let rust = c_new_ext(Some(&ffi::git_odb_options::from(opts))).unwrap();
        // SAFETY: `rust.0` is the one reference `git_odb_new_ext` returned, given up to
        // the owner here.
        let rust = unsafe { OdbArc::from_c(rust.into_raw()) }.unwrap();
        compare_blobs(c.0, &rust);
    }
    for version in [0, 2, u32::MAX, ffi::GIT_ODB_OPTIONS_VERSION] {
        opts.as_mut().set_version(version);
        let c_opts = ffi::git_odb_options {
            version,
            oid_type: ffi::GIT_OID_SHA256,
        };
        let c = c_new_ext(Some(&c_opts)).map(drop);
        let rust = c_new_ext(Some(&ffi::git_odb_options::from(opts))).map(drop);
        assert_eq!(rust, c, "version {version}");
        assert_eq!(c.is_ok(), version == 1, "version {version}");
        let c_init = c_options_init(version);
        assert_eq!(c_init.0 == 0, version == 1, "version {version}");
    }
}

impl COdb {
    /// Gives the reference up, unreleased.
    fn into_raw(self) -> *mut ffi::git_odb {
        core::mem::ManuallyDrop::new(self).0
    }
}

/// One query's fields: the id's type byte and bytes, the length and the type.
type QueryFields = (u8, [u8; 32], u16, ffi::git_object_t);

fn c_fields(q: &ffi::git_odb_expand_id) -> QueryFields {
    (q.id.type_, q.id.id, q.length, q.type_)
}

fn rust_fields(q: &OdbExpandId) -> QueryFields {
    let r = q.as_ref();
    (r.id().raw_kind(), r.id().id(), r.length(), r.raw_kind())
}

/// `git_odb_expand_ids` over a run of queries: the return and, after a failure, the
/// thread's error.
///
/// # Safety
///
/// `odb` is live, and `ids` addresses `count` writable `git_odb_expand_id`s.
unsafe fn expand(
    odb: *mut ffi::git_odb,
    ids: *mut ffi::git_odb_expand_id,
    count: usize,
) -> Result<(), (c_int, (c_int, String))> {
    // SAFETY: per the contract.
    let rc = unsafe { ffi::git_odb_expand_ids(odb, ids, count) };
    if rc == 0 { Ok(()) } else { Err((rc, c_last())) }
}

/// What [`expand`] answered for the C queries and for the Rust ones.
type Expanded = (
    Result<(), (c_int, (c_int, String))>,
    Result<(), (c_int, (c_int, String))>,
);

/// [`expand`] over the first `count` queries of each run (all of them for `None`): the C
/// ones on `c_odb`, the Rust ones on `rust_odb`.
fn expand_both(
    c_odb: *mut ffi::git_odb,
    rust_odb: &OdbArc<'_>,
    c: &mut [ffi::git_odb_expand_id],
    rust: &mut [OdbExpandId],
    count: Option<usize>,
) -> Expanded {
    let (c_count, rust_count) = count.map_or((c.len(), rust.len()), |n| (n, n));
    assert!(c_count <= c.len() && rust_count <= rust.len());
    // SAFETY: `c_odb` is live (the caller holds it); `c` holds at least `c_count`
    // writable queries, borrowed exclusively for the call.
    let c_rc = unsafe { expand(c_odb, c.as_mut_ptr(), c_count) };
    // SAFETY: the owner keeps `rust_odb` live; `rust` holds at least `rust_count`
    // writable queries, borrowed exclusively for the call, which `OdbExpandId` lays out
    // as `git_odb_expand_id`s (it is `repr(transparent)` over one).
    let rust_rc = unsafe { expand(rust_odb.as_c_ptr(), rust.as_mut_ptr().cast(), rust_count) };
    (c_rc, rust_rc)
}

/// The C queries and their Rust twins, built independently from the same description:
/// the C ones as struct literals, the Rust ones through [`OdbExpandId::new`] and the
/// setters.
fn queries(kind: OidType, stored: &[Id]) -> (Vec<ffi::git_odb_expand_id>, Vec<OdbExpandId>) {
    let hex = u16::try_from(kind.hex_size()).unwrap();
    let missing = {
        let mut id = stored[0];
        id.1[..kind.size()].iter_mut().for_each(|b| *b ^= 0xA5);
        id
    };
    // (id, length, the type as C stores it, the type the Rust setter takes)
    let mut plan: Vec<(Id, u16, ffi::git_object_t, Option<Option<ObjectType>>)> = Vec::new();
    for &id in stored {
        plan.extend([
            (id, hex, ffi::GIT_OBJECT_ANY, Some(None)),
            (id, hex, ffi::GIT_OBJECT_BLOB, Some(Some(ObjectType::Blob))),
            (
                id,
                hex,
                ffi::GIT_OBJECT_COMMIT,
                Some(Some(ObjectType::Commit)),
            ),
            (id, 8, 0, None),
            (id, 8, ffi::GIT_OBJECT_BLOB, Some(Some(ObjectType::Blob))),
            (id, 8, ffi::GIT_OBJECT_TREE, Some(Some(ObjectType::Tree))),
            (id, 4, ffi::GIT_OBJECT_ANY, Some(None)),
            (id, 3, ffi::GIT_OBJECT_ANY, Some(None)),
            (id, hex - 1, ffi::GIT_OBJECT_ANY, Some(None)),
            (id, u16::MAX, ffi::GIT_OBJECT_ANY, Some(None)),
        ]);
    }
    plan.extend([
        (missing, hex, ffi::GIT_OBJECT_ANY, Some(None)),
        (missing, 8, ffi::GIT_OBJECT_ANY, Some(None)),
        ((0, [0; 32]), 0, 0, None),
    ]);
    let c = plan
        .iter()
        .map(|&((type_, id), length, kind, _)| ffi::git_odb_expand_id {
            id: ffi::git_oid { type_, id },
            length,
            type_: kind,
        })
        .collect();
    let rust = plan
        .iter()
        .map(|&((type_, id), length, raw, kind)| {
            let size = OidType::from_raw(type_.into()).map_or(0, OidType::size);
            let mut q = if size == 0 {
                OdbExpandId::zeroed()
            } else {
                let oid =
                    Oid::from_raw(&id[..size], OidType::from_raw(type_.into()).unwrap()).unwrap();
                OdbExpandId::new(oid.as_ref(), length, None)
            };
            match kind {
                Some(kind) => q.as_mut().set_kind(kind),
                None => assert_eq!(raw, 0),
            }
            if size == 0 {
                q.as_mut().set_length(length);
            } else if kind.is_none() {
                // Leave the type 0, as the C query does: the routine reads it as any.
                let mut raw_q = ffi::git_odb_expand_id::from(q);
                raw_q.type_ = 0;
                q = OdbExpandId::from(raw_q);
            }
            q
        })
        .collect();
    (c, rust)
}

/// Runs the queries on both odbs, then again over what the first pass wrote, then after
/// narrowing every query to an 8-digit prefix of any type, comparing every query's
/// fields and the routine's result after each pass.
fn compare_expand(
    c_odb: *mut ffi::git_odb,
    rust_odb: &OdbArc<'_>,
    kind: OidType,
    stored: &[Id],
    found: bool,
) {
    let (mut c, mut rust) = queries(kind, stored);
    assert_eq!(c.len(), rust.len());
    let compare = |c: &[ffi::git_odb_expand_id], rust: &[OdbExpandId], pass: &str| {
        for (i, (c, rust)) in c.iter().zip(rust).enumerate() {
            assert_eq!(rust_fields(rust), c_fields(c), "{pass}: query {i}");
        }
    };
    compare(&c, &rust, "built");
    for pass in ["first", "again"] {
        let (c_rc, rust_rc) = expand_both(c_odb, rust_odb, &mut c, &mut rust, None);
        assert_eq!(rust_rc, c_rc, "{pass}");
        compare(&c, &rust, pass);
        // Per stored id: the full id untyped and as a blob, the 8-digit prefixes untyped
        // and as a blob, the 4-digit and the one-short ones, are found (a found prefix
        // takes the full length; a length past it is kept); a wrong type and a 3-digit
        // prefix are cleared. Without backends, everything is cleared.
        for (n, chunk) in rust.chunks(10).take(stored.len()).enumerate() {
            let hex = u16::try_from(kind.hex_size()).unwrap();
            let lengths: Vec<u16> = chunk.iter().map(|q| q.as_ref().length()).collect();
            let want = |f: u16| if found { f } else { 0 };
            assert_eq!(
                lengths,
                [
                    want(hex),
                    want(hex),
                    0,
                    want(hex),
                    want(hex),
                    0,
                    want(hex),
                    0,
                    want(hex),
                    want(u16::MAX)
                ],
                "{pass}: stored id {n}"
            );
            if found {
                assert_eq!(chunk[3].as_ref().kind(), Some(ObjectType::Blob));
                assert_eq!(chunk[3].as_ref().id().id(), stored[n].1);
            }
        }
    }
    for (c, rust) in c.iter_mut().zip(&mut rust) {
        c.length = 8;
        c.type_ = ffi::GIT_OBJECT_ANY;
        let mut m = rust.as_mut();
        m.set_length(8);
        m.set_kind(None);
    }
    compare(&c, &rust, "narrowed");
    let (c_rc, rust_rc) = expand_both(c_odb, rust_odb, &mut c, &mut rust, None);
    assert_eq!(rust_rc, c_rc, "narrowed");
    compare(&c, &rust, "narrowed, expanded");
    // A run of no queries changes nothing and succeeds.
    let (c_rc, rust_rc) = expand_both(c_odb, rust_odb, &mut c, &mut rust, Some(0));
    assert_eq!((rust_rc, c_rc), (Ok(()), Ok(())));
}

/// Queries built through [`OdbExpandId`] are answered by `git_odb_expand_ids` exactly as
/// C-built ones: full ids and prefixes found, typed and untyped, a wrong type, a prefix
/// too short, a missing id and an empty query cleared; over every id type, on an odb
/// with backends and one without.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn expand_ids_answers_rust_queries_as_c_queries() {
    init();
    for kind in [OidType::Sha1, OidType::Sha256] {
        let dirs = Pair::new(&format!("expand-{}", kind.raw()), |_| {});
        let c = c_open_ext(&dirs.c, Some(&c_options(kind.raw()))).unwrap();
        let rust = Odb::open_with_oid_type(&dirs.rust, kind).unwrap();
        let mut stored = Vec::new();
        for blob in blobs() {
            let seen = step(c.0, Op::Write(&blob));
            assert_eq!(step(rust.as_c_ptr(), Op::Write(&blob)), seen);
            let Seen::Written(id) = seen else {
                panic!("an on-disk odb writes")
            };
            stored.push(id);
        }
        compare_expand(c.0, &rust, kind, &stored, true);

        // No backends: nothing is found, and every query is cleared alike.
        let c = c_new_ext(Some(&c_options(kind.raw()))).unwrap();
        let rust = Odb::with_oid_type(kind).unwrap();
        compare_expand(c.0, &rust, kind, &stored, false);
    }
}
