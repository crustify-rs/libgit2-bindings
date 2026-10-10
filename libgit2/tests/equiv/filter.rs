//! Equivalence tests for `git2/filter.h`: the `git_filter_list` owner.
//!
//! Each case loads a list twice, independently, one per side, each on its own
//! repository of identical content: C through `git_filter_list_new`,
//! `git_filter_list_load` or `git_filter_list_load_ext`, Rust through
//! [`FilterList::new`], [`FilterList::load`] / [`FilterList::load_ext`] and their
//! `_with_blob` forms. The repositories' `.gitattributes` select libgit2's `crlf` and
//! `ident` filters by extension, and a probe filter (`crustify-probe`, registered once)
//! whose `check` records the source's blob id and keeps a payload its `cleanup` frees;
//! a refusing filter (`crustify-refuse`, after `ident`) fails its `check`, so that
//! libgit2 discards a list it had started (`*.refuse`, after `ident` joined it) or never
//! starts one (`*.refuse-only`), and a filter whose `initialize` fails does the same for
//! every load selecting it (`*.uninit`, in [`failing_initialize`]). `HEAD` is a commit whose own `.gitattributes` selects
//! `ident` for `*.head`, read with `GIT_FILTER_ATTRIBUTES_FROM_HEAD` or, by id,
//! `GIT_FILTER_ATTRIBUTES_FROM_COMMIT`.
//!
//! Compared for each case: success or the error (class and message); whether a list was
//! built; its length and which filters it holds (`git_filter_list_contains`); the ids the
//! probe's `check` saw. Then both lists apply the same sequence of inputs (line endings
//! both ways, `$Id$` keywords), compared after every step, as the filters' state
//! changes in place; then both are released (C: `git_filter_list_free`; Rust: drop),
//! and the probe payloads each release freed are compared (all its load made).
//!
//! Documented divergence, outside these comparisons: libgit2 asserts that the path is
//! relative, which fails the load with `GIT_ERROR_INTERNAL` in a release build and aborts
//! a debug one (`_DEBUG`, as the `coverage` variant builds). The Rust constructors
//! refuse an absolute path before calling libgit2, with the release build's error:
//! [`absolute_paths`] compares that with a release build, and checks the abort is the
//! only difference with a debug one, running the C side in a child process.
//!
//! A libgit2 defect, on both sides alike: when a filter's `initialize` fails,
//! `git_filter_list__load` leaves its loop without freeing the attribute values it read
//! for the filter (`filter.c`, the `break` after `filter_initialize`), so every such load
//! leaks them; [`failing_initialize`] runs with LeakSanitizer off.
//!
//! Every test runs its loads alone in a child process, so that no other module's raw
//! first loads race with them (see `filter.rs`'s module docs).

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

use libgit2::ffi;
use libgit2::libgit2::filter::filter::{FilterList, FilterListBox};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
use libgit2::libgit2::util::errors::ErrorBuf;

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process: libgit2 initialises a filter, unsynchronised,
/// the first time any list selects it, and other modules' tests load lists through raw
/// calls the crate cannot order (see `filter.rs`'s module docs).
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
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

thread_local! {
    /// The blob ids (hex) the probe's `check` saw on this thread, in order.
    static SEEN: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
    /// Probe payloads made and freed on this thread.
    static PAYLOADS: RefCell<(usize, usize)> = const { RefCell::new((0, 0)) };
}

fn hex(id: *const ffi::git_oid) -> String {
    let mut out = [0u8; 41];
    // SAFETY: a live id; `out` holds 41 bytes, NUL included.
    unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
    CStr::from_bytes_until_nul(&out)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

/// `git_filter.check` of the probe: records the source's id and keeps a payload.
unsafe extern "C" fn probe_check(
    _filter: *mut ffi::git_filter,
    payload: *mut *mut c_void,
    src: *const ffi::git_filter_source,
    _values: *mut *const c_char,
) -> c_int {
    // SAFETY: libgit2's live source, read for the call.
    let id = unsafe { ffi::git_filter_source_id(src) };
    SEEN.with_borrow_mut(|seen| seen.push((!id.is_null()).then(|| hex(id))));
    PAYLOADS.with_borrow_mut(|p| p.0 += 1);
    // SAFETY: libgit2's live payload slot; the list hands it back to `probe_cleanup`.
    unsafe { payload.write(Box::into_raw(Box::new(7u32)).cast()) };
    0
}

/// `git_filter.cleanup` of the probe: frees the payload `probe_check` made.
unsafe extern "C" fn probe_cleanup(_filter: *mut ffi::git_filter, payload: *mut c_void) {
    // SAFETY: the payload `probe_check` boxed, handed back once.
    drop(unsafe { Box::from_raw(payload.cast::<u32>()) });
    PAYLOADS.with_borrow_mut(|p| p.1 += 1);
}

/// `git_filter.check` of the refusing filter: fails the load, with an error of its own.
unsafe extern "C" fn refuse_check(
    _filter: *mut ffi::git_filter,
    _payload: *mut *mut c_void,
    _src: *const ffi::git_filter_source,
    _values: *mut *const c_char,
) -> c_int {
    // SAFETY: a NUL-terminated message, which libgit2 copies; the runtime is initialised.
    unsafe { ffi::git_error_set_str(ffi::GIT_ERROR_FILTER as c_int, c"refused by check".as_ptr()) };
    ffi::GIT_EUSER
}

/// `git_filter.initialize` of the uninitialisable filter: fails, so every load selecting
/// it fails, and libgit2 tries again on the next.
unsafe extern "C" fn refuse_initialize(_filter: *mut ffi::git_filter) -> c_int {
    // SAFETY: as in `refuse_check`.
    unsafe {
        ffi::git_error_set_str(
            ffi::GIT_ERROR_FILTER as c_int,
            c"cannot initialise".as_ptr(),
        )
    };
    -1
}

/// Initialises libgit2, registers the probe, and loads a list selecting it once, on one
/// thread: libgit2 marks a registered filter initialised, unsynchronised, the first time
/// a list selects it (the crate does so for the built-in ones only).
fn setup() {
    static DONE: OnceLock<()> = OnceLock::new();
    DONE.get_or_init(|| {
        // SAFETY: balanced by nothing: the reference lives for the process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
        let filter = Box::leak(Box::new(ffi::git_filter::default()));
        // SAFETY: a live filter struct, filled with the version's defaults.
        let rc = unsafe { ffi::git_filter_init(filter, ffi::GIT_FILTER_VERSION) };
        assert_eq!(rc, 0);
        filter.attributes = c"+crustify-probe".as_ptr();
        filter.check = Some(probe_check);
        filter.cleanup = Some(probe_cleanup);
        // SAFETY: a NUL-terminated name and a filter that lives for the process.
        let rc = unsafe { ffi::git_filter_register(c"crustify-probe".as_ptr(), filter, 0) };
        assert_eq!(rc, 0);
        // After `ident` (priority 100): its list has been started when this one refuses.
        let refuse = Box::leak(Box::new(ffi::git_filter::default()));
        // SAFETY: as for the probe.
        let rc = unsafe { ffi::git_filter_init(refuse, ffi::GIT_FILTER_VERSION) };
        assert_eq!(rc, 0);
        refuse.attributes = c"+crustify-refuse".as_ptr();
        refuse.check = Some(refuse_check);
        // SAFETY: as for the probe.
        let rc = unsafe { ffi::git_filter_register(c"crustify-refuse".as_ptr(), refuse, 200) };
        assert_eq!(rc, 0);
        let uninit = Box::leak(Box::new(ffi::git_filter::default()));
        // SAFETY: as for the probe.
        let rc = unsafe { ffi::git_filter_init(uninit, ffi::GIT_FILTER_VERSION) };
        assert_eq!(rc, 0);
        uninit.attributes = c"+crustify-uninit".as_ptr();
        uninit.initialize = Some(refuse_initialize);
        // SAFETY: as for the probe.
        let rc = unsafe { ffi::git_filter_register(c"crustify-uninit".as_ptr(), uninit, 200) };
        assert_eq!(rc, 0);
        let wd = Workdir::new("prime");
        let (list, _) = c_load(
            wd.raw(),
            None,
            c"a.probe",
            ffi::GIT_FILTER_TO_ODB,
            Opts::Flags(0),
        );
        assert!(!list.is_null());
        // SAFETY: the fresh list, released once.
        unsafe { ffi::git_filter_list_free(list) };
        let _ = take_seen();
    });
}

/// The ids the probe's `check` saw, and the payloads it made and freed, since the last
/// call on this thread.
fn take_seen() -> (Vec<Option<String>>, (usize, usize)) {
    let seen = SEEN.with_borrow_mut(std::mem::take);
    (seen, PAYLOADS.with_borrow_mut(std::mem::take))
}

/// A repository with a working directory: `HEAD` commits `.gitattributes` reading
/// `*.head ident`, and the working directory's then selects `crlf` for `*.txt`,
/// `ident` for `*.id`, both for `*.both` and the probe for `*.probe`. Every commit is
/// made with a fixed identity and time, so two of them have the same ids.
struct Workdir {
    path: PathBuf,
    repo: Option<RepositoryBox>,
    head: ffi::git_oid,
}

impl Workdir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-filter-equiv-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let repo = Repository::init(&c_path, false).unwrap();
        let raw = repo.as_c_ptr();
        let attributes = b"*.head ident\n";
        let (mut builder, mut tree, mut who) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
        let (mut blob, mut tree_id) = (ffi::git_oid::default(), ffi::git_oid::default());
        let mut head = ffi::git_oid::default();
        let opts = ffi::git_commit_create_ext_options {
            version: 1,
            update_ref: c"HEAD".as_ptr(),
            ..Default::default()
        };
        // The commit's tree is built directly, so the index stays empty and only
        // `HEAD` (or the commit, by id) has these attributes.
        // SAFETY: writable slots, the live repository, live buffers and NUL-terminated
        // strings; every object is freed once.
        unsafe {
            let rc = ffi::git_blob_create_from_buffer(
                &mut blob,
                raw,
                attributes.as_ptr().cast(),
                attributes.len(),
            );
            assert_eq!(rc, 0);
            assert_eq!(ffi::git_treebuilder_new(&mut builder, raw, ptr::null()), 0);
            let rc = ffi::git_treebuilder_insert(
                ptr::null_mut(),
                builder,
                c".gitattributes".as_ptr(),
                &blob,
                ffi::GIT_FILEMODE_BLOB,
            );
            assert_eq!(rc, 0);
            assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
            ffi::git_treebuilder_free(builder);
            assert_eq!(ffi::git_tree_lookup(&mut tree, raw, &tree_id), 0);
            let rc = ffi::git_signature_new(
                &mut who,
                c"A".as_ptr(),
                c"a@example.com".as_ptr(),
                1_700_000_000,
                0,
            );
            assert_eq!(rc, 0);
            let rc = ffi::git_commit_create_ext(
                &mut head,
                raw,
                who,
                who,
                c"attributes\n".as_ptr(),
                tree,
                0,
                ptr::null(),
                &opts,
            );
            assert_eq!(rc, 0);
            ffi::git_tree_free(tree);
            ffi::git_signature_free(who);
        }
        std::fs::write(
            path.join(".gitattributes"),
            "*.txt text eol=crlf\n*.id ident\n*.both text eol=crlf ident\n*.probe crustify-probe\n\
             *.refuse ident crustify-refuse\n*.refuse-only crustify-refuse\n\
             *.uninit ident crustify-uninit\n",
        )
        .unwrap();
        Self {
            path,
            repo: Some(repo),
            head,
        }
    }

    fn box_(&self) -> &RepositoryBox {
        self.repo.as_ref().unwrap()
    }

    fn raw(&self) -> *mut ffi::git_repository {
        self.box_().as_c_ptr()
    }

    /// A new blob of `content`, looked up: the caller frees it.
    fn blob(&self, content: &[u8]) -> NonNull<ffi::git_blob> {
        let mut id = ffi::git_oid::default();
        let mut blob = ptr::null_mut();
        // SAFETY: writable slots, the live repository and `content`, read for the call.
        unsafe {
            let rc = ffi::git_blob_create_from_buffer(
                &mut id,
                self.raw(),
                content.as_ptr().cast(),
                content.len(),
            );
            assert_eq!(rc, 0);
            assert_eq!(ffi::git_blob_lookup(&mut blob, self.raw(), &id), 0);
        }
        NonNull::new(blob).unwrap()
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        drop(self.repo.take());
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn free_blob(blob: NonNull<ffi::git_blob>) {
    // SAFETY: a blob `Workdir::blob` looked up, freed once.
    unsafe { ffi::git_blob_free(blob.as_ptr()) }
}

/// An error as both sides report it: class and message.
type Reported = (c_int, CString);

fn c_last() -> Reported {
    // SAFETY: the runtime is initialised; the error is copied before any other call.
    unsafe {
        let e = ffi::git_error_last();
        ((*e).klass, CStr::from_ptr((*e).message).to_owned())
    }
}

fn rust_error(e: ErrorBuf) -> Reported {
    (e.raw_class(), e.message().to_owned())
}

/// What a constructor gave, as the public API shows it.
#[derive(Debug, PartialEq)]
enum Built {
    Failed(Reported),
    NoList,
    List { len: usize, holds: [c_int; 3] },
}

/// The list's shape: its length and whether it holds `crlf`, `ident`, the probe.
fn shape(list: *mut ffi::git_filter_list) -> Built {
    // SAFETY: a live list, only read; NUL-terminated names.
    unsafe {
        Built::List {
            len: ffi::git_filter_list_length(list),
            holds: [c"crlf", c"ident", c"crustify-probe"]
                .map(|name| ffi::git_filter_list_contains(list, name.as_ptr())),
        }
    }
}

/// `git_filter_list_apply_to_buffer`: the output, or the error.
fn apply(list: *mut ffi::git_filter_list, data: &[u8]) -> Result<Vec<u8>, Reported> {
    let mut out = ffi::git_buf::default();
    // SAFETY: a zeroed buffer, a NULL or live list, used by this call alone, and `data`.
    let rc = unsafe {
        ffi::git_filter_list_apply_to_buffer(&mut out, list, data.as_ptr().cast(), data.len())
    };
    let result = if rc < 0 {
        Err(c_last())
    } else {
        // SAFETY: on success `out` holds `size` bytes at `ptr`.
        Ok(unsafe { std::slice::from_raw_parts(out.ptr.cast::<u8>(), out.size) }.to_vec())
    };
    // SAFETY: the buffer, released once.
    unsafe { ffi::git_buf_dispose(&mut out) };
    result
}

/// The inputs every list applies in turn.
const INPUTS: [&[u8]; 4] = [b"a\nb\n", b"a\r\nb\r\n", b"$Id$\nx\n", b"$Id: 0123 $\r\ny"];

/// Options as a case gives them: flags for `load`, or a `load_ext` options struct.
#[derive(Clone, Copy)]
enum Opts<'a> {
    Flags(u32),
    Ext(Option<&'a ffi::git_filter_options>),
}

/// The C side: the list `git_filter_list_load` / `_load_ext` built, or NULL, and what
/// was built.
fn c_load(
    repo: *mut ffi::git_repository,
    blob: Option<NonNull<ffi::git_blob>>,
    path: &CStr,
    mode: ffi::git_filter_mode_t,
    opts: Opts<'_>,
) -> (*mut ffi::git_filter_list, Built) {
    let mut list = ptr::null_mut();
    let blob = blob.map_or(ptr::null_mut(), NonNull::as_ptr);
    // SAFETY: a writable slot; the live repository and blob (or NULL); a NUL-terminated
    // path; NULL or live options, only copied.
    let rc = unsafe {
        match opts {
            Opts::Flags(flags) => {
                ffi::git_filter_list_load(&mut list, repo, blob, path.as_ptr(), mode, flags)
            }
            Opts::Ext(o) => {
                let o = o.map_or(ptr::null_mut(), |o| ptr::from_ref(o).cast_mut());
                ffi::git_filter_list_load_ext(&mut list, repo, blob, path.as_ptr(), mode, o)
            }
        }
    };
    let built = match rc {
        rc if rc < 0 => Built::Failed(c_last()),
        _ if list.is_null() => Built::NoList,
        _ => shape(list),
    };
    (list, built)
}

/// The Rust side, as `c_load`.
fn rust_load<'r>(
    wd: &'r Workdir,
    blob: Option<NonNull<ffi::git_blob>>,
    path: &CStr,
    mode: ffi::git_filter_mode_t,
    opts: Opts<'_>,
) -> (Option<FilterListBox<'r>>, Built) {
    let repo = wd.box_().as_ref();
    // SAFETY: the blob, if any, is live for the call.
    let result = unsafe {
        match (opts, blob) {
            (Opts::Flags(flags), None) => FilterList::load(repo, path, mode, flags),
            (Opts::Flags(flags), Some(blob)) => {
                FilterList::load_with_blob(repo, blob, path, mode, flags)
            }
            (Opts::Ext(o), None) => FilterList::load_ext(repo, path, mode, o),
            (Opts::Ext(o), Some(blob)) => FilterList::load_ext_with_blob(repo, blob, path, mode, o),
        }
    };
    match result {
        Err(e) => (None, Built::Failed(rust_error(e))),
        Ok(None) => (None, Built::NoList),
        Ok(Some(list)) => {
            let built = shape(list.as_c_ptr());
            (Some(list), built)
        }
    }
}

/// Runs one case on both sides and compares every step.
fn compare(
    c: &Workdir,
    r: &Workdir,
    with_blob: bool,
    path: &CStr,
    mode: ffi::git_filter_mode_t,
    opts: Opts<'_>,
) {
    let case = format!("{path:?} mode {mode} blob {with_blob}");
    let (c_blob, r_blob) = if with_blob {
        (Some(c.blob(b"blob\n")), Some(r.blob(b"blob\n")))
    } else {
        (None, None)
    };
    let (c_list, c_built) = c_load(c.raw(), c_blob, path, mode, opts);
    let c_seen = take_seen();
    let (mut r_list, r_built) = rust_load(r, r_blob, path, mode, opts);
    let r_seen = take_seen();
    c_blob.into_iter().chain(r_blob).for_each(free_blob);
    assert_eq!(c_built, r_built, "{case}");
    assert_eq!(c_seen, r_seen, "{case}");
    assert_eq!(c_list.is_null(), r_list.is_none(), "{case}");
    // The probe has no stream: a list holding it is never applied.
    let applies = !matches!(
        c_built,
        Built::List {
            holds: [_, _, 1],
            ..
        }
    );
    for (step, input) in INPUTS.iter().enumerate().filter(|_| applies) {
        let r_raw = r_list.as_mut().map_or(ptr::null_mut(), |l| l.as_c_ptr());
        assert_eq!(
            apply(c_list, input),
            apply(r_raw, input),
            "{case} step {step}"
        );
    }
    // SAFETY: the C list, or NULL, released once.
    unsafe { ffi::git_filter_list_free(c_list) };
    let c_after = take_seen();
    drop(r_list);
    let r_after = take_seen();
    assert_eq!(c_after, r_after, "{case}");
    // Each side frees what its load made.
    assert_eq!(c_after.1.1, c_seen.1.0, "{case}");
}

const PATHS: [&CStr; 11] = [
    c"a.txt",
    c"a.id",
    c"a.both",
    c"a.head",
    c"a.probe",
    c"a.bin",
    c"",
    c"sub/b.txt",
    c"../c.id",
    c"a.refuse",
    c"a.refuse-only",
];
const MODES: [ffi::git_filter_mode_t; 3] = [ffi::GIT_FILTER_TO_WORKTREE, ffi::GIT_FILTER_TO_ODB, 5];

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn load() {
    isolated(concat!(module_path!(), "::load"), || {
        setup();
        let (c, r) = (Workdir::new("load-c"), Workdir::new("load-r"));
        for flags in [
            0,
            ffi::GIT_FILTER_ALLOW_UNSAFE,
            ffi::GIT_FILTER_NO_SYSTEM_ATTRIBUTES,
            ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
            ffi::GIT_FILTER_ATTRIBUTES_FROM_COMMIT,
        ] {
            for path in PATHS {
                for mode in MODES {
                    compare(&c, &r, false, path, mode, Opts::Flags(flags));
                }
            }
        }
    });
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn load_ext() {
    isolated(concat!(module_path!(), "::load_ext"), || {
        setup();
        let (c, r) = (Workdir::new("ext-c"), Workdir::new("ext-r"));
        assert_eq!(c.head.id, r.head.id);
        let base = ffi::git_filter_options {
            version: 1,
            ..Default::default()
        };
        let from_head = ffi::git_filter_options {
            flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
            ..base
        };
        let from_commit = ffi::git_filter_options {
            flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_COMMIT | ffi::GIT_FILTER_NO_SYSTEM_ATTRIBUTES,
            attr_commit_id: c.head,
            ..base
        };
        let mut missing = from_commit;
        missing.attr_commit_id.id[..2].copy_from_slice(&[0xba, 0xd0]);
        let unchecked = ffi::git_filter_options {
            version: 99,
            flags: ffi::GIT_FILTER_ALLOW_UNSAFE,
            reserved: NonNull::<u8>::dangling().as_ptr().cast(),
            ..base
        };
        for opts in [
            None,
            Some(&base),
            Some(&from_head),
            Some(&from_commit),
            Some(&missing),
            Some(&unchecked),
        ] {
            for path in PATHS {
                for mode in MODES {
                    compare(&c, &r, false, path, mode, Opts::Ext(opts));
                }
            }
        }
    });
}

/// The blob forms: the probe's `check` sees the blob's id on both sides alike.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn with_blob() {
    isolated(concat!(module_path!(), "::with_blob"), || {
        setup();
        let (c, r) = (Workdir::new("blob-c"), Workdir::new("blob-r"));
        let from_head = ffi::git_filter_options {
            version: 1,
            flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
            ..Default::default()
        };
        for path in PATHS {
            for mode in MODES {
                for opts in [Opts::Flags(0), Opts::Ext(None), Opts::Ext(Some(&from_head))] {
                    compare(&c, &r, true, path, mode, opts);
                }
            }
        }
    });
}

/// A filter whose `initialize` fails fails every load that selects it, with its error,
/// and libgit2 discards the list it had started (`ident` joined it first). libgit2 leaks
/// the attribute values it read for the filter on each (see the module docs), hence
/// LeakSanitizer off in the child.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failing_initialize() {
    let name = "filter::failing_initialize";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        setup();
        let (c, r) = (Workdir::new("uninit-c"), Workdir::new("uninit-r"));
        let (list, built) = c_load(c.raw(), None, c"a.uninit", 0, Opts::Flags(0));
        assert!(list.is_null());
        let expect = (
            ffi::GIT_ERROR_FILTER as c_int,
            c"cannot initialise".to_owned(),
        );
        assert_eq!(built, Built::Failed(expect));
        for _ in 0..2 {
            for mode in MODES {
                compare(&c, &r, false, c"a.uninit", mode, Opts::Flags(0));
                compare(&c, &r, true, c"a.uninit", mode, Opts::Ext(None));
            }
        }
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

/// `git_filter_list_new` against [`FilterList::new`]: an empty list, which passes every
/// input through, for any direction and flags.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new() {
    isolated(concat!(module_path!(), "::new"), || {
        setup();
        let (c, r) = (Workdir::new("new-c"), Workdir::new("new-r"));
        for mode in MODES.into_iter().chain([u32::MAX]) {
            for flags in [0, ffi::GIT_FILTER_ALLOW_UNSAFE, u32::MAX] {
                let mut c_list = ptr::null_mut();
                // SAFETY: a writable slot and the live repository.
                let rc = unsafe { ffi::git_filter_list_new(&mut c_list, c.raw(), mode, flags) };
                assert_eq!(rc, 0);
                let r_list = FilterList::new(r.box_().as_ref(), mode, flags).unwrap();
                assert_eq!(shape(c_list), shape(r_list.as_c_ptr()));
                for input in INPUTS {
                    let c_out = apply(c_list, input);
                    assert_eq!(c_out, apply(r_list.as_c_ptr(), input));
                    assert_eq!(c_out.as_deref(), Ok(input));
                }
                // SAFETY: the C list, released once.
                unsafe { ffi::git_filter_list_free(c_list) };
                drop(r_list);
            }
        }
    });
}

/// An absolute path: the C side runs in a child process, which a debug libgit2 aborts.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn absolute_paths() {
    const PATHS: [&CStr; 3] = [c"/a.txt", c"/", c"//x/a.id"];
    let name = "filter::absolute_paths";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        setup();
        let c = Workdir::new("abs-child");
        for path in PATHS {
            for opts in [Opts::Flags(0), Opts::Ext(None)] {
                let (list, built) = c_load(c.raw(), None, path, ffi::GIT_FILTER_TO_ODB, opts);
                assert!(list.is_null());
                println!("C-BUILT {built:?}");
            }
        }
        return;
    }
    // No list is loaded here: the Rust side refuses before libgit2, and the C side runs
    // in the child.
    let r = Workdir::new("abs-r");
    let mut rust = Vec::new();
    for path in PATHS {
        for opts in [Opts::Flags(0), Opts::Ext(None)] {
            let (list, built) = rust_load(&r, None, path, ffi::GIT_FILTER_TO_ODB, opts);
            assert!(list.is_none());
            rust.push(format!("C-BUILT {built:?}"));
        }
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        // libtest prints the first line after its `test ... ` prefix.
        let c: Vec<_> = stdout
            .lines()
            .filter_map(|l| l.find("C-BUILT ").map(|at| &l[at..]))
            .collect();
        assert_eq!(c, rust, "child:\n{stdout}\n{stderr}");
    } else {
        // A debug build: the assertion aborts the child at its first load.
        assert!(
            stderr.contains("Assertion") && stderr.contains("git_fs_path_is_absolute"),
            "child failed otherwise:\n{stdout}\n{stderr}"
        );
        assert!(!stdout.contains("C-BUILT "));
    }
    // Either way the Rust side failed as a release build does.
    for line in &rust {
        assert!(line.contains("unrecoverable internal error"), "{line}");
    }
}
