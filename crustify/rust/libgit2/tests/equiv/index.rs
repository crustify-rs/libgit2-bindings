//! Equivalence tests for `index/index.rs`: `Index::new` / `Index::open` against
//! `git_index_new` / `git_index_open` on independently owned indexes, compared on their
//! verdicts, errors and the state the index then reports, after each step of a
//! sequence; and the owners' release against `git_index_free` around a repository.

use std::ffi::{CStr, CString, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::index::index::{Index, IndexArc, IndexBox};
use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository};

fn init() {
    core::mem::forget(libgit2::libgit2::core::global::init().unwrap());
}

fn clear() {
    // SAFETY: the runtime is initialised; clears this thread's error.
    unsafe { ffi::git_error_clear() }
}

/// The thread's last error, as `(class, message)`.
fn c_last() -> (c_int, CString) {
    // SAFETY: the runtime is initialised; the error is copied before any other call.
    unsafe {
        let e = ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_owned())
    }
}

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).unwrap()
}

/// A scratch directory for one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("crustify-index-equiv-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a constructor's caller can observe: the error of a failure, or the index's
/// path and entries.
#[derive(Debug, PartialEq)]
enum Seen {
    Failed {
        class: c_int,
        message: CString,
    },
    Built {
        path: Option<CString>,
        entries: Vec<(CString, u16)>,
        owned_by_repository: bool,
    },
}

/// What `index` reports.
fn state(index: *mut ffi::git_index) -> Seen {
    // SAFETY: a live index, only read; every string is copied before the next call.
    unsafe {
        let path = ffi::git_index_path(index);
        let entries = (0..ffi::git_index_entrycount(index))
            .map(|i| {
                let e = ffi::git_index_get_byindex(index, i);
                (CStr::from_ptr((*e).path).to_owned(), (*e).flags)
            })
            .collect();
        Seen::Built {
            path: (!path.is_null()).then(|| CStr::from_ptr(path).to_owned()),
            entries,
            owned_by_repository: !ffi::git_index_owner(index).is_null(),
        }
    }
}

/// One C-owned index, freed on drop.
struct CIndex(*mut ffi::git_index);

impl Drop for CIndex {
    fn drop(&mut self) {
        // SAFETY: the reference `git_index_new` / `git_index_open` handed out, freed once.
        unsafe { ffi::git_index_free(self.0) }
    }
}

/// `git_index_open(path)` (`git_index_new` for `None`), observed; asserts that a
/// failure is libgit2's generic `-1`, which `Index::open`'s error stands for, and leaves
/// the slot NULL.
fn c_open(path: Option<&CStr>) -> (Seen, Option<CIndex>) {
    clear();
    let mut out = ptr::null_mut();
    // SAFETY: a live slot and NULL or a NUL-terminated path, read for the call.
    let rc = unsafe {
        match path {
            Some(path) => ffi::git_index_open(&mut out, path.as_ptr()),
            None => ffi::git_index_new(&mut out),
        }
    };
    if rc < 0 {
        assert_eq!(rc, -1);
        assert!(out.is_null());
        let (class, message) = c_last();
        (Seen::Failed { class, message }, None)
    } else {
        assert_eq!(rc, 0);
        (state(out), Some(CIndex(out)))
    }
}

/// `Index::open(path)` (`Index::new` for `None`), observed.
fn rust_open(path: Option<&CStr>) -> (Seen, Option<IndexBox>) {
    clear();
    let built = match path {
        Some(path) => Index::open(path),
        None => Index::new(),
    };
    match built {
        Ok(index) => (state(index.as_c_ptr()), Some(index)),
        Err(e) => (
            Seen::Failed {
                class: e.raw_class(),
                message: e.message().to_owned(),
            },
            None,
        ),
    }
}

/// `git_index_add` of a regular file at `path`, then the status and the index's state.
fn add(index: *mut ffi::git_index, path: &CStr) -> (c_int, Seen) {
    let mut entry = ffi::git_index_entry {
        mode: 0o100644,
        path: path.as_ptr(),
        ..Default::default()
    };
    entry.id.id[0] = 7;
    entry.id.type_ = ffi::GIT_OID_SHA1 as _;
    // SAFETY: a live index; `entry` and its path are read and copied for the call.
    let rc = unsafe { ffi::git_index_add(index, &entry) };
    (rc, state(index))
}

/// `git_index_write`, then the status and the thread's error when it failed.
fn write(index: *mut ffi::git_index) -> (c_int, Option<(c_int, CString)>) {
    clear();
    // SAFETY: a live index.
    let rc = unsafe { ffi::git_index_write(index) };
    (rc, (rc < 0).then(c_last))
}

/// The bytes of a valid index file with `n` entries, as libgit2 writes it.
fn index_bytes(scratch: &Scratch, n: usize) -> Vec<u8> {
    let file = c_path(&scratch.file("source.index"));
    let (_, index) = c_open(Some(&file));
    let index = index.unwrap();
    for i in 0..n {
        let name = CString::new(format!("d{}/f{i}", i % 2)).unwrap();
        assert_eq!(add(index.0, &name).0, 0);
    }
    assert_eq!(write(index.0).0, 0);
    drop(index);
    let file = scratch.file("source.index");
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    bytes
}

/// The bytes of a valid version-`version` index file, as libgit2 writes it: four
/// entries, plus an intent-to-add one (a version 3 entry) from version 3 on.
fn versioned_index_bytes(scratch: &Scratch, version: u32) -> Vec<u8> {
    let file = scratch.file("versioned.index");
    let (_, index) = c_open(Some(&c_path(&file)));
    let index = index.unwrap();
    for name in [c"a", c"d0/f0", c"d0/f1", c"d1/x"] {
        assert_eq!(add(index.0, name).0, 0);
    }
    if version >= 3 {
        let mut entry = ffi::git_index_entry {
            mode: 0o100644,
            path: c"d1/new".as_ptr(),
            flags_extended: ffi::GIT_INDEX_ENTRY_INTENT_TO_ADD as u16,
            ..Default::default()
        };
        entry.id.type_ = ffi::GIT_OID_SHA1 as _;
        // SAFETY: a live index; `entry` and its path are read and copied for the call.
        assert_eq!(unsafe { ffi::git_index_add(index.0, &entry) }, 0);
    }
    // SAFETY: a live index.
    assert_eq!(unsafe { ffi::git_index_set_version(index.0, version) }, 0);
    assert_eq!(write(index.0).0, 0);
    drop(index);
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_file(&file).unwrap();
    bytes
}

/// `bytes` (a whole index file) with `extensions` appended after its entries, and a
/// zero checksum, which libgit2 accepts (`index.skipHash`).
fn with_extensions(bytes: &[u8], extensions: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = bytes[..bytes.len() - 20].to_vec();
    for (signature, data) in extensions {
        out.extend_from_slice(&signature[..]);
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&[0; 20]);
    out
}

/// A tree cache over `d0` and an invalidated `d1`; one resolved conflict; one
/// conflict's names; an optional extension libgit2 keeps unread.
fn all_extensions(bytes: &[u8]) -> Vec<u8> {
    let tree = [
        &b"\x005 2\n"[..],
        &[0x11; 20],
        b"d0\x002 0\n",
        &[0x22; 20],
        b"d1\x00-1 0\n",
    ]
    .concat();
    let reuc = [
        &b"a\x00100644\x00100644\x000\x00"[..],
        &[0x33; 20],
        &[0x44; 20],
    ]
    .concat();
    with_extensions(
        bytes,
        &[
            (b"TREE", &tree),
            (b"REUC", &reuc),
            (b"NAME", b"anc\x00ours\x00\x00"),
            (b"ZOPT", b"kept as is"),
        ],
    )
}

/// Both open the file at `path` as it is now; they agree.
fn assert_same_open(path: &CStr, what: &str) -> Seen {
    let (c, _c_index) = c_open(Some(path));
    let (rust, _rust_index) = rust_open(Some(path));
    assert_eq!(rust, c, "{what}");
    rust
}

/// A fresh in-memory index, then a sequence of edits on each, compared after every
/// step; neither can be written.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_matches_git_index_new() {
    init();
    let (c, c_index) = c_open(None);
    let (rust, mut rust_index) = rust_open(None);
    assert_eq!(rust, c);
    assert_eq!(
        rust,
        Seen::Built {
            path: None,
            entries: vec![],
            owned_by_repository: false
        }
    );
    let c_index = c_index.unwrap();
    let rust_index = rust_index.as_mut().unwrap();
    for name in [c"b", c"a", c"a", c"dir/c", c""] {
        assert_eq!(
            add(rust_index.as_c_ptr(), name),
            add(c_index.0, name),
            "{name:?}"
        );
    }
    let (rc, error) = write(rust_index.as_c_ptr());
    assert!(rc < 0);
    assert_eq!((rc, error), write(c_index.0));
}

/// Opening every kind of file, valid or not: missing, a directory, empty,
/// garbage, a valid index, each of its truncations and corruptions.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_matches_git_index_open() {
    init();
    let scratch = Scratch::new("open");
    let missing = assert_same_open(&c_path(&scratch.file("missing")), "missing");
    assert!(matches!(missing, Seen::Built { ref entries, .. } if entries.is_empty()));
    // An empty path is a deliberate divergence (`Index::open` refuses it; writing the
    // index C opens is undefined behaviour), checked in the module's unit tests.
    let dir = assert_same_open(&c_path(&scratch.0), "directory");
    assert!(matches!(dir, Seen::Failed { .. }));

    let file = scratch.file("index");
    let c_file = c_path(&file);
    for (what, bytes) in [
        ("empty", &b""[..]),
        (
            "garbage",
            &b"DIRC but then nothing that parses at all......"[..],
        ),
    ] {
        std::fs::write(&file, bytes).unwrap();
        assert!(matches!(
            assert_same_open(&c_file, what),
            Seen::Failed { .. }
        ));
    }

    let bytes = index_bytes(&scratch, 5);
    std::fs::write(&file, &bytes).unwrap();
    let valid = assert_same_open(&c_file, "valid");
    assert!(matches!(valid, Seen::Built { ref entries, .. } if entries.len() == 5));

    let stride = if cfg!(bsan) { 7 } else { 1 };
    for len in (0..bytes.len()).step_by(stride) {
        std::fs::write(&file, &bytes[..len]).unwrap();
        assert_same_open(&c_file, &format!("truncated to {len}"));
    }
    let body = bytes.len() - 20;
    for at in (0..body).step_by(stride) {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 0xff;
        std::fs::write(&file, &corrupt).unwrap();
        assert_same_open(&c_file, &format!("byte {at} flipped"));
        corrupt[body..].fill(0);
        std::fs::write(&file, &corrupt).unwrap();
        assert_same_open(&c_file, &format!("byte {at} flipped, no checksum"));
    }
}

/// Opening index files of every version, with every extension, valid or refused, and
/// each truncation and corruption of the version 2 and 3 ones with every extension.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_matches_on_every_version_and_extension() {
    init();
    let scratch = Scratch::new("versions");
    let file = scratch.file("index");
    let c_file = c_path(&file);
    for version in [2, 3, 4] {
        let bytes = versioned_index_bytes(&scratch, version);
        let mut extension_size = with_extensions(&bytes, &[(b"ZOPT", b"data")]);
        let at = extension_size.len() - 20 - b"data".len() - 8;
        extension_size[at + 4..at + 8].copy_from_slice(&1000u32.to_be_bytes());
        for (what, contents) in [
            ("plain", bytes.clone()),
            ("all extensions", all_extensions(&bytes)),
            (
                "mandatory",
                with_extensions(&bytes, &[(b"link", b"mandatory")]),
            ),
            (
                "short reuc",
                with_extensions(&bytes, &[(b"REUC", b"a\x00")]),
            ),
            (
                "bad tree",
                with_extensions(&bytes, &[(b"TREE", b"\x00x 0\n")]),
            ),
            ("extension size", extension_size),
        ] {
            std::fs::write(&file, &contents).unwrap();
            let seen = assert_same_open(&c_file, &format!("version {version}, {what}"));
            let built = matches!(seen, Seen::Built { .. });
            assert_eq!(built, ["plain", "all extensions"].contains(&what), "{what}");
        }
    }

    // An entry count above the entries the file holds.
    let mut bytes = versioned_index_bytes(&scratch, 2);
    bytes[11] += 1;
    let body = bytes.len() - 20;
    bytes[body..].fill(0);
    std::fs::write(&file, &bytes).unwrap();
    assert!(matches!(
        assert_same_open(&c_file, "count too high"),
        Seen::Failed { .. }
    ));

    for version in [2, 3] {
        sweep(
            &scratch,
            &all_extensions(&versioned_index_bytes(&scratch, version)),
        );
    }
}

/// Paths of 0xFFF bytes and more, which version 2 entries flag as "look for the NUL"
/// and version 4 refuses past `GIT_PATH_MAX`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_matches_on_long_paths() {
    init();
    let scratch = Scratch::new("long-paths");
    let file = scratch.file("index");
    let c_file = c_path(&file);
    for (version, len, built) in [(2, 0xfff, true), (2, 4200, true), (4, 4200, false)] {
        let _ = std::fs::remove_file(&file);
        let (_, index) = c_open(Some(&c_file));
        let index = index.unwrap();
        let long = CString::new(format!("d/{}", "x".repeat(len - 2))).unwrap();
        assert_eq!(add(index.0, c"a").0, 0);
        assert_eq!(add(index.0, &long).0, 0);
        // SAFETY: a live index.
        assert_eq!(unsafe { ffi::git_index_set_version(index.0, version) }, 0);
        assert_eq!(write(index.0).0, 0);
        drop(index);
        let seen = assert_same_open(&c_file, &format!("version {version}, {len} bytes"));
        assert_eq!(matches!(seen, Seen::Built { .. }), built, "{seen:?}");
    }
}

/// Both open every truncation and single-byte corruption of `bytes`, an index file;
/// they agree on each.
fn sweep(scratch: &Scratch, bytes: &[u8]) {
    let file = scratch.file("index");
    let c_file = c_path(&file);
    let stride = if cfg!(bsan) { 7 } else { 1 };
    for len in (0..bytes.len()).step_by(stride) {
        std::fs::write(&file, &bytes[..len]).unwrap();
        assert_same_open(&c_file, &format!("truncated to {len}"));
    }
    for at in (0..bytes.len() - 20).step_by(stride) {
        for flip in [0x01, 0x30, 0xff] {
            let mut corrupt = bytes.to_vec();
            corrupt[at] ^= flip;
            std::fs::write(&file, &corrupt).unwrap();
            assert_same_open(&c_file, &format!("byte {at} ^ {flip:#x}"));
        }
    }
}

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process with LeakSanitizer off: libgit2 leaks the path
/// of a version 4 entry that runs past the end of the file (`read_entry` in
/// `src/libgit2/index.c`), on either side.
fn isolated_tolerating_leaks(test: &str, body: fn()) {
    let name = test.strip_prefix("equiv::").unwrap_or(test);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
    let separator = if inherited.is_empty() { "" } else { ":" };
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .env(
            "ASAN_OPTIONS",
            format!("{inherited}{separator}detect_leaks=0"),
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Each truncation and corruption of a version 4 index carrying every extension.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_matches_on_corrupted_version_4_files() {
    isolated_tolerating_leaks(
        concat!(
            module_path!(),
            "::open_matches_on_corrupted_version_4_files"
        ),
        || {
            init();
            let scratch = Scratch::new("corrupted-v4");
            sweep(
                &scratch,
                &all_extensions(&versioned_index_bytes(&scratch, 4)),
            );
        },
    );
}

/// Open a missing file, edit, write, reopen, edit again: each side on its own file,
/// compared after every step.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_edit_write_reopen_sequence_matches() {
    init();
    let c_dir = Scratch::new("sequence-c");
    let rust_dir = Scratch::new("sequence-rust");
    let c_file = c_path(&c_dir.file("index"));
    let rust_file = c_path(&rust_dir.file("index"));
    let relative = |seen: Seen, file: &CStr| match seen {
        Seen::Built {
            path,
            entries,
            owned_by_repository,
        } => {
            assert_eq!(path.as_deref(), Some(file));
            (entries, owned_by_repository)
        }
        Seen::Failed { .. } => panic!("{seen:?}"),
    };

    let (c, c_index) = c_open(Some(&c_file));
    let (rust, rust_index) = rust_open(Some(&rust_file));
    assert_eq!(relative(rust, &rust_file), relative(c, &c_file));
    let c_index = c_index.unwrap();
    let rust_index = rust_index.unwrap();
    for name in [c"z", c"m/n", c"a"] {
        let (c_rc, c) = add(c_index.0, name);
        let (rust_rc, rust) = add(rust_index.as_c_ptr(), name);
        assert_eq!(
            (rust_rc, relative(rust, &rust_file)),
            (c_rc, relative(c, &c_file))
        );
    }
    assert_eq!(write(rust_index.as_c_ptr()), write(c_index.0));
    assert_eq!(
        std::fs::read(rust_dir.file("index")).unwrap(),
        std::fs::read(c_dir.file("index")).unwrap()
    );

    let (c, c_again) = c_open(Some(&c_file));
    let (rust, rust_again) = rust_open(Some(&rust_file));
    assert_eq!(relative(rust, &rust_file), relative(c, &c_file));
    drop((c_index, rust_index));
    let (c_again, rust_again) = (c_again.unwrap(), rust_again.unwrap());
    let (c_rc, c) = add(c_again.0, c"b");
    let (rust_rc, rust) = add(rust_again.as_c_ptr(), c"b");
    assert_eq!(
        (rust_rc, relative(rust, &rust_file)),
        (c_rc, relative(c, &c_file))
    );
}

/// A scratch bare repository, freed and removed on drop.
struct Repo {
    _dir: Scratch,
    repo: *mut ffi::git_repository,
}

impl Repo {
    fn new(tag: &str) -> Self {
        let dir = Scratch::new(tag);
        let path = c_path(&dir.0);
        let mut opts = InitOptions::new();
        opts.as_mut().set_flags(InitFlag::Bare.into());
        // The raw repository both sides' calls use; `Drop` frees it.
        let repo = Repository::init_ext(&path, opts.as_mut()).unwrap().into_c();
        Self { _dir: dir, repo }
    }

    /// `git_repository_set_index`.
    fn set_index(&self, index: *mut ffi::git_index) {
        // SAFETY: a live repository and index; libgit2 counts the reference it keeps.
        let rc = unsafe { ffi::git_repository_set_index(self.repo, index) };
        assert_eq!(rc, 0);
    }

    /// `git_repository_index`'s raw reference.
    fn index(&self) -> *mut ffi::git_index {
        let mut out = ptr::null_mut();
        // SAFETY: a live slot and repository.
        assert_eq!(unsafe { ffi::git_repository_index(&mut out, self.repo) }, 0);
        out
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // SAFETY: the repository `new` made, freed once.
        unsafe { ffi::git_repository_free(self.repo) }
    }
}

/// A built index handed to a repository: C keeps its pointer and frees it, Rust shares
/// the box as an arc and drops it. Both then reach the same index through the
/// repository, which owns it until it is freed; then the remaining reference does.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn shared_index_lifecycle_matches_git_index_free() {
    init();
    let c_repo = Repo::new("lifecycle-c");
    let rust_repo = Repo::new("lifecycle-rust");

    let (_, c_index) = c_open(None);
    let c_index = c_index.unwrap();
    let (_, rust_index) = rust_open(None);
    let rust_index: IndexArc = rust_index.unwrap().into();
    assert_eq!(state(rust_index.as_c_ptr()), state(c_index.0));

    c_repo.set_index(c_index.0);
    rust_repo.set_index(rust_index.as_c_ptr());
    assert_eq!(state(rust_index.as_c_ptr()), state(c_index.0));
    assert!(matches!(
        state(c_index.0),
        Seen::Built {
            owned_by_repository: true,
            ..
        }
    ));
    let c_ptr = c_index.0;
    let rust_ptr = rust_index.as_c_ptr();

    // Each side lets go of its own reference; the repository's keeps the index.
    drop(c_index);
    drop(rust_index);
    let c_again = CIndex(c_repo.index());
    let rust_again = {
        let raw = rust_repo.index();
        // SAFETY: `git_repository_index` hands out one more counted reference, released
        // by `git_index_free` as `IndexFree` does.
        unsafe { IndexArc::from_c(raw) }.unwrap()
    };
    assert_eq!((c_again.0, rust_again.as_c_ptr()), (c_ptr, rust_ptr));
    assert_eq!(state(rust_again.as_c_ptr()), state(c_again.0));
    let (c_rc, c) = add(c_again.0, c"after");
    let (rust_rc, rust) = add(rust_again.as_c_ptr(), c"after");
    assert_eq!((rust_rc, rust), (c_rc, c));

    // The repository goes first; the remaining reference owns the index alone.
    drop(c_repo);
    drop(rust_repo);
    assert_eq!(state(rust_again.as_c_ptr()), state(c_again.0));
    assert!(matches!(
        state(c_again.0),
        Seen::Built {
            owned_by_repository: false,
            ..
        }
    ));
}

/// `Index::with_oid_type` / `Index::open_with_oid_type` against `git_index_new_ext` /
/// `git_index_open_ext` (repository wave 0 type review).
mod oid_types {
    use super::*;
    use libgit2::libgit2::oid::oid::OidType;

    const TYPES: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

    /// `git_index_open_ext(path, {VERSION, oid_type})` (`git_index_new_ext` for `None`),
    /// observed as [`c_open`] observes.
    fn c_open_ext(path: Option<&CStr>, oid_type: OidType) -> (Seen, Option<CIndex>) {
        clear();
        let opts = ffi::git_index_options {
            version: ffi::GIT_INDEX_OPTIONS_VERSION,
            oid_type: oid_type.raw(),
        };
        let mut out = ptr::null_mut();
        // SAFETY: a live slot, NULL or a NUL-terminated path, and live options of this
        // version, all read for the call.
        let rc = unsafe {
            match path {
                Some(path) => ffi::git_index_open_ext(&mut out, path.as_ptr(), &opts),
                None => ffi::git_index_new_ext(&mut out, &opts),
            }
        };
        if rc < 0 {
            assert_eq!(rc, -1);
            assert!(out.is_null());
            let (class, message) = c_last();
            (Seen::Failed { class, message }, None)
        } else {
            assert_eq!(rc, 0);
            (state(out), Some(CIndex(out)))
        }
    }

    /// `Index::open_with_oid_type(path, oid_type)` (`Index::with_oid_type` for `None`),
    /// observed.
    fn rust_open_ext(path: Option<&CStr>, oid_type: OidType) -> (Seen, Option<IndexBox>) {
        clear();
        let built = match path {
            Some(path) => Index::open_with_oid_type(path, oid_type),
            None => Index::with_oid_type(oid_type),
        };
        match built {
            Ok(index) => (state(index.as_c_ptr()), Some(index)),
            Err(e) => (
                Seen::Failed {
                    class: e.raw_class(),
                    message: e.message().to_owned(),
                },
                None,
            ),
        }
    }

    /// `git_index_add` of a regular file at `path` whose id is of `oid_type`.
    fn add_typed(index: *mut ffi::git_index, path: &CStr, oid_type: OidType) -> (c_int, Seen) {
        let mut entry = ffi::git_index_entry {
            mode: 0o100644,
            path: path.as_ptr(),
            ..Default::default()
        };
        entry.id.id = [0x5a; 32];
        entry.id.type_ = oid_type.raw() as _;
        // SAFETY: a live index; `entry` and its path are read and copied for the call.
        let rc = unsafe { ffi::git_index_add(index, &entry) };
        (rc, state(index))
    }

    /// The entries of a built index, with its path checked against `file`.
    fn relative(seen: Seen, file: &CStr) -> (Vec<(CString, u16)>, bool) {
        match seen {
            Seen::Built {
                path,
                entries,
                owned_by_repository,
            } => {
                assert_eq!(path.as_deref(), Some(file));
                (entries, owned_by_repository)
            }
            Seen::Failed { .. } => panic!("{seen:?}"),
        }
    }

    /// In-memory indexes of each type match `git_index_new_ext`'s, through adding
    /// entries of that type; `Index::new` is the SHA1 form.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn with_oid_type_matches_git_index_new_ext() {
        init();
        for kind in TYPES {
            let (c, c_index) = c_open_ext(None, kind);
            let (rust, rust_index) = rust_open_ext(None, kind);
            assert_eq!(rust, c, "{kind:?}");
            let (c_index, rust_index) = (c_index.unwrap(), rust_index.unwrap());
            for name in [c"b", c"a/x", c"a"] {
                assert_eq!(
                    add_typed(rust_index.as_c_ptr(), name, kind),
                    add_typed(c_index.0, name, kind),
                    "{kind:?} {name:?}"
                );
            }
        }
        let (c, _c) = c_open(None);
        let (rust, _rust) = rust_open_ext(None, OidType::Sha1);
        assert_eq!(rust, c);
    }

    /// On-disk indexes of each type match `git_index_open_ext`'s: the file written by
    /// one side is byte for byte the other's, reopens as the same type on both, and is
    /// refused as the other type by both.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_with_oid_type_matches_git_index_open_ext() {
        init();
        for kind in TYPES {
            let other = match kind {
                OidType::Sha1 => OidType::Sha256,
                _ => OidType::Sha1,
            };
            let c_dir = Scratch::new(&format!("typed-c-{kind:?}"));
            let rust_dir = Scratch::new(&format!("typed-rust-{kind:?}"));
            let c_file = c_path(&c_dir.file("index"));
            let rust_file = c_path(&rust_dir.file("index"));

            let (c, c_index) = c_open_ext(Some(&c_file), kind);
            let (rust, rust_index) = rust_open_ext(Some(&rust_file), kind);
            assert_eq!(relative(rust, &rust_file), relative(c, &c_file));
            let (c_index, rust_index) = (c_index.unwrap(), rust_index.unwrap());
            for name in [c"z", c"m/n", c"a"] {
                let (c_rc, c) = add_typed(c_index.0, name, kind);
                let (rust_rc, rust) = add_typed(rust_index.as_c_ptr(), name, kind);
                assert_eq!(
                    (rust_rc, relative(rust, &rust_file)),
                    (c_rc, relative(c, &c_file))
                );
            }
            assert_eq!(write(rust_index.as_c_ptr()), write(c_index.0));
            let bytes = std::fs::read(c_dir.file("index")).unwrap();
            assert_eq!(std::fs::read(rust_dir.file("index")).unwrap(), bytes);
            drop((c_index, rust_index));

            // Reopened as its own type, then as the other, then as `open` reads it.
            let (c, _c) = c_open_ext(Some(&c_file), kind);
            let (rust, _rust) = rust_open_ext(Some(&rust_file), kind);
            assert_eq!(relative(rust, &rust_file), relative(c, &c_file));
            let (c, _c) = c_open_ext(Some(&c_file), other);
            let (rust, _rust) = rust_open_ext(Some(&rust_file), other);
            assert_eq!(rust, c, "{kind:?} read as {other:?}");
            let (c, _c) = c_open(Some(&c_file));
            let (rust, _rust) = rust_open(Some(&rust_file));
            match (&rust, &c) {
                (Seen::Built { .. }, Seen::Built { .. }) => {
                    assert_eq!(kind, OidType::Sha1, "read by open");
                    assert_eq!(relative(rust, &rust_file), relative(c, &c_file));
                }
                _ => assert_eq!(rust, c, "{kind:?} read by open"),
            }
        }
    }

    /// A missing file opens empty as its type; a directory fails alike; the empty path
    /// is refused by Rust alone, as `open` refuses it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_with_oid_type_fails_as_git_index_open_ext_fails() {
        init();
        let dir = Scratch::new("typed-failures");
        let missing = c_path(&dir.file("missing"));
        let directory = c_path(&dir.0);
        for kind in TYPES {
            for path in [&missing, &directory] {
                let (c, _c) = c_open_ext(Some(path), kind);
                let (rust, _rust) = rust_open_ext(Some(path), kind);
                assert_eq!(rust, c, "{kind:?} {path:?}");
            }
            let (rust, none) = rust_open_ext(Some(c""), kind);
            assert!(none.is_none());
            assert_eq!(
                rust,
                Seen::Failed {
                    class: ffi::GIT_ERROR_INDEX as c_int,
                    message: c"failed to open index: the path is empty".to_owned(),
                }
            );
        }
    }
}

/// The resource-free types' constants and every combination retain the values
/// published by the C header, including the signed owner-command sentinel.
#[test]
fn scalar_options_match_c_values() {
    use libgit2::libgit2::index::index::{AddFlags, Capabilities, CapabilitySetting};

    assert_eq!(AddFlags::DEFAULT.bits(), ffi::GIT_INDEX_ADD_DEFAULT);
    let add = [
        AddFlags::FORCE,
        AddFlags::DISABLE_PATHSPEC_MATCH,
        AddFlags::CHECK_PATHSPEC,
    ];
    let c_add = [
        ffi::GIT_INDEX_ADD_FORCE,
        ffi::GIT_INDEX_ADD_DISABLE_PATHSPEC_MATCH,
        ffi::GIT_INDEX_ADD_CHECK_PATHSPEC,
    ];
    let caps = [
        Capabilities::IGNORE_CASE,
        Capabilities::NO_FILEMODE,
        Capabilities::NO_SYMLINKS,
    ];
    let c_caps = [
        ffi::GIT_INDEX_CAPABILITY_IGNORE_CASE,
        ffi::GIT_INDEX_CAPABILITY_NO_FILEMODE,
        ffi::GIT_INDEX_CAPABILITY_NO_SYMLINKS,
    ];
    for mask in 0..8 {
        let (mut rust_add, mut raw_add) = (AddFlags::empty(), 0);
        let (mut rust_caps, mut raw_caps) = (Capabilities::empty(), 0);
        for bit in 0..3 {
            if mask & (1 << bit) != 0 {
                rust_add |= add[bit];
                raw_add |= c_add[bit];
                rust_caps |= caps[bit];
                raw_caps |= c_caps[bit];
            }
        }
        assert_eq!(rust_add.bits(), raw_add);
        assert_eq!(rust_caps.bits(), raw_caps);
        assert_eq!(CapabilitySetting::from(rust_caps).to_raw(), raw_caps);
    }
    assert_eq!(
        CapabilitySetting::FromOwner.to_raw(),
        ffi::GIT_INDEX_CAPABILITY_FROM_OWNER
    );
}

/// Repeatedly apply typed and raw capability values to independent indexes,
/// comparing state after each call. Index methods consuming these enums are a
/// later symbol batch; this test uses the raw boundary only to observe C.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn scalar_capabilities_c_state_sequence() {
    use libgit2::libgit2::index::index::{Capabilities, CapabilitySetting};

    let typed = Index::new().unwrap();
    let reference = Index::new().unwrap();
    for bits in [0, 7, 1, 2, 3, 4, 5, 6, 0] {
        let setting = CapabilitySetting::from(Capabilities::from_bits(bits).unwrap());
        // SAFETY: two distinct live indexes, exclusively owned for these calls.
        let (a, b) = unsafe {
            (
                ffi::git_index_set_caps(typed.as_c_ptr(), setting.to_raw()),
                ffi::git_index_set_caps(reference.as_c_ptr(), bits),
            )
        };
        assert_eq!((a, b), (0, 0));
        // SAFETY: both indexes remain live and unaliased; getters copy integers.
        let (a, b) = unsafe {
            (
                ffi::git_index_caps(typed.as_c_ptr()),
                ffi::git_index_caps(reference.as_c_ptr()),
            )
        };
        assert_eq!((a, b), (bits, bits));
        // Neither index has an owner. The command must fail without setting all
        // bits or changing the previous explicit state.
        // SAFETY: the same two live indexes; the sentinel is a valid C command.
        let (a, b) = unsafe {
            (
                ffi::git_index_set_caps(typed.as_c_ptr(), CapabilitySetting::FromOwner.to_raw()),
                ffi::git_index_set_caps(reference.as_c_ptr(), ffi::GIT_INDEX_CAPABILITY_FROM_OWNER),
            )
        };
        assert_eq!((a, b), (-1, -1));
        // SAFETY: both indexes are still live; failed setters do not free them.
        let (a, b) = unsafe {
            (
                ffi::git_index_caps(typed.as_c_ptr()),
                ffi::git_index_caps(reference.as_c_ptr()),
            )
        };
        assert_eq!((a, b), (bits, bits));
    }
}

/// The sentinel also reaches its successful owner-configuration branch. No
/// index wrapper method is added by this type batch; C observes the typed value.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn scalar_capabilities_from_owner() {
    use libgit2::libgit2::index::index::{Capabilities, CapabilitySetting};

    let typed_repo = Repo::new("scalar-caps-typed");
    let raw_repo = Repo::new("scalar-caps-raw");
    // Convert to shared owners before the repositories retain their references.
    let typed: IndexArc = Index::new().unwrap().into();
    let raw: IndexArc = Index::new().unwrap().into();
    typed_repo.set_index(typed.as_c_ptr());
    raw_repo.set_index(raw.as_c_ptr());
    for initial in [Capabilities::empty(), Capabilities::all()] {
        // SAFETY: the live indexes and retaining repositories remain on this
        // thread, without simultaneous access or borrowed index contents.
        unsafe {
            assert_eq!(ffi::git_index_set_caps(typed.as_c_ptr(), initial.bits()), 0);
            assert_eq!(ffi::git_index_set_caps(raw.as_c_ptr(), initial.bits()), 0);
            assert_eq!(
                ffi::git_index_set_caps(typed.as_c_ptr(), CapabilitySetting::FromOwner.to_raw()),
                0
            );
            assert_eq!(
                ffi::git_index_set_caps(raw.as_c_ptr(), ffi::GIT_INDEX_CAPABILITY_FROM_OWNER),
                0
            );
            let actual = ffi::git_index_caps(typed.as_c_ptr());
            assert_eq!(actual, ffi::git_index_caps(raw.as_c_ptr()));
            assert!(Capabilities::from_bits(actual).is_some());
        }
    }
}

mod matched_path_callback {
    use super::*;
    use libgit2::libgit2::index::index::{MatchedPathCallback, MatchedPathResult};
    use std::ffi::{c_char, c_void};

    #[derive(Default, Debug, PartialEq)]
    struct Recorder {
        codes: Vec<c_int>,
        seen: Vec<(CString, Option<CString>)>,
    }

    impl Recorder {
        fn call(&mut self, path: &CStr, pattern: Option<&CStr>) -> c_int {
            let code = self.codes.get(self.seen.len()).copied().unwrap_or(0);
            self.seen
                .push((path.to_owned(), pattern.map(CStr::to_owned)));
            code
        }
    }

    unsafe extern "C" fn reference(
        path: *const c_char,
        pattern: *const c_char,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: remove_all supplies immutable live NUL strings (pattern nullable),
        // and the harness supplies an exclusively borrowed Recorder for the scan.
        unsafe {
            let path = CStr::from_ptr(path);
            let pattern = (!pattern.is_null()).then(|| CStr::from_ptr(pattern));
            (&mut *payload.cast::<Recorder>()).call(path, pattern)
        }
    }

    /// The integration harness lends C's strings to the public callable API.
    /// Production add/remove/update wrappers will use the crate-private scan seam.
    unsafe extern "C" fn public_api(
        path: *const c_char,
        pattern: *const c_char,
        payload: *mut c_void,
    ) -> c_int {
        // SAFETY: same C string contract as reference; the harness instead supplies
        // a live exclusive MatchedPathCallback, never retained or reentered.
        unsafe {
            let path = CStr::from_ptr(path);
            let pattern = (!pattern.is_null()).then(|| CStr::from_ptr(pattern));
            (&mut *payload.cast::<MatchedPathCallback<'_>>())
                .call(path, pattern)
                .code()
        }
    }

    /// Safe construction; raw additions stand in for the not-yet-wrapped index_add.
    fn seeded() -> IndexBox {
        let index = Index::new().unwrap();
        for path in [c"a", c"b", c"c", c"dir/non-utf8-\xff"] {
            let mut entry = ffi::git_index_entry {
                mode: 0o100644,
                path: path.as_ptr(),
                ..Default::default()
            };
            entry.id.type_ = ffi::GIT_OID_SHA1 as _;
            entry.id.id[0] = 1;
            // SAFETY: the fresh index is exclusively held and C copies this entry
            // and its immutable NUL path before returning.
            assert_eq!(unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }, 0);
        }
        index
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_scan_matches_public_callback_results_arguments_and_mutations() {
        let raw = seeded();
        let safe = seeded();
        for (patterns, codes) in [
            (vec![c"missing"], vec![i32::MIN]),
            // No usable pattern: every entry is seen, without a pattern.
            (vec![c"", c""], vec![1, i32::MAX, 1, 1]),
            (vec![c"#a", c"\nb", c"\r\nb"], vec![1, 1, 1, 1]),
            (vec![c"#a", c"c"], vec![1]),
            (vec![c"*"], vec![0, i32::MAX, -987]),
            (vec![c"b", c"dir/*"], vec![1, 0]),
            (vec![], vec![i32::MIN]),
            (vec![], vec![0, 0]),
        ] {
            let mut patterns: Vec<_> = patterns
                .into_iter()
                .map(|p| p.as_ptr().cast_mut())
                .collect();
            let paths = ffi::git_strarray {
                strings: patterns.as_mut_ptr(),
                count: patterns.len(),
            };
            let mut expected = Recorder {
                codes: codes.clone(),
                ..Default::default()
            };
            clear();
            // SAFETY: live exclusively used index, borrowed pathspec and recorder;
            // reference has the matching payload type and retains nothing.
            let c_rc = unsafe {
                ffi::git_index_remove_all(
                    raw.as_c_ptr(),
                    &paths,
                    Some(reference),
                    ptr::from_mut(&mut expected).cast(),
                )
            };
            let c_error = (c_rc < 0).then(c_last);
            let mut actual = Recorder {
                codes,
                ..Default::default()
            };
            let mut f = |path: &CStr, pattern: Option<&CStr>| {
                MatchedPathResult::from_code(actual.call(path, pattern))
            };
            let mut callback = MatchedPathCallback::new(&mut f);
            clear();
            // SAFETY: as above, public_api's payload is a live exclusive callback;
            // its closure merely records the arguments and returns a typed code.
            let rust_rc = unsafe {
                ffi::git_index_remove_all(
                    safe.as_c_ptr(),
                    &paths,
                    Some(public_api),
                    ptr::from_mut(&mut callback).cast(),
                )
            };
            let rust_error = (rust_rc < 0).then(c_last);
            assert_eq!(c_rc, rust_rc);
            assert_eq!(c_error, rust_error);
            assert_eq!(state(raw.as_c_ptr()), state(safe.as_c_ptr()));
            drop(callback);
            assert_eq!(expected, actual);
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn stage_values_match_c() {
    use libgit2::libgit2::index::index::Stage;
    for stage in [
        Stage::Any,
        Stage::Normal,
        Stage::Ancestor,
        Stage::Ours,
        Stage::Theirs,
    ] {
        assert_eq!(Stage::from_raw(stage.raw()), Some(stage));
        if stage == Stage::Any {
            assert_eq!(stage.raw(), ffi::GIT_INDEX_STAGE_ANY);
            continue;
        }
        let raw = ffi::git_index_entry {
            flags: ((stage.raw() as u16) << 12) | 0xcfff,
            ..Default::default()
        };
        // SAFETY: initialized local entry; entry_stage reads only its flags.
        let observed = unsafe { ffi::git_index_entry_stage(&raw) };
        assert_eq!(observed, stage.raw());
    }
}

/// Creation is the safe operation under test. `next` is an observation of the
/// resulting C state, not another wrapper (the entry type is a later workset).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn iterator_snapshot_lifecycle_matches_c() {
    use libgit2::libgit2::index::index::IndexIterator;

    struct CIterator(*mut ffi::git_index_iterator);
    impl Drop for CIterator {
        fn drop(&mut self) {
            // SAFETY: one successful raw constructor result, released once.
            unsafe { ffi::git_index_iterator_free(self.0) }
        }
    }
    fn next(iterator: *mut ffi::git_index_iterator) -> (i32, Option<(CString, u16)>) {
        let mut entry = ptr::null();
        // SAFETY: each caller supplies a live, exclusively advanced iterator;
        // its retained index keeps the entry alive until copied below.
        let rc = unsafe { ffi::git_index_iterator_next(&mut entry, iterator) };
        let value = if rc == 0 {
            // SAFETY: a successful next writes a live entry with a NUL path;
            // neither is mutated or released before this copy.
            Some(unsafe { (CStr::from_ptr((*entry).path).to_owned(), (*entry).flags) })
        } else {
            assert!(entry.is_null());
            None
        };
        (rc, value)
    }

    init();
    for populated in [false, true] {
        for parent_first in [false, true] {
            let c_index = c_open(None).1.unwrap();
            let rust_index: IndexArc = Index::new().unwrap().into();
            if populated {
                for path in [c"z", c"a", c"dir/b"] {
                    assert_eq!(add(c_index.0, path), add(rust_index.as_c_ptr(), path));
                }
            }
            let mut raw = ptr::null_mut();
            // SAFETY: writable output slot and live index; C retains a counted
            // reference on success, settled by CIterator below.
            let rc = unsafe { ffi::git_index_iterator_new(&mut raw, c_index.0) };
            assert_eq!(rc, 0);
            let c_iter = CIterator(raw);
            let rust_iter = IndexIterator::new(&rust_index).unwrap();
            assert_eq!(state(c_index.0), state(rust_index.as_c_ptr()));
            // Adding after creation must not enter either snapshot. Replacing an
            // existing entry exercises C's deferred deletion/readers mechanism.
            for path in [c"new", c"a"] {
                assert_eq!(add(c_index.0, path), add(rust_index.as_c_ptr(), path));
            }
            let parents = (c_index, rust_index);
            let parents = if parent_first {
                drop(parents);
                None
            } else {
                Some(parents)
            };
            let expected: &[&CStr] = if populated {
                &[c"a", c"dir/b", c"z"]
            } else {
                &[]
            };
            for name in expected {
                let c_seen = next(c_iter.0);
                assert_eq!(c_seen, next(rust_iter.as_c_ptr()));
                assert_eq!(c_seen.0, 0);
                assert_eq!(c_seen.1.unwrap().0.as_c_str(), *name);
            }
            for _ in 0..3 {
                let c_seen = next(c_iter.0);
                assert_eq!(c_seen, next(rust_iter.as_c_ptr()));
                assert_eq!(c_seen, (ffi::GIT_ITEROVER, None));
            }
            drop(c_iter);
            drop(rust_iter);
            if let Some((c_index, rust_index)) = parents {
                assert_eq!(state(c_index.0), state(rust_index.as_c_ptr()));
            }
        }
    }
}

#[test]
fn entry_flags_match_c_enum_and_preserve_other_fields() {
    use libgit2::libgit2::index::index::EntryFlags;
    assert_eq!(EntryFlags::EXTENDED.bits(), ffi::GIT_INDEX_ENTRY_EXTENDED);
    assert_eq!(EntryFlags::VALID.bits(), ffi::GIT_INDEX_ENTRY_VALID);
    for bits in 0..=u16::MAX {
        let bits = u32::from(bits);
        let mut flags = EntryFlags::from_bits_retain(bits);
        assert_eq!(flags.bits(), bits);
        flags.insert(EntryFlags::EXTENDED);
        assert_eq!(flags.bits(), bits | ffi::GIT_INDEX_ENTRY_EXTENDED);
        flags.remove(EntryFlags::VALID);
        assert_eq!(
            flags.bits(),
            (bits | ffi::GIT_INDEX_ENTRY_EXTENDED) & !ffi::GIT_INDEX_ENTRY_VALID
        );
    }
}

mod conflict_cursor {
    use super::*;
    use libgit2::libgit2::index::index::{ConflictIterator, EntryExtendedFlags as F};

    /// No entry constructor is available yet; C copies these stack entries and
    /// static paths into the safely constructed index during each add call.
    fn populate(index: &mut IndexBox) {
        let mut resolved = ffi::git_index_entry {
            mode: 0o100644,
            path: c"aa-resolved".as_ptr(),
            ..Default::default()
        };
        resolved.id.type_ = ffi::GIT_OID_SHA1 as _;
        // SAFETY: the sole index is live and C copies this initialized entry and
        // its static NUL-terminated path. Stage zero must be skipped by the cursor.
        let rc = unsafe { ffi::git_index_add(index.as_c_ptr(), &resolved) };
        assert_eq!(rc, 0);
        for (name, mask) in [(c"a", 7), (c"b", 1), (c"c", 2), (c"d", 4), (c"e", 5)] {
            let entry = ffi::git_index_entry {
                mode: 0o100644,
                path: name.as_ptr(),
                ..Default::default()
            };
            let side = |bit| {
                if mask & bit != 0 {
                    &raw const entry
                } else {
                    ptr::null()
                }
            };
            let (index, ancestor, ours, theirs) = (index.as_c_ptr(), side(1), side(2), side(4));
            // SAFETY: sole live index, valid entries with static NUL paths read
            // and copied only during this call. NULL represents an absent side.
            let rc = unsafe { ffi::git_index_conflict_add(index, ancestor, ours, theirs) };
            assert_eq!(rc, 0);
        }
    }

    type Observation = (i32, [Option<(Vec<u8>, u16)>; 3]);

    /// Observes constructor state via C (next is owned by the later symbol wave).
    ///
    /// # Safety
    /// `cursor` must be a live cursor over a live, unmodified, exclusively held index.
    unsafe fn observe(cursor: *mut ffi::git_index_conflict_iterator) -> Observation {
        let mut a = ptr::null();
        let mut o = ptr::null();
        let mut t = ptr::null();
        // SAFETY: caller guarantees cursor and index liveness/exclusivity;
        // independent writable slots receive borrowed optional entry pointers.
        let rc = unsafe { ffi::git_index_conflict_next(&mut a, &mut o, &mut t, cursor) };
        let entries = [a, o, t].map(|p| {
            if p.is_null() {
                None
            } else {
                // SAFETY: C returned a live entry from the unchanged index; its
                // NUL path and flags are copied before any mutation or teardown.
                unsafe {
                    Some((
                        CStr::from_ptr(ptr::addr_of!((*p).path).read())
                            .to_bytes()
                            .to_vec(),
                        ptr::addr_of!((*p).flags).read(),
                    ))
                }
            }
        });
        (rc, entries)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_preserve_conflict_state_and_release_only_cursor() {
        let mut raw_parent = Index::new().unwrap();
        let mut safe_parent = Index::new().unwrap();
        for populated in [false, true] {
            if populated {
                populate(&mut raw_parent);
                populate(&mut safe_parent);
            }
            for _ in 0..2 {
                let safe = ConflictIterator::new(safe_parent.as_mut()).unwrap();
                let mut raw = ptr::null_mut();
                // SAFETY: writable slot and live, exclusive independently owned
                // index which remains unchanged until raw cursor destruction.
                let rc = unsafe {
                    ffi::git_index_conflict_iterator_new(&mut raw, raw_parent.as_c_ptr())
                };
                assert_eq!(rc, 0);
                for step in 0..if populated { 7 } else { 2 } {
                    // SAFETY: both cursors and their independent parents stay live
                    // and untouched; observe only advances each cursor's position.
                    let (c, r) = unsafe { (observe(raw), observe(safe.as_c_ptr())) };
                    assert_eq!(c, r);
                    assert_eq!(
                        r.0,
                        if populated && step < 5 {
                            0
                        } else {
                            ffi::GIT_ITEROVER
                        }
                    );
                }
                // SAFETY: raw is the sole allocation returned above; no pointer
                // into its storage is retained. Parents are still live.
                unsafe { ffi::git_index_conflict_iterator_free(raw) };
                drop(safe);
                // SAFETY: owners remain live after the cursor-only destructors.
                unsafe {
                    assert_eq!(
                        ffi::git_index_entrycount(raw_parent.as_c_ptr()),
                        ffi::git_index_entrycount(safe_parent.as_c_ptr())
                    );
                }
            }
        }
    }

    #[test]
    fn extended_flags_match_c_bitmasks() {
        for (rust, c) in [
            (F::INTENT_TO_ADD, ffi::GIT_INDEX_ENTRY_INTENT_TO_ADD),
            (F::SKIP_WORKTREE, ffi::GIT_INDEX_ENTRY_SKIP_WORKTREE),
            (F::ON_DISK, ffi::GIT_INDEX_ENTRY_EXTENDED_FLAGS),
            (F::UP_TO_DATE, ffi::GIT_INDEX_ENTRY_UPTODATE),
        ] {
            assert_eq!(rust.bits(), c);
        }
        let mut rust = F::EMPTY;
        let mut c = 0;
        for flag in [
            F::UP_TO_DATE,
            F::INTENT_TO_ADD,
            F::SKIP_WORKTREE,
            F::from_bits_retain(1 << 31),
        ] {
            rust.insert(flag);
            c |= flag.bits();
            assert_eq!(rust.bits(), c);
            assert_eq!(
                (rust & F::ON_DISK).bits(),
                c & ffi::GIT_INDEX_ENTRY_EXTENDED_FLAGS
            );
        }
        rust.remove(F::UP_TO_DATE);
        c &= !ffi::GIT_INDEX_ENTRY_UPTODATE;
        assert_eq!(rust.bits(), c);
    }
}

/// `git_index_add` copies entry timestamps verbatim, without normalising them: an entry
/// whose times were set through the safe [`IndexTime`] API and one built from raw
/// integers read back the same after every step.
///
/// [`IndexTime`]: libgit2::libgit2::index::index::IndexTime
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn timestamp_entry_roundtrips() {
    use libgit2::libgit2::index::index::{IndexTime, IndexTimeRef};
    let raw = |time: IndexTimeRef<'_>| ffi::git_index_time {
        seconds: time.seconds(),
        nanoseconds: time.nanoseconds(),
    };
    let left = Index::new().unwrap();
    let right = Index::new().unwrap();
    let mut safe = IndexTime::new(0, 0);
    let cases = [
        (0, 0),
        (i32::MIN, u32::MAX),
        (-1, 1_000_000_000),
        (i32::MAX, 999_999_999),
    ];
    for (step, (seconds, nanoseconds)) in cases.into_iter().enumerate() {
        safe.as_mut().set_seconds(seconds);
        safe.as_mut().set_nanoseconds(nanoseconds);
        let copied = safe.as_ref().to_time();
        let raw_time = ffi::git_index_time {
            seconds,
            nanoseconds,
        };
        let mut c_entry = ffi::git_index_entry {
            ctime: raw_time,
            mtime: raw_time,
            mode: 0o100644,
            path: c"timestamp".as_ptr(),
            ..Default::default()
        };
        c_entry.id.type_ = ffi::GIT_OID_SHA1 as u8;
        let mut rust_entry = c_entry;
        rust_entry.ctime = raw(copied.as_ref());
        rust_entry.mtime = raw(safe.as_ref());
        // SAFETY: two live independent indexes; entries and NUL paths are readable
        // for the calls, which copy them. The entry pointers stay valid until the
        // next mutation of their index, after the reads below.
        unsafe {
            assert_eq!(ffi::git_index_add(left.as_c_ptr(), &c_entry), 0);
            assert_eq!(ffi::git_index_add(right.as_c_ptr(), &rust_entry), 0);
            assert_eq!(ffi::git_index_entrycount(left.as_c_ptr()), 1);
            let a = ffi::git_index_get_byindex(left.as_c_ptr(), 0);
            let b = ffi::git_index_get_byindex(right.as_c_ptr(), 0);
            for (x, y) in [((*a).ctime, (*b).ctime), ((*a).mtime, (*b).mtime)] {
                assert_eq!(
                    (x.seconds, x.nanoseconds),
                    (y.seconds, y.nanoseconds),
                    "step {step}"
                );
            }
            assert_eq!(
                ((*a).mtime.seconds, (*a).mtime.nanoseconds),
                (seconds, nanoseconds)
            );
        }
    }
}
