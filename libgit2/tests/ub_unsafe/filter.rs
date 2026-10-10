//! UB tests for the unsafe API of `git2/filter.h`: the filter list constructors
//! `FilterList::load_with_blob` and `FilterList::load_ext_with_blob`, whose blob is a raw
//! seam until `git_blob` is wrapped.
//!
//! Each is called within its contract: a blob live for the call. libgit2 reads only the
//! blob's id, which each filter's `check` sees during the load and the list does not
//! keep. A probe filter (registered once as `crustify-probe`, selected only where the
//! `crustify-probe` attribute is set) records that id from its `check`, and keeps a payload of
//! its own in the list, which its `cleanup` frees: the tests free the blob (and, for a
//! blob of another repository, that repository) before the list, so a list that kept
//! the blob would read freed memory on its free (ASan), and a list freed without its
//! filters' cleanup leaks the payload (LSan). The safe forms' edge cases recur with a
//! blob: no filter for the path (no list), a failing load
//! (`GIT_FILTER_ATTRIBUTES_FROM_HEAD` on an unborn `HEAD`), and an absolute path,
//! refused before libgit2 is called; the blob stays the caller's throughout. Each test
//! runs alone in a child process, so that no other module's raw first loads race with
//! the ones here (see `filter.rs`'s module docs).

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

use libgit2::ffi;
use libgit2::libgit2::filter::filter::{FilterList, FilterListBox};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};
use libgit2::libgit2::util::errors::ErrorClass;

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

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
    /// What the probe's `check` saw on this thread: the source's id, hex, if any.
    static SEEN: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
    /// Payloads the probe's `check` made and its `cleanup` freed on this thread.
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
    unsafe { payload.write(Box::into_raw(Box::new(0x5eed_u64)).cast()) };
    0
}

/// `git_filter.cleanup` of the probe: frees the payload `probe_check` made.
unsafe extern "C" fn probe_cleanup(_filter: *mut ffi::git_filter, payload: *mut c_void) {
    // SAFETY: the payload `probe_check` boxed, handed back once by the list's free.
    let value = unsafe { Box::from_raw(payload.cast::<u64>()) };
    assert_eq!(*value, 0x5eed);
    PAYLOADS.with_borrow_mut(|p| p.1 += 1);
}

/// Registers the probe once; it is never unregistered, so outlives every list. Then
/// loads a list selecting it on this thread, before any test loads its own: libgit2
/// marks a registered filter initialised, unsynchronised, the first time a list selects
/// it (the crate does so for the built-in ones only).
fn probe() {
    struct Registered;
    static PROBE: OnceLock<Registered> = OnceLock::new();
    PROBE.get_or_init(|| {
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
        let wd = Workdir::make("prime");
        drop(FilterList::load(wd.repo(), c"a.probe", ffi::GIT_FILTER_TO_ODB, 0).unwrap());
        let _ = seen();
        Registered
    });
}

/// The ids the probe saw since the last call, and the payloads still unfreed.
fn seen() -> (Vec<Option<String>>, usize) {
    let seen = SEEN.with_borrow_mut(std::mem::take);
    let (made, freed) = PAYLOADS.with_borrow(|p| *p);
    (seen, made - freed)
}

/// A fresh repository whose `.gitattributes` gives `*.probe` the probe and `*.id`
/// libgit2's `ident` filter; freed and removed on drop.
struct Workdir {
    path: PathBuf,
    repo: Option<RepositoryBox>,
}

impl Workdir {
    fn new(tag: &str) -> Self {
        probe();
        Self::make(tag)
    }

    fn make(tag: &str) -> Self {
        // SAFETY: balanced by nothing: the reference lives for the process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
        let path = std::env::temp_dir().join(format!(
            "crustify-filter-ub-unsafe-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let repo = Repository::init(&c_path, false).unwrap();
        std::fs::write(
            path.join(".gitattributes"),
            "*.probe crustify-probe\n*.id ident\n",
        )
        .unwrap();
        Self {
            path,
            repo: Some(repo),
        }
    }

    fn repo(&self) -> RepositoryRef<'_> {
        self.repo.as_ref().unwrap().as_ref()
    }

    /// A new blob of `content` in this repository, looked up, and its id: the caller
    /// frees it.
    fn blob(&self, content: &[u8]) -> (NonNull<ffi::git_blob>, String) {
        let raw = self.repo.as_ref().unwrap().as_c_ptr();
        let mut id = ffi::git_oid::default();
        let mut blob = ptr::null_mut();
        // SAFETY: writable slots, the live repository and `content`, read for the call.
        unsafe {
            let rc = ffi::git_blob_create_from_buffer(
                &mut id,
                raw,
                content.as_ptr().cast(),
                content.len(),
            );
            assert_eq!(rc, 0);
            assert_eq!(ffi::git_blob_lookup(&mut blob, raw, &id), 0);
        }
        (NonNull::new(blob).unwrap(), hex(&id))
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

fn length(list: &FilterListBox<'_>) -> usize {
    // SAFETY: the live list, only read.
    unsafe { ffi::git_filter_list_length(list.as_c_ptr()) }
}

mod load_with_blob {
    use super::*;

    /// The probe's `check` sees the blob's id, and libgit2's `ident` filter (which has
    /// no `check`) nothing; the blob goes before the list, whose free hands the probe's
    /// payload back to its `cleanup`.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn checks_see_the_id_and_the_list_keeps_no_blob() {
        isolated(
            concat!(
                module_path!(),
                "::checks_see_the_id_and_the_list_keeps_no_blob"
            ),
            || {
                let wd = Workdir::new("blob-freed");
                let _ = seen();
                for path in [c"a.probe", c"a.id"] {
                    for mode in [ffi::GIT_FILTER_TO_WORKTREE, ffi::GIT_FILTER_TO_ODB] {
                        let (blob, id) = wd.blob(b"$Id$\n");
                        // SAFETY: `blob` is live for the call.
                        let list =
                            unsafe { FilterList::load_with_blob(wd.repo(), blob, path, mode, 0) };
                        free_blob(blob);
                        let list = list.unwrap().unwrap();
                        assert_eq!(length(&list), 1);
                        let expect = if path == c"a.probe" {
                            (vec![Some(id)], 1)
                        } else {
                            (vec![], 0)
                        };
                        assert_eq!(seen(), expect);
                        drop(list);
                        assert_eq!(seen().1, 0);
                    }
                }
            },
        );
    }

    /// A blob of another repository: only its id is read, and that repository may go
    /// before the list.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn blob_of_another_repository() {
        isolated(
            concat!(module_path!(), "::blob_of_another_repository"),
            || {
                let wd = Workdir::new("blob-own");
                let other = Workdir::new("blob-other");
                let _ = seen();
                let (blob, id) = other.blob(b"elsewhere\n");
                // SAFETY: `blob` is live for the call.
                let list = unsafe {
                    FilterList::load_with_blob(
                        wd.repo(),
                        blob,
                        c"x.probe",
                        ffi::GIT_FILTER_TO_ODB,
                        0,
                    )
                };
                free_blob(blob);
                drop(other);
                let list = list.unwrap().unwrap();
                assert_eq!(seen(), (vec![Some(id)], 1));
                drop(list);
                assert_eq!(seen().1, 0);
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn edge_cases_build_nothing_and_leave_the_blob_alone() {
        isolated(
            concat!(
                module_path!(),
                "::edge_cases_build_nothing_and_leave_the_blob_alone"
            ),
            || {
                let wd = Workdir::new("blob-edges");
                let _ = seen();
                let (blob, _) = wd.blob(b"x");
                for _ in 0..3 {
                    // SAFETY: `blob` is live for each call, freed after the loop.
                    unsafe {
                        // No filter for the path.
                        assert!(
                            FilterList::load_with_blob(wd.repo(), blob, c"a.bin", 0, 0)
                                .unwrap()
                                .is_none()
                        );
                        // An unborn `HEAD` fails the load.
                        let error = FilterList::load_with_blob(
                            wd.repo(),
                            blob,
                            c"a.probe",
                            0,
                            ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
                        )
                        .map(|_| ())
                        .unwrap_err();
                        assert_eq!(error.class(), Some(ErrorClass::Reference));
                        // Absolute: refused before libgit2, so before any check.
                        let error = FilterList::load_with_blob(wd.repo(), blob, c"/a.probe", 0, 0)
                            .map(|_| ())
                            .unwrap_err();
                        assert_eq!(error.class(), Some(ErrorClass::Internal));
                    }
                }
                assert_eq!(seen(), (vec![], 0));
                // The blob is still the caller's: freed once (ASan flags a double free).
                free_blob(blob);
            },
        );
    }
}

mod load_ext_with_blob {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn checks_see_the_id_and_the_list_keeps_no_blob() {
        isolated(
            concat!(
                module_path!(),
                "::checks_see_the_id_and_the_list_keeps_no_blob"
            ),
            || {
                let wd = Workdir::new("ext-blob-freed");
                let _ = seen();
                let opts = ffi::git_filter_options {
                    version: 1,
                    flags: ffi::GIT_FILTER_NO_SYSTEM_ATTRIBUTES,
                    ..Default::default()
                };
                for opts in [None, Some(&opts)] {
                    let (blob, id) = wd.blob(b"ext\n");
                    // SAFETY: `blob` is live for the call.
                    let list = unsafe {
                        FilterList::load_ext_with_blob(
                            wd.repo(),
                            blob,
                            c"a.probe",
                            ffi::GIT_FILTER_TO_WORKTREE,
                            opts,
                        )
                    };
                    free_blob(blob);
                    let list = list.unwrap().unwrap();
                    assert_eq!(seen(), (vec![Some(id)], 1));
                    drop(list);
                    assert_eq!(seen().1, 0);
                }
            },
        );
    }

    /// Options read from a commit that does not exist, which libgit2 skips, and options
    /// whose `reserved` pointer dangles, which libgit2 never reads; neither outlives the
    /// call.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn options_are_only_copied() {
        isolated(concat!(module_path!(), "::options_are_only_copied"), || {
            let wd = Workdir::new("ext-opts");
            let other = Workdir::new("ext-opts-other");
            let _ = seen();
            let (blob, id) = other.blob(b"opts\n");
            let list = {
                let mut opts = ffi::git_filter_options {
                    version: 1,
                    flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_COMMIT,
                    reserved: NonNull::<u64>::dangling().as_ptr().cast(),
                    ..Default::default()
                };
                opts.attr_commit_id.id[..4].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
                // SAFETY: `blob` is live for the call.
                unsafe {
                    FilterList::load_ext_with_blob(
                        wd.repo(),
                        blob,
                        c"a.probe",
                        ffi::GIT_FILTER_TO_ODB,
                        Some(&opts),
                    )
                }
            };
            free_blob(blob);
            drop(other);
            let list = list.unwrap().unwrap();
            assert_eq!(seen(), (vec![Some(id)], 1));
            drop(list);
            assert_eq!(seen().1, 0);
        });
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn edge_cases_build_nothing_and_leave_the_blob_alone() {
        isolated(
            concat!(
                module_path!(),
                "::edge_cases_build_nothing_and_leave_the_blob_alone"
            ),
            || {
                let wd = Workdir::new("ext-blob-edges");
                let _ = seen();
                let (blob, _) = wd.blob(b"y");
                let head = ffi::git_filter_options {
                    version: 1,
                    flags: ffi::GIT_FILTER_ATTRIBUTES_FROM_HEAD,
                    ..Default::default()
                };
                // SAFETY: `blob` is live for each call, freed after.
                unsafe {
                    assert!(
                        FilterList::load_ext_with_blob(wd.repo(), blob, c"a.bin", 0, None)
                            .unwrap()
                            .is_none()
                    );
                    assert!(
                        FilterList::load_ext_with_blob(wd.repo(), blob, c"a.probe", 0, Some(&head))
                            .is_err()
                    );
                    let error =
                        FilterList::load_ext_with_blob(wd.repo(), blob, c"//a.probe", 0, None)
                            .map(|_| ())
                            .unwrap_err();
                    assert_eq!(error.class(), Some(ErrorClass::Internal));
                }
                assert_eq!(seen(), (vec![], 0));
                free_blob(blob);
            },
        );
    }
}
