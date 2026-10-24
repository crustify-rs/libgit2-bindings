//! Equivalence tests for the `index/index.rs` routines that read, write and remove
//! entries: `IndexRef::{path, version, write, write_tree, write_tree_to}` and
//! `IndexMut::{read, set_version, set_caps, remove, remove_bypath, remove_directory,
//! remove_all, update_all}` against their `git_index_*` routines, on independently owned
//! indexes (and index files, and repositories), compared after every step of a
//! sequence: status, error, entries, resolve-undo count, version, capabilities, and the
//! bytes each side wrote.

use std::panic::{AssertUnwindSafe, catch_unwind};

use libgit2::libgit2::src::libgit2::blob::create_from_buffer;
use libgit2::libgit2::src::libgit2::index::{
    Capabilities, CapabilitySetting, IndexRef, MatchedPathCallback, MatchedPathResult, Stage,
};
use libgit2::libgit2::src::libgit2::oid::Oid;
use libgit2::libgit2::src::libgit2::repository::RepositoryError;
use libgit2::libgit2::src::libgit2::strarray::StrArrayView;

use super::*;

/// A C status and the thread's error when it failed.
type Status = (c_int, Option<(c_int, CString)>);

fn c_status(rc: c_int) -> Status {
    (rc, (rc != 0).then(c_last))
}

fn rust_status<T>(result: &Result<T, RepositoryError>) -> Status {
    match result {
        Ok(_) => (0, None),
        Err(e) => (e.raw_code(), Some((e.raw_class(), e.message().to_owned()))),
    }
}

/// Everything an index's caller can observe of it through the public C API.
#[derive(Debug, PartialEq)]
struct Observed {
    entries: Vec<(CString, u16, u32, u8, [u8; 32], u32)>,
    reuc: usize,
    version: u32,
    caps: c_int,
    path: Option<CString>,
}

fn observe(index: *mut ffi::git_index) -> Observed {
    {
        // SAFETY: a live index, only read; every string is copied before the next call.
        let path = unsafe { ffi::git_index_path(index) };
        // SAFETY: as above.
        let count = unsafe { ffi::git_index_entrycount(index) };
        let entries = (0..count)
            .map(|i| {
                // SAFETY: as above; `i` is below the entry count.
                let e = unsafe { ffi::git_index_get_byindex(index, i) };
                // SAFETY: a live entry, copied before the next call.
                let e = unsafe { &*e };
                // SAFETY: its path is NUL-terminated; it is copied.
                let path = unsafe { CStr::from_ptr(e.path) }.to_owned();
                (path, e.flags, e.mode, e.id.type_, e.id.id, e.file_size)
            })
            .collect();
        // SAFETY: as above.
        let reuc = unsafe { ffi::git_index_reuc_entrycount(index) };
        // SAFETY: as above.
        let version = unsafe { ffi::git_index_version(index) };
        // SAFETY: as above.
        let caps = unsafe { ffi::git_index_caps(index) };
        Observed {
            entries,
            reuc,
            version,
            caps,
            // SAFETY: as above, a NUL-terminated path; it is copied.
            path: (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) }.to_owned()),
        }
    }
}

/// Adds a regular-file entry (or `mode`) at `path` and `stage` with id byte `byte`.
fn add_entry(index: *mut ffi::git_index, path: &CStr, stage: u16, id: &[u8], mode: u32) {
    let mut entry = ffi::git_index_entry {
        mode,
        path: path.as_ptr(),
        flags: stage << 12,
        ..Default::default()
    };
    entry.id.type_ = u8::try_from(ffi::GIT_OID_SHA1).unwrap();
    entry.id.id[..id.len()].copy_from_slice(id);
    // SAFETY: a live index with no entry borrowed; C copies the entry and its path.
    assert_eq!(unsafe { ffi::git_index_add(index, &raw const entry) }, 0);
}

/// The same entries, added to both indexes.
fn seed(indexes: [*mut ffi::git_index; 2], entries: &[(&CStr, u16)]) {
    for index in indexes {
        for (n, &(path, stage)) in entries.iter().enumerate() {
            add_entry(
                index,
                path,
                stage,
                &[u8::try_from(n).unwrap() + 1],
                0o100_644,
            );
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn path_and_version_sequences_match() {
    init();
    let scratch = Scratch::new("edits-version");
    let (_, c) = c_open(Some(&c_path(&scratch.file("c"))));
    let c = c.unwrap();
    let mut rust = Index::open(&c_path(&scratch.file("r"))).unwrap();
    let file = |p: Option<CString>| p.map(|p| p.to_bytes().last().copied());
    assert_eq!(file(observe(c.0).path), Some(Some(b'c')));
    assert_eq!(
        rust.as_ref().path(),
        Some(c_path(&scratch.file("r")).as_c_str())
    );
    assert_eq!(Index::new().unwrap().as_ref().path(), None);
    for version in [2, 3, 4, 1, 5, 0, u32::MAX, 3, 2] {
        clear();
        // SAFETY: a live index.
        let expected = c_status(unsafe { ffi::git_index_set_version(c.0, version) });
        clear();
        let actual = rust_status(&rust.as_mut().set_version(version));
        assert_eq!(expected, actual, "{version}");
        // SAFETY: a live index, only read.
        let c_version = unsafe { ffi::git_index_version(c.0) };
        assert_eq!(c_version, rust.as_ref().version());
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn capability_sequences_match_and_reorder_alike() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    let c_ptr = c.0;
    seed(
        [c_ptr, rust.as_c_ptr()],
        &[(c"B", 0), (c"a", 0), (c"A", 0), (c"b", 0)],
    );
    for bits in [1, 0, 7, 2, 4, 3, 0] {
        let setting = CapabilitySetting::from(Capabilities::from_bits(bits).unwrap());
        clear();
        // SAFETY: a live index.
        let expected = c_status(unsafe { ffi::git_index_set_caps(c_ptr, bits) });
        clear();
        let actual = rust_status(&rust.as_mut().set_caps(setting));
        assert_eq!(expected, actual);
        assert_eq!(observe(c_ptr), observe(rust.as_c_ptr()));
        clear();
        let from_owner = ffi::GIT_INDEX_CAPABILITY_FROM_OWNER;
        // SAFETY: a live index; the command fails for an index with no owner.
        let expected = c_status(unsafe { ffi::git_index_set_caps(c_ptr, from_owner) });
        clear();
        let actual = rust_status(&rust.as_mut().set_caps(CapabilitySetting::FromOwner));
        assert_eq!(expected, actual);
        assert_eq!(observe(c_ptr), observe(rust.as_c_ptr()));
    }
}

/// Removal by path and stage, by path, and by directory, from equal indexes holding
/// stage-0 entries and conflicts, after each of which the indexes, then the files they
/// write, are compared.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::items_after_statements,
    reason = "each helper item sits beside the steps that use it"
)]
fn removal_sequences_match() {
    init();
    let scratch = Scratch::new("edits-remove");
    let c = c_open(Some(&c_path(&scratch.file("c")))).1.unwrap();
    let mut rust = Index::open(&c_path(&scratch.file("r"))).unwrap();
    seed(
        [c.0, rust.as_c_ptr()],
        &[
            (c"a", 0),
            (c"conflict", 1),
            (c"conflict", 2),
            (c"conflict", 3),
            (c"dir/one", 0),
            (c"dir/two", 0),
            (c"dir/three", 2),
            (c"dir/sub/four", 0),
            (c"dirt", 0),
            (c"z", 0),
            (c"z", 3),
            (c"other-\xff", 0),
        ],
    );
    enum Step {
        Remove(&'static CStr, Stage),
        ByPath(&'static CStr),
        Directory(&'static CStr, Stage),
    }
    use Step::*;
    for step in [
        Remove(c"a", Stage::Ours),
        Remove(c"a", Stage::Normal),
        Remove(c"a", Stage::Normal),
        Remove(c"z", Stage::Any),
        Remove(c"z", Stage::Any),
        Remove(c"z", Stage::Theirs),
        Remove(c"", Stage::Any),
        ByPath(c"missing"),
        ByPath(c"conflict"),
        ByPath(c"conflict"),
        Directory(c"dir", Stage::Ours),
        Directory(c"dir/", Stage::Normal),
        Directory(c"dir", Stage::Normal),
        Directory(c"nowhere", Stage::Normal),
        ByPath(c"other-\xff"),
        Directory(c"", Stage::Normal),
    ] {
        clear();
        // SAFETY: a live index; each path is NUL-terminated, read for the call.
        let expected = c_status(match step {
            Remove(path, Stage::Any) => {
                // A deliberate divergence: C removes whichever stage its search meets,
                // and drops the path's stage 3 entry from its lookup table; Rust removes
                // the first entry at the path, which `git_index_find` finds.
                // SAFETY: as above.
                unsafe { ffi::git_index_remove(c.0, path.as_ptr(), c_first_stage(c.0, path)) }
            }
            // SAFETY: as above.
            Remove(path, stage) => unsafe {
                ffi::git_index_remove(c.0, path.as_ptr(), stage.raw())
            },
            // SAFETY: as above.
            ByPath(path) => unsafe { ffi::git_index_remove_bypath(c.0, path.as_ptr()) },
            Directory(dir, stage) => {
                // SAFETY: as above.
                unsafe { ffi::git_index_remove_directory(c.0, dir.as_ptr(), stage.raw()) }
            }
        });
        clear();
        let mut handle = rust.as_mut();
        let actual = rust_status(&match step {
            Remove(path, stage) => handle.remove(path, stage),
            ByPath(path) => handle.remove_bypath(path),
            Directory(dir, stage) => handle.remove_directory(dir, stage),
        });
        assert_eq!(expected, actual);
        let (c_seen, r_seen) = (observe(c.0), observe(rust.as_c_ptr()));
        assert_eq!(c_seen.entries, r_seen.entries);
        assert_eq!(c_seen.reuc, r_seen.reuc);
    }
    assert_eq!(write(c.0), (0, None));
    assert_eq!(rust_status(&rust.as_ref().write()), (0, None));
    assert_eq!(
        std::fs::read(scratch.file("c")).unwrap(),
        std::fs::read(scratch.file("r")).unwrap()
    );
}

/// The stage of the first entry at `path`, as `git_index_find` finds it, or C's
/// wildcard when there is none.
fn c_first_stage(index: *mut ffi::git_index, path: &CStr) -> c_int {
    let mut position = 0;
    // SAFETY: a writable slot, a live index and a NUL-terminated path; a found position
    // is within the count, so the entry it names is live, and only read.
    unsafe {
        if ffi::git_index_find(&raw mut position, index, path.as_ptr()) < 0 {
            return Stage::Any.raw();
        }
    }
    // SAFETY: as above.
    let index_get_byindex = unsafe { ffi::git_index_get_byindex(index, position) };
    // SAFETY: as above.
    unsafe { ffi::git_index_entry_stage(index_get_byindex) }
}

/// What one scan's callback saw and returned.
#[derive(Debug, Default, PartialEq)]
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

unsafe extern "C" fn record(
    path: *const std::ffi::c_char,
    pattern: *const std::ffi::c_char,
    payload: *mut std::ffi::c_void,
) -> c_int {
    {
        // SAFETY: the scan lends live NUL-terminated strings (the pattern nullable) for
        // the call, and the test an exclusively borrowed recorder.
        let pattern = unsafe { (!pattern.is_null()).then(|| CStr::from_ptr(pattern)) };
        // SAFETY: as above.
        let recorder = unsafe { &mut *payload.cast::<Recorder>() };
        // SAFETY: as above, a NUL-terminated path.
        recorder.call(unsafe { CStr::from_ptr(path) }, pattern)
    }
}

/// A C scan (`remove_all` or `update_all`) with the recorder as its callback, or none.
fn c_scan(
    scan: unsafe extern "C" fn(
        *mut ffi::git_index,
        *const ffi::git_strarray,
        ffi::git_index_matched_path_cb,
        *mut std::ffi::c_void,
    ) -> c_int,
    index: *mut ffi::git_index,
    patterns: &[&CStr],
    recorder: Option<&mut Recorder>,
) -> Status {
    let mut strings: Vec<_> = patterns.iter().map(|p| p.as_ptr().cast_mut()).collect();
    let array = ffi::git_strarray {
        strings: strings.as_mut_ptr(),
        count: strings.len(),
    };
    clear();
    // SAFETY: a live index and array of live strings, read for the call; the recorder,
    // if any, is exclusively lent to `record` for the call.
    c_status(match recorder {
        // SAFETY: as above.
        Some(r) => unsafe {
            scan(
                index,
                &raw const array,
                Some(record),
                ptr::from_mut(r).cast(),
            )
        },
        // SAFETY: as above.
        None => unsafe { scan(index, &raw const array, None, ptr::null_mut()) },
    })
}

/// `IndexMut::remove_all` against `git_index_remove_all`, with recorders returning the
/// same codes (apply, skip, abort with any negative code), without a callback, and
/// with a Rust callback that panics against a C one that returns `GIT_EUSER`.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn remove_all_scans_match() {
    init();
    let entries: &[(&CStr, u16)] = &[
        (c"a", 0),
        (c"b", 0),
        (c"c", 1),
        (c"c", 3),
        (c"dir/x", 0),
        (c"dir/y", 0),
        (c"\xff", 0),
    ];
    for (patterns, codes) in [
        (vec![], vec![0, 1, 0, 0, 1]),
        (vec![c"*"], vec![1, 1, 1]),
        (vec![c"dir/*", c"a"], vec![0, 0, 0]),
        (vec![c"c"], vec![0]),
        (vec![c"b", c"missing"], vec![-1234]),
        (vec![c"#a", c"\nb"], vec![1, i32::MAX, -7]),
        (vec![c"", c""], vec![i32::MIN]),
    ] {
        let c = c_open(None).1.unwrap();
        let mut rust = Index::new().unwrap();
        seed([c.0, rust.as_c_ptr()], entries);
        let mut expected = Recorder {
            codes: codes.clone(),
            ..Default::default()
        };
        let c_rc = c_scan(
            ffi::git_index_remove_all,
            c.0,
            &patterns,
            Some(&mut expected),
        );
        let mut actual = Recorder {
            codes,
            ..Default::default()
        };
        let mut f = |path: &CStr, pattern: Option<&CStr>| {
            MatchedPathResult::from_code(actual.call(path, pattern))
        };
        let mut callback = MatchedPathCallback::new(&mut f);
        let view = StrArrayView::new(&patterns);
        clear();
        let r_rc = rust_status(&rust.as_mut().remove_all(view.as_ref(), Some(&mut callback)));
        drop(callback);
        assert_eq!(c_rc, r_rc);
        assert_eq!(expected, actual);
        assert_eq!(observe(c.0), observe(rust.as_c_ptr()));

        // Then without a callback, on what is left.
        let c_rc = c_scan(ffi::git_index_remove_all, c.0, &patterns, None);
        clear();
        let r_rc = rust_status(&rust.as_mut().remove_all(view.as_ref(), None));
        assert_eq!(c_rc, r_rc);
        assert_eq!(observe(c.0), observe(rust.as_c_ptr()));
    }

    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    seed([c.0, rust.as_c_ptr()], entries);
    let mut expected = Recorder {
        codes: vec![0, ffi::GIT_EUSER],
        ..Default::default()
    };
    let c_rc = c_scan(ffi::git_index_remove_all, c.0, &[], Some(&mut expected));
    let mut calls = 0;
    let mut f = |_: &CStr, _: Option<&CStr>| {
        calls += 1;
        assert!(calls != 2, "second path");
        MatchedPathResult::APPLY
    };
    let mut callback = MatchedPathCallback::new(&mut f);
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        drop(
            rust.as_mut()
                .remove_all(StrArrayView::default().as_ref(), Some(&mut callback)),
        );
    }));
    drop(callback);
    assert!(panicked.is_err());
    assert_eq!(c_rc.0, ffi::GIT_EUSER);
    assert_eq!(calls, 2);
    assert_eq!(observe(c.0), observe(rust.as_c_ptr()));
}

/// Neither side's index has an owner: both fail before any callback, alike.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn update_all_without_an_owner_matches() {
    init();
    let c = c_open(None).1.unwrap();
    let mut rust = Index::new().unwrap();
    seed([c.0, rust.as_c_ptr()], &[(c"a", 0)]);
    for patterns in [vec![], vec![c"a"], vec![c"", c""]] {
        let mut expected = Recorder::default();
        let c_rc = c_scan(
            ffi::git_index_update_all,
            c.0,
            &patterns,
            Some(&mut expected),
        );
        let mut actual = Recorder::default();
        let mut f = |path: &CStr, pattern: Option<&CStr>| {
            MatchedPathResult::from_code(actual.call(path, pattern))
        };
        let mut callback = MatchedPathCallback::new(&mut f);
        let view = StrArrayView::new(&patterns);
        clear();
        let r_rc = rust_status(&rust.as_mut().update_all(view.as_ref(), Some(&mut callback)));
        drop(callback);
        assert_eq!(c_rc, r_rc);
        assert_eq!(expected, actual);
        let c_rc = c_scan(ffi::git_index_update_all, c.0, &patterns, None);
        clear();
        let r_rc = rust_status(&rust.as_mut().update_all(view.as_ref(), None));
        assert_eq!(c_rc, r_rc);
        assert_eq!(observe(c.0), observe(rust.as_c_ptr()));
    }
}

/// Soft and forced reads of a missing, an unchanged, a changed, a vanished and a
/// corrupt file, and writes to a locked one, each side on its own file.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn read_and_write_sequences_match() {
    init();
    let scratch = Scratch::new("edits-read");
    let (c_file, r_file) = (scratch.file("c"), scratch.file("r"));
    let c = c_open(Some(&c_path(&c_file))).1.unwrap();
    let mut rust = Index::open(&c_path(&r_file)).unwrap();
    let both = |c: *mut ffi::git_index, rust: &mut IndexBox, step: &str| {
        let (force, op) = step.split_at(1);
        let force = force == "!";
        clear();
        // SAFETY: a live index.
        let expected = c_status(match op {
            // SAFETY: as above.
            "read" => unsafe { ffi::git_index_read(c, c_int::from(force)) },
            // SAFETY: as above.
            _ => unsafe { ffi::git_index_write(c) },
        });
        clear();
        let actual = rust_status(&match op {
            "read" => rust.as_mut().read(force),
            _ => rust.as_ref().write(),
        });
        // Errors name each side's own file.
        let name = |s: Status| {
            let (rc, e) = s;
            let e = e.map(|(class, m)| {
                let m = m
                    .to_string_lossy()
                    .replace(&*r_file.to_string_lossy(), "<file>");
                (class, m.replace(&*c_file.to_string_lossy(), "<file>"))
            });
            (rc, e)
        };
        assert_eq!(name(expected), name(actual), "{step}");
    };
    let check = |c: *mut ffi::git_index, rust: &IndexBox| {
        let (c_seen, mut r_seen) = (observe(c), observe(rust.as_c_ptr()));
        r_seen.path = c_seen.path.clone();
        assert_eq!(c_seen, r_seen);
    };
    for step in ["?read", "!read"] {
        both(c.0, &mut rust, step);
        check(c.0, &rust);
    }
    seed([c.0, rust.as_c_ptr()], &[(c"a", 0), (c"b", 2)]);
    for step in ["?read", "?write", "?read", "!read"] {
        both(c.0, &mut rust, step);
        check(c.0, &rust);
    }
    assert_eq!(
        std::fs::read(&c_file).unwrap(),
        std::fs::read(&r_file).unwrap()
    );
    // Unwritten changes survive a soft read of an unchanged file, not a forced one.
    seed([c.0, rust.as_c_ptr()], &[(c"c", 0)]);
    for step in ["?read", "!read"] {
        both(c.0, &mut rust, step);
        check(c.0, &rust);
    }
    // Another writer changes the file: a soft read picks it up.
    let bytes = index_bytes(&scratch, 3);
    std::fs::write(&c_file, &bytes).unwrap();
    std::fs::write(&r_file, &bytes).unwrap();
    both(c.0, &mut rust, "?read");
    check(c.0, &rust);
    // A held lock.
    std::fs::write(scratch.file("c.lock"), b"").unwrap();
    std::fs::write(scratch.file("r.lock"), b"").unwrap();
    both(c.0, &mut rust, "?write");
    std::fs::remove_file(scratch.file("c.lock")).unwrap();
    std::fs::remove_file(scratch.file("r.lock")).unwrap();
    both(c.0, &mut rust, "?write");
    // The file vanishes: a soft read keeps the entries, a forced one clears them.
    std::fs::remove_file(&c_file).unwrap();
    std::fs::remove_file(&r_file).unwrap();
    for step in ["?read", "!read"] {
        both(c.0, &mut rust, step);
        check(c.0, &rust);
    }
    // A corrupt file fails, and so does a file that is too short for its checksum.
    for bytes in [
        &b"DIRC\0\0\0\x02\0\0\0\x01not an entry, but long enough to be read"[..],
        b"x",
    ] {
        std::fs::write(&c_file, bytes).unwrap();
        std::fs::write(&r_file, bytes).unwrap();
        for step in ["?read", "!read"] {
            both(c.0, &mut rust, step);
            check(c.0, &rust);
        }
    }
    // An in-memory index can neither read nor write.
    let memory = c_open(None).1.unwrap();
    let mut rust_memory = Index::new().unwrap();
    for step in ["?read", "!read", "?write"] {
        both(memory.0, &mut rust_memory, step);
    }
}

/// The type and bytes of an id.
type Id = (u8, [u8; 32]);

fn id_of(id: &Oid) -> Id {
    (id.as_ref().raw_kind(), id.as_ref().id())
}

/// The blob holding `data` in a raw repository.
fn c_blob(repo: *mut ffi::git_repository, data: &[u8]) -> Id {
    let mut id = ffi::git_oid::default();
    // SAFETY: a live repository and buffer; `id` is written on success.
    let rc = unsafe {
        ffi::git_blob_create_from_buffer(&raw mut id, repo, data.as_ptr().cast(), data.len())
    };
    assert_eq!(rc, 0);
    (id.type_, id.id)
}

fn c_tree(rc: c_int, id: &ffi::git_oid) -> (Status, Option<Id>) {
    (c_status(rc), (rc == 0).then_some((id.type_, id.id)))
}

fn rust_tree(result: Result<Oid, RepositoryError>) -> (Status, Option<Id>) {
    (rust_status(&result), result.ok().as_ref().map(id_of))
}

/// `write_tree_to` with each repository's own blobs, then `write_tree` through each
/// repository's own index, after each change: equal ids, tree caches, conflicts and
/// missing objects alike.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
#[expect(
    clippy::used_underscore_binding,
    reason = "the binding is held for its drop, and read here on purpose"
)]
fn tree_write_sequences_match() {
    init();
    let c_repo = Repo::new("edits-tree-c");
    let r_dir = Scratch::new("edits-tree-r");
    let r_repo = Repository::init(&c_path(&r_dir.0), true).unwrap();
    let c = c_open(None).1.unwrap();
    let rust = Index::new().unwrap();
    let mut blobs = Vec::new();
    for data in [&b"one"[..], b"two", b"three"] {
        let id = c_blob(c_repo.repo, data);
        assert_eq!(
            id,
            id_of(&create_from_buffer(r_repo.as_ref(), data).unwrap())
        );
        blobs.push(id);
    }
    let step = |c: *mut ffi::git_index, rust: IndexRef<'_>| {
        let mut id = ffi::git_oid::default();
        clear();
        // SAFETY: a live index and repository; `id` is written on success.
        let rc = unsafe { ffi::git_index_write_tree_to(&raw mut id, c, c_repo.repo) };
        let expected = c_tree(rc, &id);
        clear();
        assert_eq!(expected, rust_tree(rust.write_tree_to(r_repo.as_ref())));
        // The tree cache answers a repeated write.
        clear();
        assert_eq!(expected, rust_tree(rust.write_tree_to(r_repo.as_ref())));
        expected
    };
    let add = |path: &CStr, stage: u16, id: &Id, mode: u32| {
        for index in [c.0, rust.as_c_ptr()] {
            add_entry(index, path, stage, &id.1[..20], mode);
        }
    };
    assert_eq!(step(c.0, rust.as_ref()).0, (0, None));
    add(c"a", 0, &blobs[0], 0o100_644);
    add(c"dir/b", 0, &blobs[1], 0o100_755);
    add(c"dir/sub/c", 0, &blobs[2], 0o100_644);
    add(c"link", 0, &blobs[0], 0o120_000);
    assert_eq!(step(c.0, rust.as_ref()).0, (0, None));
    let both = |f: &dyn Fn(*mut ffi::git_index) -> c_int| {
        assert_eq!(f(c.0), 0);
        assert_eq!(f(rust.as_c_ptr()), 0);
    };
    // A case-insensitive index is written case-sensitively, then restored.
    // SAFETY: live indexes with no entry borrowed.
    both(&|index| unsafe { ffi::git_index_set_caps(index, 1) });
    add(c"B", 0, &blobs[2], 0o100_644);
    assert_eq!(step(c.0, rust.as_ref()).0, (0, None));
    // SAFETY: as above.
    both(&|index| unsafe { ffi::git_index_set_caps(index, 0) });
    // A missing object in a subtree fails the subtree's write.
    let missing: Id = (u8::try_from(ffi::GIT_OID_SHA1).unwrap(), [9; 32]);
    add(c"dir/sub/missing", 0, &missing, 0o100_644);
    assert_ne!(step(c.0, rust.as_ref()).0, (0, None));
    // SAFETY: live indexes with no entry borrowed; the path is read for the call.
    both(&|index| unsafe { ffi::git_index_remove(index, c"dir/sub/missing".as_ptr(), 0) });
    let missing: Id = (u8::try_from(ffi::GIT_OID_SHA1).unwrap(), [9; 32]);
    add(c"missing", 0, &missing, 0o100_644);
    assert_ne!(step(c.0, rust.as_ref()).0, (0, None));
    add(c"missing", 0, &missing, ffi::GIT_FILEMODE_COMMIT); // a submodule: not looked up
    assert_eq!(step(c.0, rust.as_ref()).0, (0, None));
    add(c"conflict", 1, &blobs[0], 0o100_644);
    add(c"conflict", 2, &blobs[1], 0o100_644);
    assert_eq!(step(c.0, rust.as_ref()).0.0, ffi::GIT_EUNMERGED);
    // Neither index has an owner.
    let mut id = ffi::git_oid::default();
    clear();
    // SAFETY: a live index; `id` is written on success.
    let rc = unsafe { ffi::git_index_write_tree(&raw mut id, c.0) };
    let expected = c_tree(rc, &id);
    clear();
    assert_eq!(expected, rust_tree(rust.as_ref().write_tree()));
    assert_eq!(expected.0.0, -1);

    // Each repository's own index, written to its owner.
    let c_index = CIndex(c_repo.index());
    let r_index = r_repo.as_ref().index().unwrap();
    for (path, id) in [(c"x", &blobs[0]), (c"y/z", &blobs[1])] {
        for index in [c_index.0, r_index.as_c_ptr()] {
            add_entry(index, path, 0, &id.1[..20], 0o100_644);
        }
        clear();
        let mut out = ffi::git_oid::default();
        // SAFETY: a live index its repository owns; `out` is written on success.
        let rc = unsafe { ffi::git_index_write_tree(&raw mut out, c_index.0) };
        let expected = c_tree(rc, &out);
        clear();
        assert_eq!(expected, rust_tree(r_index.as_ref().write_tree()));
        assert_eq!(expected.0, (0, None));
        // And to its file, under the repository.
        assert_eq!(write(c_index.0), (0, None));
        assert_eq!(rust_status(&r_index.as_ref().write()), (0, None));
        assert_eq!(
            std::fs::read(c_repo._dir.file("index")).unwrap(),
            std::fs::read(r_dir.file("index")).unwrap()
        );
    }
}

/// `write` on an index owned by a repository with a working directory resets the size
/// of racily clean entries that changed on disk; both sides alike. Adding by path loads
/// filters, whose first loads race in libgit2: the body runs in a child process.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn racily_clean_writes_match() {
    isolated_tolerating_leaks(
        concat!(module_path!(), "::racily_clean_writes_match"),
        || {
            init();
            let dirs = [Scratch::new("edits-racy-c"), Scratch::new("edits-racy-r")];
            let repos = dirs
                .iter()
                .map(|d| Repository::init(&c_path(&d.0), false).unwrap())
                .collect::<Vec<_>>();
            let indexes = repos
                .iter()
                .map(|r| r.as_ref().index().unwrap())
                .collect::<Vec<_>>();
            for (dir, index) in dirs.iter().zip(&indexes) {
                for name in ["f", "g"] {
                    std::fs::write(dir.file(name), b"first").unwrap();
                    // SAFETY: a live index its repository owns; the path is read for the call.
                    let rc = unsafe {
                        ffi::git_index_add_bypath(
                            index.as_c_ptr(),
                            c_path(Path::new(name)).as_ptr(),
                        )
                    };
                    assert_eq!(rc, 0);
                }
                assert_eq!(index.as_ref().write(), Ok(()));
                // An index file older than its entries, reread: every entry is racy.
                let old = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
                let file = std::fs::File::options()
                    .write(true)
                    .open(dir.file(".git/index"));
                file.unwrap().set_modified(old).unwrap();
                // SAFETY: a live index its repository owns, with no entry borrowed.
                assert_eq!(unsafe { ffi::git_index_read(index.as_c_ptr(), 1) }, 0);
                // Same size: only the contents tell it changed.
                std::fs::write(dir.file("f"), b"other").unwrap();
            }
            clear();
            let expected = write(indexes[0].as_c_ptr());
            clear();
            assert_eq!(expected, rust_status(&indexes[1].as_ref().write()));
            let seen: Vec<_> = indexes
                .iter()
                .map(|i| {
                    let mut seen = observe(i.as_c_ptr());
                    seen.path = None;
                    seen.entries.iter_mut().for_each(|e| e.1 = 0);
                    seen
                })
                .collect();
            assert_eq!(seen[0], seen[1]);
            let sizes: Vec<_> = seen[0].entries.iter().map(|e| (e.0.clone(), e.5)).collect();
            assert_eq!(sizes, [(c"f".to_owned(), 0), (c"g".to_owned(), 5)]);
        },
    );
}
