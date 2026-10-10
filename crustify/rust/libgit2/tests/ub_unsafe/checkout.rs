//! UB tests for the unsafe API of `git2/checkout.h`: the raw seams of
//! [`CheckoutOptionsMut`], one suite each, called within their contracts, edge cases
//! included, and then handed to real checkouts.
//!
//! The contracts keep what a seam stores live until the options are dropped or the
//! field is replaced, and the callbacks sound for whatever libgit2 passes them. So a
//! checkout must read only what is live (ASan): a payload freed right
//! after it was replaced must never be reached again, the arguments a callback gets
//! must be readable in full, and dropping the options must release nothing. Callbacks
//! reenter libgit2 and the Rust API, and checkouts on several threads, each with its
//! own options, repository and callbacks, must race nothing (TSan). The baseline tree
//! and index are no seams: their safe setters borrow a `TreeRef` and an `IndexRef`,
//! tested in `ub_safe`.
//!
//! The checkout routines are not wrapped yet, so the options reach
//! `git_checkout_tree` through their documented `#[repr(transparent)]` layout, which
//! [`c_ptr`] relies on; Miri cannot call libgit2, so every test is ignored there.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::OnceLock;

use libgit2::ffi;
use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository};
use libgit2::libgit2::workdir::checkout::CheckoutOptions;

/// `git_checkout_strategy_t` bits, from `git2/checkout.h`: the `-sys` crate does not bind
/// the enum, whose header comment bindgen copies into a doc comment rustdoc would run as
/// a doctest.
const GIT_CHECKOUT_FORCE: u32 = 1 << 1;
const GIT_CHECKOUT_DRY_RUN: u32 = 1 << 24;

/// The C struct a [`CheckoutOptions`] is laid out as; stands in for the checkout
/// wrappers.
fn c_ptr(options: &CheckoutOptions<'_>) -> *const ffi::git_checkout_options {
    ptr::from_ref(options).cast()
}

/// A non-bare repository with two trees, `old` (`a.txt`, `b.txt`) and `new` (`a.txt`
/// changed, `c.txt` added), whose working directory and index hold `old`; released
/// on drop.
struct Repo {
    root: PathBuf,
    repo: *mut ffi::git_repository,
    old: *mut ffi::git_tree,
    new: *mut ffi::git_tree,
    target: CString,
}

impl Repo {
    fn new(tag: &str) -> Self {
        // libgit2 marks a filter initialised on its first load outside the registry
        // lock (`filter_initialize`), so concurrent first loads race (TSan): the first
        // repository, whose files load the filters, is built alone.
        static PRIMED: OnceLock<()> = OnceLock::new();
        PRIMED.get_or_init(|| drop(Self::build("prime")));
        Self::build(tag)
    }

    fn build(tag: &str) -> Self {
        // SAFETY: initialising the library has no precondition.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
        let root = std::env::temp_dir().join(format!(
            "crustify-checkout-ub-unsafe-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let work = root.join("work");
        let c_work = CString::new(work.as_os_str().as_bytes()).unwrap();
        let mut opts = InitOptions::new();
        opts.as_mut().set_flags(InitFlag::Mkpath.into());
        // The raw repository the raw tree and index calls below use; `Drop` frees it.
        let repo = Repository::init_ext(&c_work, opts.as_mut())
            .unwrap()
            .into_c();
        let mut index = ptr::null_mut();
        // SAFETY: `repo` is live; the result is a reference this function releases.
        assert_eq!(unsafe { ffi::git_repository_index(&mut index, repo) }, 0);
        let tree = |files: &[(&str, &str)]| {
            for (path, content) in files {
                fs::write(work.join(path), content).unwrap();
                let path = CString::new(*path).unwrap();
                // SAFETY: the index is live and `path` NUL-terminated.
                let rc = unsafe { ffi::git_index_add_bypath(index, path.as_ptr()) };
                assert_eq!(rc, 0);
            }
            let mut id = ffi::git_oid::default();
            // SAFETY: a live out-parameter and a live index.
            assert_eq!(unsafe { ffi::git_index_write_tree(&mut id, index) }, 0);
            let mut tree = ptr::null_mut();
            // SAFETY: a live slot; the result is a reference the `Repo` owns.
            assert_eq!(unsafe { ffi::git_tree_lookup(&mut tree, repo, &id) }, 0);
            tree
        };
        let new = tree(&[("a.txt", "new\n"), ("b.txt", "b\n"), ("c.txt", "c\n")]);
        fs::remove_file(work.join("c.txt")).unwrap();
        let old = tree(&[("a.txt", "old\n"), ("b.txt", "b\n")]);
        // SAFETY: the index and tree are live; then the index reference is released.
        unsafe {
            assert_eq!(ffi::git_index_read_tree(index, old), 0);
            assert_eq!(ffi::git_index_write(index), 0);
            ffi::git_index_free(index);
        }
        let target = CString::new(root.join("out").as_os_str().as_bytes()).unwrap();
        Self {
            root,
            repo,
            old,
            new,
            target,
        }
    }

    /// Checks `tree` out with `options`.
    fn checkout(&self, tree: *mut ffi::git_tree, options: &CheckoutOptions<'_>) -> c_int {
        // SAFETY: the repository and tree are live, and the options' contracts hold:
        // whatever their seams store is live and their callbacks sound.
        unsafe { ffi::git_checkout_tree(self.repo, tree.cast(), c_ptr(options)) }
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        // SAFETY: the trees and repository are the references `new` took, freed once.
        unsafe {
            ffi::git_tree_free(self.old);
            ffi::git_tree_free(self.new);
            ffi::git_repository_free(self.repo);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// What the callbacks saw, through their payload.
#[derive(Debug, Default)]
struct Log {
    calls: Vec<String>,
    cancel: c_int,
}

/// The log behind a payload: a `RefCell<Log>` live for the call.
fn log<'a>(payload: *mut c_void) -> &'a RefCell<Log> {
    // SAFETY: every payload these tests install is a live `RefCell<Log>`.
    unsafe { &*payload.cast::<RefCell<Log>>() }
}

fn payload(log: &RefCell<Log>) -> *mut c_void {
    ptr::from_ref(log).cast_mut().cast()
}

/// Reads a whole C string, or `"<null>"`.
fn text(s: *const c_char) -> String {
    if s.is_null() {
        return "<null>".to_owned();
    }
    // SAFETY: libgit2 passes NUL-terminated strings live for the callback.
    unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

/// Reads a whole diff file, or `"-"`.
fn file(f: *const ffi::git_diff_file) -> String {
    if f.is_null() {
        return "-".to_owned();
    }
    // SAFETY: libgit2 passes diff files live for the callback.
    let f = unsafe { &*f };
    format!(
        "{}:{:o}:{}:{:?}",
        text(f.path),
        f.mode,
        f.size,
        &f.id.id[..4]
    )
}

mod set_notify_callback {
    use super::*;

    extern "C" fn notify(
        why: ffi::git_checkout_notify_t,
        path: *const c_char,
        baseline: *const ffi::git_diff_file,
        target: *const ffi::git_diff_file,
        workdir: *const ffi::git_diff_file,
        payload: *mut c_void,
    ) -> c_int {
        let mut log = log(payload).borrow_mut();
        let line = format!(
            "{why} {} {} {} {}",
            text(path),
            file(baseline),
            file(target),
            file(workdir)
        );
        log.calls.push(line);
        log.cancel
    }

    /// Reenters the Rust API and libgit2 from inside the callback.
    extern "C" fn reentrant(
        why: ffi::git_checkout_notify_t,
        path: *const c_char,
        baseline: *const ffi::git_diff_file,
        target: *const ffi::git_diff_file,
        workdir: *const ffi::git_diff_file,
        payload: *mut c_void,
    ) -> c_int {
        let label = CString::new(text(path)).unwrap();
        let mut inner = CheckoutOptions::new();
        inner.as_mut().set_our_label(Some(&label));
        assert_eq!(inner.as_ref().our_label(), Some(label.as_c_str()));
        // SAFETY: `git_error_last` is callable at any time after init.
        std::hint::black_box(unsafe { ffi::git_error_last() });
        notify(why, path, baseline, target, workdir, payload)
    }

    const NOTIFY: ffi::git_checkout_notify_cb = Some(notify);

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn arguments_are_readable_and_the_payload_reached() {
        let repo = Repo::new("notify-args");
        let log = RefCell::new(Log::default());
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: `notify` reads every argument only during the call, and the payload
        // is a `RefCell<Log>` that outlives the options.
        unsafe { m.set_notify_callback(NOTIFY, payload(&log)) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        let calls = log.borrow().calls.len();
        assert!(calls > 0);
        // Again, into the now populated target, and over the working directory.
        assert_eq!(repo.checkout(repo.new, &options), 0);
        options.as_mut().set_target_directory(None);
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert!(log.borrow().calls.len() > calls, "{:?}", log.borrow().calls);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_replaced_payload_may_be_freed() {
        let repo = Repo::new("notify-replaced");
        let first = Box::new(RefCell::new(Log::default()));
        let second = RefCell::new(Log::default());
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: `first` lives until the callback is replaced below.
        unsafe { m.set_notify_callback(NOTIFY, payload(&first)) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert!(!first.borrow().calls.is_empty());
        // SAFETY: `second` outlives the options.
        unsafe {
            options
                .as_mut()
                .set_notify_callback(NOTIFY, payload(&second))
        };
        drop(first);
        assert_eq!(repo.checkout(repo.old, &options), 0);
        assert!(!second.borrow().calls.is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_cancel_is_returned_and_no_file_is_written() {
        let repo = Repo::new("notify-cancel");
        let log = RefCell::new(Log {
            cancel: -1234,
            ..Log::default()
        });
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: as above.
        unsafe { m.set_notify_callback(NOTIFY, payload(&log)) };
        assert_eq!(repo.checkout(repo.new, &options), -1234);
        assert_eq!(log.borrow().calls.len(), 1);
        assert!(!repo.root.join("out/a.txt").exists());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn the_callback_may_reenter() {
        let repo = Repo::new("notify-reenter");
        let log = RefCell::new(Log::default());
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: `reentrant` builds its own options and calls a reentrant libgit2
        // routine; the payload outlives the options.
        unsafe { m.set_notify_callback(Some(reentrant), payload(&log)) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert!(!log.borrow().calls.is_empty());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn no_callback_ignores_the_payload_and_flags() {
        let repo = Repo::new("notify-none");
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: without a callback the payload is never used: any pointer will do.
        unsafe { m.set_notify_callback(None, ptr::dangling_mut::<u64>().cast()) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert!(repo.root.join("out/c.txt").exists());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn checkouts_on_threads_share_nothing() {
        std::thread::scope(|s| {
            for t in 0..4 {
                s.spawn(move || {
                    let repo = Repo::new(&format!("notify-thread-{t}"));
                    let log = RefCell::new(Log::default());
                    let mut options = CheckoutOptions::new();
                    let mut m = options.as_mut();
                    m.set_strategy(GIT_CHECKOUT_FORCE);
                    m.set_notify_flags(ffi::GIT_CHECKOUT_NOTIFY_ALL);
                    m.set_target_directory(Some(&repo.target));
                    // SAFETY: as above; the callback runs on this thread only.
                    unsafe { m.set_notify_callback(NOTIFY, payload(&log)) };
                    for _ in 0..3 {
                        assert_eq!(repo.checkout(repo.new, &options), 0);
                        assert_eq!(repo.checkout(repo.old, &options), 0);
                    }
                    assert!(!log.borrow().calls.is_empty());
                });
            }
        });
    }
}

mod set_progress_callback {
    use super::*;

    extern "C" fn progress(
        path: *const c_char,
        completed: usize,
        total: usize,
        payload: *mut c_void,
    ) {
        assert!(completed <= total);
        log(payload)
            .borrow_mut()
            .calls
            .push(format!("{} {completed}/{total}", text(path)));
    }

    const PROGRESS: ffi::git_checkout_progress_cb = Some(progress);

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn every_report_is_readable_including_the_null_path() {
        let repo = Repo::new("progress-args");
        let log = RefCell::new(Log::default());
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: `progress` reads the path only during the call and accepts NULL; the
        // payload outlives the options.
        unsafe { m.set_progress_callback(PROGRESS, payload(&log)) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        let calls = log.borrow().calls.clone();
        assert!(calls.iter().any(|c| c.starts_with("<null>")), "{calls:?}");
        assert!(calls.iter().any(|c| c.starts_with("c.txt")), "{calls:?}");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_replaced_payload_may_be_freed() {
        let repo = Repo::new("progress-replaced");
        let first = Box::new(RefCell::new(Log::default()));
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: `first` lives until the callback is cleared below.
        unsafe { m.set_progress_callback(PROGRESS, payload(&first)) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert!(!first.borrow().calls.is_empty());
        // SAFETY: clearing stores nothing libgit2 would call.
        unsafe {
            options
                .as_mut()
                .set_progress_callback(None, ptr::null_mut())
        };
        drop(first);
        assert_eq!(repo.checkout(repo.old, &options), 0);
    }
}

mod set_perfdata_callback {
    use super::*;

    extern "C" fn perfdata(data: *const ffi::git_checkout_perfdata, payload: *mut c_void) {
        // SAFETY: libgit2 passes its counters, live for the call; they are copied.
        let d = unsafe { data.read() };
        let line = format!("{} {} {}", d.mkdir_calls, d.stat_calls, d.chmod_calls);
        log(payload).borrow_mut().calls.push(line);
    }

    const PERFDATA: ffi::git_checkout_perfdata_cb = Some(perfdata);

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reported_once_per_successful_checkout() {
        let repo = Repo::new("perfdata");
        let log = RefCell::new(Log::default());
        let mut options = CheckoutOptions::new();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_target_directory(Some(&repo.target));
        // SAFETY: `perfdata` copies the counters during the call; the payload outlives
        // the options.
        unsafe { m.set_perfdata_callback(PERFDATA, payload(&log)) };
        assert_eq!(repo.checkout(repo.new, &options), 0);
        assert_eq!(log.borrow().calls.len(), 1);
        options
            .as_mut()
            .set_strategy(GIT_CHECKOUT_FORCE | GIT_CHECKOUT_DRY_RUN);
        assert_eq!(repo.checkout(repo.old, &options), 0);
        // A failing checkout reports nothing: the target is a file.
        let file = repo.root.join("blocker");
        fs::write(&file, "x").unwrap();
        let blocker = CString::new(file.as_os_str().as_bytes()).unwrap();
        let mut m = options.as_mut();
        m.set_strategy(GIT_CHECKOUT_FORCE);
        m.set_target_directory(Some(&blocker));
        let calls = log.borrow().calls.len();
        assert_ne!(repo.checkout(repo.new, &options), 0);
        assert_eq!(log.borrow().calls.len(), calls);
    }
}
