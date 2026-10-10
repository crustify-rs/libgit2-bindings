//! Equivalence of [`CheckoutOptions`] with the `git_checkout_options` C code builds.
//!
//! [`CheckoutOptions::new`] must build what `git_checkout_options_init` and
//! `GIT_CHECKOUT_OPTIONS_INIT` do. Then each sequence of option changes runs twice, on
//! independently built fixtures: once on a C struct written field by field, as C code
//! writes one, and once on a [`CheckoutOptions`] through its setters, each persisting
//! across the steps. After every step the getters must read what the C struct holds,
//! and a checkout must behave the same with either: libgit2's return value and error,
//! every file and directory it writes (content and mode), the index and working
//! directory it leaves, and every call it makes to the notification, progress and
//! performance callbacks. The checkout routines are not wrapped yet, so the Rust
//! options reach `git_checkout_tree` / `git_checkout_index` through their documented
//! `#[repr(transparent)]` layout, which [`c_ptr`] relies on.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use std::sync::OnceLock;

use libgit2::ffi;
use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::index::IndexArc;
use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository};
use libgit2::libgit2::workdir::checkout::{CheckoutOptions, CheckoutOptionsRef};

/// `git_checkout_strategy_t` bits, from `git2/checkout.h`: the `-sys` crate does not bind
/// the enum, whose header comment bindgen copies into a doc comment rustdoc would run as
/// a doctest.
const GIT_CHECKOUT_FORCE: u32 = 1 << 1;
const GIT_CHECKOUT_ALLOW_CONFLICTS: u32 = 1 << 4;
const GIT_CHECKOUT_SKIP_UNMERGED: u32 = 1 << 10;
const GIT_CHECKOUT_USE_OURS: u32 = 1 << 11;
const GIT_CHECKOUT_USE_THEIRS: u32 = 1 << 12;
const GIT_CHECKOUT_DISABLE_PATHSPEC_MATCH: u32 = 1 << 13;
const GIT_CHECKOUT_CONFLICT_STYLE_MERGE: u32 = 1 << 20;
const GIT_CHECKOUT_CONFLICT_STYLE_DIFF3: u32 = 1 << 21;
const GIT_CHECKOUT_DRY_RUN: u32 = 1 << 24;
const GIT_CHECKOUT_CONFLICT_STYLE_ZDIFF3: u32 = 1 << 25;

fn init() {
    // SAFETY: initialising the library has no precondition.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The calling thread's last error, with `root` spelled `<root>`.
fn c_last(root: &Path) -> (c_int, String) {
    // SAFETY: `git_error_last` never returns NULL after init, and its message is a
    // NUL-terminated string, both live until the next libgit2 call on this thread,
    // after the copy.
    let (class, message) = unsafe {
        let e = ffi::git_error_last();
        (
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        )
    };
    (class, message.replace(root.to_str().unwrap(), "<root>"))
}

/// The C struct a [`CheckoutOptions`] is laid out as: `#[repr(transparent)]` over
/// `git_checkout_options`, as its docs state. Stands in for the checkout wrappers.
fn c_ptr(options: &CheckoutOptions<'_>) -> *const ffi::git_checkout_options {
    ptr::from_ref(options).cast()
}

const TREE1: &[(&str, &str)] = &[
    (".gitattributes", "e.txt text eol=crlf\n"),
    ("a.txt", "one\n"),
    ("d.md", "four\n"),
    ("dir/c.txt", "three\n"),
];
const TREE2: &[(&str, &str)] = &[
    (".gitattributes", "e.txt text eol=crlf\n"),
    ("a.txt", "one changed\n"),
    ("d.md", "four\n"),
    ("dir/c.txt", "three\n"),
    ("e.txt", "five\nsix\n"),
];

/// A non-bare repository under a scratch root, with two trees, its index and ten
/// target directory names, all released on drop. The working directory and the index
/// hold `TREE1`, uncommitted (`HEAD` is unborn).
struct Fixture {
    root: PathBuf,
    work: PathBuf,
    repo: *mut ffi::git_repository,
    /// The repository's index, for C; `index_ref` holds the reference.
    index: *mut ffi::git_index,
    trees: [*mut ffi::git_tree; 2],
    targets: Vec<CString>,
    index_ref: IndexArc,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        // libgit2 marks a filter initialised on its first load outside the registry
        // lock (`filter_initialize`), so concurrent first loads race (TSan): the first
        // fixture, whose `.gitattributes` loads the CRLF filter, is built alone.
        static PRIMED: OnceLock<()> = OnceLock::new();
        PRIMED.get_or_init(|| drop(Self::build("prime")));
        Self::build(tag)
    }

    fn build(tag: &str) -> Self {
        init();
        let root = std::env::temp_dir().join(format!(
            "crustify-checkout-equiv-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let work = root.join("work");
        let c_work = CString::new(work.as_os_str().as_bytes()).unwrap();
        let mut opts = InitOptions::new();
        opts.as_mut().set_flags(InitFlag::Mkpath.into());
        // The raw repository both sides' calls use; `Drop` frees it.
        let repo = Repository::init_ext(&c_work, opts.as_mut())
            .unwrap()
            .into_c();
        let mut index = ptr::null_mut();
        // SAFETY: `repo` is live; the result is a new reference to its index.
        let rc = unsafe { ffi::git_repository_index(&mut index, repo) };
        assert_eq!(rc, 0);
        // SAFETY: one counted reference to the repository's index, which `git_index_free`
        // releases; the repository keeps another, so it is shared.
        let index_ref = unsafe { IndexArc::from_c(index) }.unwrap();
        let mut fixture = Self {
            index_ref,
            targets: (0..10)
                .map(|i| CString::new(root.join(format!("t{i}")).as_os_str().as_bytes()).unwrap())
                .collect(),
            root,
            work,
            repo,
            index,
            trees: [ptr::null_mut(); 2],
        };
        fixture.trees[0] = fixture.write_tree(TREE1);
        fixture.trees[1] = fixture.write_tree(TREE2);
        fixture.write_files(TREE1);
        let _ = fs::remove_file(fixture.work.join("e.txt"));
        // SAFETY: the index and tree are live.
        let rc = unsafe { ffi::git_index_read_tree(fixture.index, fixture.trees[0]) };
        assert_eq!(rc, 0);
        // SAFETY: the index is live and backed by the repository's index file.
        assert_eq!(unsafe { ffi::git_index_write(fixture.index) }, 0);
        fixture
    }

    fn write_files(&self, files: &[(&str, &str)]) {
        for (path, content) in files {
            let path = self.work.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }

    /// Writes `files` into the working directory and the index, and looks up the tree
    /// the index then holds.
    fn write_tree(&self, files: &[(&str, &str)]) -> *mut ffi::git_tree {
        self.write_files(files);
        for (path, _) in files {
            let path = CString::new(*path).unwrap();
            // SAFETY: the index is live and `path` NUL-terminated, read for the call.
            let rc = unsafe { ffi::git_index_add_bypath(self.index, path.as_ptr()) };
            assert_eq!(rc, 0, "{:?}", c_last(&self.root));
        }
        let mut id = ffi::git_oid::default();
        // SAFETY: `id` is a live out-parameter and the index is live.
        assert_eq!(unsafe { ffi::git_index_write_tree(&mut id, self.index) }, 0);
        let mut tree = ptr::null_mut();
        // SAFETY: `tree` is a live slot; the result is a new reference the fixture owns.
        let rc = unsafe { ffi::git_tree_lookup(&mut tree, self.repo, &id) };
        assert_eq!(rc, 0);
        tree
    }

    /// Adds conflicts to the index, in memory: a content conflict on `f.txt`
    /// (ancestor `a`, ours `b`, theirs `c`), and two directory/file conflicts, which
    /// libgit2 writes as side files named with the conflict labels: `g`, modified by
    /// us and deleted by them, against `g/h`, added by them, and `k`, modified by them
    /// and deleted by us, against `k/l`, added by us.
    fn add_conflict(&self) {
        let entry = |path: &'static CStr, content: &str| {
            let mut id = ffi::git_oid::default();
            // SAFETY: `id` is a live out-parameter; the buffer is read for the call.
            let rc = unsafe {
                ffi::git_blob_create_from_buffer(
                    &mut id,
                    self.repo,
                    content.as_ptr().cast(),
                    content.len(),
                )
            };
            assert_eq!(rc, 0);
            ffi::git_index_entry {
                mode: 0o100644,
                id,
                path: path.as_ptr(),
                ..Default::default()
            }
        };
        let side = |e: &Option<ffi::git_index_entry>| e.as_ref().map_or(ptr::null(), ptr::from_ref);
        let conflicts = [
            (
                Some(entry(c"f.txt", "a\nx\n")),
                Some(entry(c"f.txt", "b\nx\n")),
                Some(entry(c"f.txt", "c\nx\n")),
            ),
            (Some(entry(c"g", "g0\n")), Some(entry(c"g", "g1\n")), None),
            (None, None, Some(entry(c"g/h", "h\n"))),
            (Some(entry(c"k", "k0\n")), None, Some(entry(c"k", "k1\n"))),
            (None, Some(entry(c"k/l", "l\n")), None),
        ];
        for (ancestor, ours, theirs) in &conflicts {
            // SAFETY: the index is live and the entries are read for the call; it
            // copies their paths.
            let rc = unsafe {
                ffi::git_index_conflict_add(self.index, side(ancestor), side(ours), side(theirs))
            };
            assert_eq!(rc, 0, "{:?}", c_last(&self.root));
        }
    }

    /// Every file and directory under the root, but `.git`: its path, its content
    /// (`None` for a directory) and its permission bits.
    fn snapshot(&self) -> Vec<(String, Option<Vec<u8>>, u32)> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Option<Vec<u8>>, u32)>) {
            let mut entries: Vec<_> = fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                if path.file_name().unwrap() == ".git" {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).unwrap();
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let mode = meta.permissions().mode() & 0o7777;
                if meta.is_dir() {
                    out.push((name, None, mode));
                    walk(root, &path, out);
                } else {
                    out.push((name, Some(fs::read(&path).unwrap()), mode));
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out
    }

    /// The tree or index a C baseline pointer designates, by name.
    fn name_of(&self, tree: *mut ffi::git_tree, index: *mut ffi::git_index) -> String {
        let tree = match self.trees.iter().position(|&t| t == tree) {
            _ if tree.is_null() => "none".to_owned(),
            Some(i) => format!("tree{i}"),
            None => "unknown".to_owned(),
        };
        let index = match index {
            i if i.is_null() => "none",
            i if i == self.index => "index",
            _ => "unknown",
        };
        format!("{tree}/{index}")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // SAFETY: each object is the live reference `new` took, freed once, after every
        // options value pointing at it; `index_ref` releases the index's.
        unsafe {
            for tree in self.trees {
                ffi::git_tree_free(tree);
            }
            ffi::git_repository_free(self.repo);
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// What the callbacks saw, through their payload. `cancel` is what the notification
/// callback returns.
#[derive(Debug, Default, PartialEq)]
struct Log {
    notify: Vec<(u32, String, [Option<String>; 3])>,
    progress: Vec<(Option<String>, usize, usize)>,
    perfdata: Vec<(usize, usize, usize)>,
    cancel: c_int,
}

/// The log behind a payload.
fn log<'a>(payload: *mut c_void) -> &'a RefCell<Log> {
    // SAFETY: every payload these tests install is a `&RefCell<Log>` that outlives the
    // checkout calling back with it.
    unsafe { &*payload.cast::<RefCell<Log>>() }
}

fn string(s: *const c_char) -> Option<String> {
    // SAFETY: libgit2 passes NULL or a NUL-terminated string live for the callback.
    (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned())
}

fn diff_path(file: *const ffi::git_diff_file) -> Option<String> {
    // SAFETY: libgit2 passes NULL or a diff file live for the callback.
    (!file.is_null()).then(|| string(unsafe { (*file).path }).unwrap_or_default())
}

extern "C" fn notify(
    why: ffi::git_checkout_notify_t,
    path: *const c_char,
    baseline: *const ffi::git_diff_file,
    target: *const ffi::git_diff_file,
    workdir: *const ffi::git_diff_file,
    payload: *mut c_void,
) -> c_int {
    let mut log = log(payload).borrow_mut();
    let files = [diff_path(baseline), diff_path(target), diff_path(workdir)];
    log.notify
        .push((why, string(path).unwrap_or_default(), files));
    log.cancel
}

extern "C" fn progress(path: *const c_char, completed: usize, total: usize, payload: *mut c_void) {
    log(payload)
        .borrow_mut()
        .progress
        .push((string(path), completed, total));
}

extern "C" fn perfdata(data: *const ffi::git_checkout_perfdata, payload: *mut c_void) {
    // SAFETY: libgit2 passes its live counters for the callback.
    let d = unsafe { &*data };
    log(payload)
        .borrow_mut()
        .perfdata
        .push((d.mkdir_calls, d.stat_calls, d.chmod_calls));
}

const NOTIFY: ffi::git_checkout_notify_cb = Some(notify);
const PROGRESS: ffi::git_checkout_progress_cb = Some(progress);
const PERFDATA: ffi::git_checkout_perfdata_cb = Some(perfdata);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Baseline {
    #[default]
    Head,
    Tree(usize),
    Index,
}

#[derive(Clone, Copy, Debug, Default)]
enum Notify {
    #[default]
    Off,
    On,
    Cancel(c_int),
}

/// One step: the options it sets, and the checkout it runs with them.
#[derive(Clone, Copy, Debug, Default)]
struct Step {
    /// Check out the index (`git_checkout_index`) instead of `trees[tree]`.
    index: bool,
    tree: usize,
    strategy: u32,
    disable_filters: bool,
    dir_mode: u32,
    file_mode: u32,
    open_flags: i32,
    notify_flags: u32,
    paths: &'static [&'static CStr],
    target: Option<usize>,
    labels: [Option<&'static CStr>; 3],
    baseline: Baseline,
    notify: Notify,
    progress: bool,
    perfdata: bool,
    /// A file written into the working directory before the checkout.
    write: Option<(&'static str, &'static str)>,
}

impl Step {
    fn baseline(&self, f: &Fixture) -> (*mut ffi::git_tree, *mut ffi::git_index) {
        match self.baseline {
            Baseline::Head => (ptr::null_mut(), ptr::null_mut()),
            Baseline::Tree(i) => (f.trees[i], ptr::null_mut()),
            Baseline::Index => (ptr::null_mut(), f.index),
        }
    }

    /// Writes the step's options into a C struct, field by field.
    fn apply_c(
        &self,
        o: &mut ffi::git_checkout_options,
        f: &Fixture,
        log: &RefCell<Log>,
        paths: &[*mut c_char],
    ) {
        let payload = ptr::from_ref(log).cast_mut().cast::<c_void>();
        o.checkout_strategy = self.strategy;
        o.disable_filters = c_int::from(self.disable_filters);
        o.dir_mode = self.dir_mode;
        o.file_mode = self.file_mode;
        o.file_open_flags = self.open_flags;
        o.notify_flags = self.notify_flags;
        (o.notify_cb, o.notify_payload) = match self.notify {
            Notify::Off => (None, ptr::null_mut()),
            Notify::On | Notify::Cancel(_) => (NOTIFY, payload),
        };
        (o.progress_cb, o.progress_payload) = if self.progress {
            (PROGRESS, payload)
        } else {
            (None, ptr::null_mut())
        };
        (o.perfdata_cb, o.perfdata_payload) = if self.perfdata {
            (PERFDATA, payload)
        } else {
            (None, ptr::null_mut())
        };
        o.paths = ffi::git_strarray {
            strings: paths.as_ptr().cast_mut(),
            count: paths.len(),
        };
        (o.baseline, o.baseline_index) = self.baseline(f);
        o.target_directory = self.target.map_or(ptr::null(), |t| f.targets[t].as_ptr());
        let label = |l: Option<&CStr>| l.map_or(ptr::null(), CStr::as_ptr);
        o.ancestor_label = label(self.labels[0]);
        o.our_label = label(self.labels[1]);
        o.their_label = label(self.labels[2]);
    }

    /// Sets the step's options through the Rust setters.
    fn apply_rust<'o>(
        &self,
        options: &mut CheckoutOptions<'o>,
        f: &'o Fixture,
        log: &'o RefCell<Log>,
        paths: &'o StrArrayView<'o>,
    ) {
        let payload = ptr::from_ref(log).cast_mut().cast::<c_void>();
        let mut m = options.as_mut();
        m.set_strategy(self.strategy);
        m.set_disable_filters(self.disable_filters);
        m.set_dir_mode(self.dir_mode);
        m.set_file_mode(self.file_mode);
        m.set_file_open_flags(self.open_flags);
        m.set_notify_flags(self.notify_flags);
        let (tree, _) = self.baseline(f);
        m.set_baseline_index((self.baseline == Baseline::Index).then(|| f.index_ref.as_ref()));
        // SAFETY: the callbacks are sound for any arguments libgit2 passes, the payload
        // is a `RefCell<Log>` and the baseline tree the fixture's, all of which outlive
        // the options.
        unsafe {
            match self.notify {
                Notify::Off => m.set_notify_callback(None, ptr::null_mut()),
                Notify::On | Notify::Cancel(_) => m.set_notify_callback(NOTIFY, payload),
            }
            if self.progress {
                m.set_progress_callback(PROGRESS, payload);
            } else {
                m.set_progress_callback(None, ptr::null_mut());
            }
            if self.perfdata {
                m.set_perfdata_callback(PERFDATA, payload);
            } else {
                m.set_perfdata_callback(None, ptr::null_mut());
            }
            m.set_baseline(tree);
        }
        if self.paths.is_empty() {
            m.clear_paths();
        } else {
            m.set_paths(paths.as_ref());
        }
        m.set_target_directory(self.target.map(|t| f.targets[t].as_c_str()));
        m.set_ancestor_label(self.labels[0]);
        m.set_our_label(self.labels[1]);
        m.set_their_label(self.labels[2]);
    }

    /// Runs the step's checkout with the options at `options`.
    fn checkout(
        &self,
        f: &Fixture,
        log: &RefCell<Log>,
        options: *const ffi::git_checkout_options,
    ) -> Outcome {
        if let Some((path, content)) = self.write {
            fs::write(f.work.join(path), content).unwrap();
        }
        *log.borrow_mut() = Log {
            cancel: match self.notify {
                Notify::Cancel(rc) => rc,
                _ => 0,
            },
            ..Log::default()
        };
        // SAFETY: the repository, index and trees are the fixture's live objects, and
        // `options` addresses live options whose pointers are all live or NULL.
        let rc = unsafe {
            if self.index {
                ffi::git_checkout_index(f.repo, f.index, options)
            } else {
                ffi::git_checkout_tree(f.repo, f.trees[self.tree].cast(), options)
            }
        };
        let error = (rc != 0).then(|| c_last(&f.root));
        Outcome {
            rc,
            error,
            files: f.snapshot(),
            log: std::mem::take(&mut *log.borrow_mut()),
        }
    }
}

/// What a checkout did.
#[derive(Debug, PartialEq)]
struct Outcome {
    rc: c_int,
    error: Option<(c_int, String)>,
    files: Vec<(String, Option<Vec<u8>>, u32)>,
    log: Log,
}

/// The options' fields, comparable across fixtures: pointers by what they designate.
#[derive(Debug, PartialEq)]
struct Fields {
    version: u32,
    scalars: (u32, bool, u32, u32, i32, u32),
    callbacks: [(bool, bool); 3],
    paths: Vec<String>,
    baseline: String,
    strings: [Option<String>; 4],
}

fn c_fields(o: &ffi::git_checkout_options, f: &Fixture, log: &RefCell<Log>) -> Fields {
    let payload = ptr::from_ref(log).cast_mut().cast::<c_void>();
    let paths = (0..o.paths.count)
        .map(|i| {
            // SAFETY: the array holds `count` strings that outlive the options.
            let s = unsafe { *o.paths.strings.add(i) };
            string(s).unwrap()
        })
        .collect();
    Fields {
        version: o.version,
        scalars: (
            o.checkout_strategy,
            o.disable_filters != 0,
            o.dir_mode,
            o.file_mode,
            o.file_open_flags,
            o.notify_flags,
        ),
        callbacks: [
            (o.notify_cb.is_some(), o.notify_payload == payload),
            (o.progress_cb.is_some(), o.progress_payload == payload),
            (o.perfdata_cb.is_some(), o.perfdata_payload == payload),
        ],
        paths,
        baseline: f.name_of(o.baseline, o.baseline_index),
        strings: [
            o.target_directory,
            o.ancestor_label,
            o.our_label,
            o.their_label,
        ]
        .map(|s| string(s).map(|s| s.replace(f.root.to_str().unwrap(), "<root>"))),
    }
}

fn rust_fields(o: CheckoutOptionsRef<'_, '_>, f: &Fixture, log: &RefCell<Log>) -> Fields {
    let payload = ptr::from_ref(log).cast_mut().cast::<c_void>();
    let callback = |(set, p): (bool, *mut c_void)| (set, p == payload);
    let text = |s: Option<&CStr>| {
        s.map(|s| {
            s.to_string_lossy()
                .replace(f.root.to_str().unwrap(), "<root>")
        })
    };
    Fields {
        version: o.version(),
        scalars: (
            o.strategy(),
            o.disable_filters(),
            o.dir_mode(),
            o.file_mode(),
            o.file_open_flags(),
            o.notify_flags(),
        ),
        callbacks: [
            callback((o.notify_callback().0.is_some(), o.notify_callback().1)),
            callback((o.progress_callback().0.is_some(), o.progress_callback().1)),
            callback((o.perfdata_callback().0.is_some(), o.perfdata_callback().1)),
        ],
        paths: o
            .paths()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect(),
        // The fixture has one index; an `IndexRef` lends no pointer to compare.
        baseline: f.name_of(
            o.baseline(),
            o.baseline_index().map_or(ptr::null_mut(), |_| f.index),
        ),
        strings: [
            o.target_directory(),
            o.ancestor_label(),
            o.our_label(),
            o.their_label(),
        ]
        .map(text),
    }
}

/// Runs `steps` on a C struct and on a [`CheckoutOptions`], each over its own fixture
/// (`conflict` adds the index conflict to both), comparing after every step.
fn compare(tag: &str, conflict: bool, steps: &[Step]) -> Vec<Outcome> {
    let (cf, rf) = (
        Fixture::new(&format!("{tag}-c")),
        Fixture::new(&format!("{tag}-rust")),
    );
    if conflict {
        cf.add_conflict();
        rf.add_conflict();
    }
    let (clog, rlog) = (RefCell::new(Log::default()), RefCell::new(Log::default()));
    // The C side's `char *` arrays, and the Rust side's views, one per step, all
    // outliving the options.
    let c_paths: Vec<Vec<*mut c_char>> = steps
        .iter()
        .map(|s| s.paths.iter().map(|p| p.as_ptr().cast_mut()).collect())
        .collect();
    let views: Vec<StrArrayView<'_>> = steps.iter().map(|s| StrArrayView::new(s.paths)).collect();

    let mut c = ffi::git_checkout_options::default();
    // SAFETY: `c` is writable storage for the struct; version 1 is accepted.
    let rc = unsafe { ffi::git_checkout_options_init(&mut c, ffi::GIT_CHECKOUT_OPTIONS_VERSION) };
    assert_eq!(rc, 0);
    let mut rust = CheckoutOptions::new();
    assert_eq!(
        rust_fields(rust.as_ref(), &rf, &rlog),
        c_fields(&c, &cf, &clog)
    );

    let mut outcomes = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        step.apply_c(&mut c, &cf, &clog, &c_paths[i]);
        step.apply_rust(&mut rust, &rf, &rlog, &views[i]);
        assert_eq!(
            rust_fields(rust.as_ref(), &rf, &rlog),
            c_fields(&c, &cf, &clog),
            "options after step {i}: {step:?}"
        );
        let expected = step.checkout(&cf, &clog, &c);
        let actual = step.checkout(&rf, &rlog, c_ptr(&rust));
        assert_eq!(actual, expected, "checkout at step {i}: {step:?}");
        outcomes.push(actual);
    }
    outcomes
}

const FORCE: u32 = GIT_CHECKOUT_FORCE;
const ALL: u32 = ffi::GIT_CHECKOUT_NOTIFY_ALL;

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn new_matches_c_init() {
    let f = Fixture::new("init");
    let log = RefCell::new(Log::default());
    let mut c = ffi::git_checkout_options::default();
    // SAFETY: `c` is writable storage for the struct; version 1 is accepted.
    let rc = unsafe { ffi::git_checkout_options_init(&mut c, ffi::GIT_CHECKOUT_OPTIONS_VERSION) };
    assert_eq!(rc, 0);
    // `GIT_CHECKOUT_OPTIONS_INIT` is `{ GIT_CHECKOUT_OPTIONS_VERSION }`.
    let macro_init = ffi::git_checkout_options {
        version: 1,
        ..Default::default()
    };
    let expected = c_fields(&c, &f, &log);
    assert_eq!(c_fields(&macro_init, &f, &log), expected);
    assert_eq!(
        rust_fields(CheckoutOptions::new().as_ref(), &f, &log),
        expected
    );
    assert_eq!(
        rust_fields(CheckoutOptions::default().as_ref(), &f, &log),
        expected
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn tree_checkouts_match_step_by_step() {
    let forced = Step {
        tree: 1,
        strategy: FORCE,
        ..Step::default()
    };
    let steps = [
        // A safe checkout over an unborn `HEAD`: the dirty files conflict.
        Step {
            tree: 1,
            notify: Notify::On,
            notify_flags: ALL,
            ..Step::default()
        },
        Step {
            target: Some(0),
            notify: Notify::On,
            notify_flags: ALL,
            progress: true,
            perfdata: true,
            ..forced
        },
        Step {
            target: Some(1),
            paths: &[c"*.md"],
            progress: true,
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_DISABLE_PATHSPEC_MATCH,
            target: Some(2),
            paths: &[c"dir/c.txt", c"*.txt"],
            progress: true,
            ..forced
        },
        Step {
            target: Some(3),
            dir_mode: 0o700,
            file_mode: 0o600,
            ..forced
        },
        Step {
            target: Some(4),
            disable_filters: true,
            ..forced
        },
        Step {
            target: Some(5),
            disable_filters: false,
            perfdata: true,
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_DRY_RUN,
            target: Some(6),
            notify: Notify::On,
            notify_flags: ALL,
            progress: true,
            ..forced
        },
        Step {
            target: Some(7),
            open_flags: libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY,
            ..forced
        },
        // The same flags over files that exist already.
        Step {
            target: Some(0),
            open_flags: libc::O_CREAT | libc::O_EXCL | libc::O_WRONLY,
            dir_mode: 0o750,
            file_mode: 0o640,
            ..forced
        },
        // A safe update of the working directory, which matches the baseline tree.
        Step {
            tree: 1,
            baseline: Baseline::Tree(0),
            notify: Notify::On,
            notify_flags: ffi::GIT_CHECKOUT_NOTIFY_UPDATED | ffi::GIT_CHECKOUT_NOTIFY_DIRTY,
            ..Step::default()
        },
        // Back to tree 0 against the stale baseline: the working directory is dirty.
        Step {
            tree: 0,
            baseline: Baseline::Tree(0),
            notify: Notify::On,
            notify_flags: ALL,
            ..Step::default()
        },
        // The index as the baseline overrides it.
        Step {
            tree: 0,
            baseline: Baseline::Index,
            notify: Notify::On,
            notify_flags: ALL,
            progress: true,
            ..Step::default()
        },
        Step {
            notify: Notify::Cancel(7),
            notify_flags: ALL,
            ..forced
        },
        Step {
            notify: Notify::Cancel(-3),
            notify_flags: ffi::GIT_CHECKOUT_NOTIFY_UPDATED,
            ..forced
        },
        Step {
            target: Some(8),
            labels: [Some(c"base"), Some(c"mine"), Some(c"yours")],
            perfdata: true,
            ..forced
        },
        // Negative flags read as the default ones.
        Step {
            target: Some(9),
            open_flags: -1,
            ..forced
        },
        // Into the working directory, then safely back over a changed `e.txt`, which the
        // target deletes: a conflict.
        Step {
            notify: Notify::On,
            notify_flags: ALL,
            ..forced
        },
        Step {
            tree: 0,
            baseline: Baseline::Tree(1),
            write: Some(("e.txt", "dirty\n")),
            notify: Notify::On,
            notify_flags: ALL,
            progress: true,
            ..Step::default()
        },
        // Everything back to the defaults but the strategy.
        Step { tree: 0, ..forced },
    ];
    let outcomes = compare("tree", false, &steps);

    // The steps exercised what they set: not a vacuous comparison.
    let file = |o: &Outcome, name: &str| {
        o.files
            .iter()
            .find(|(n, ..)| n == name)
            .map(|(_, c, m)| (c.clone(), *m))
    };
    assert_ne!(outcomes[0].rc, 0, "the dirty safe checkout conflicts");
    assert!(!outcomes[1].log.notify.is_empty() && !outcomes[1].log.progress.is_empty());
    assert_eq!(outcomes[1].log.perfdata.len(), 1);
    assert!(file(&outcomes[2], "t1/d.md").is_some() && file(&outcomes[2], "t1/a.txt").is_none());
    assert!(
        file(&outcomes[3], "t2/dir/c.txt").is_some() && file(&outcomes[3], "t2/a.txt").is_none()
    );
    assert_eq!(
        file(&outcomes[4], "t3/a.txt").unwrap().1 & 0o777,
        0o600 & !umask()
    );
    assert_eq!(
        file(&outcomes[5], "t4/e.txt").unwrap().0.unwrap(),
        b"five\nsix\n"
    );
    assert_eq!(
        file(&outcomes[6], "t5/e.txt").unwrap().0.unwrap(),
        b"five\r\nsix\r\n"
    );
    assert!(
        !outcomes[7].files.iter().any(|(n, ..)| n.starts_with("t6/")),
        "a dry run writes no file (it does create the target directory)"
    );
    assert!(!outcomes[7].log.notify.is_empty());
    assert_eq!(outcomes[13].rc, 7);
    assert_eq!(outcomes[14].rc, -3);
    assert_eq!(
        file(&outcomes[16], "t9/a.txt").unwrap().0.unwrap(),
        b"one changed\n"
    );
    let deleted = &outcomes[18].log.notify;
    assert!(
        deleted.iter().any(|(_, path, [base, target, _])| {
            path == "e.txt" && base.is_some() && target.is_none()
        }),
        "{deleted:?}"
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn conflict_labels_match_step_by_step() {
    let forced = Step {
        index: true,
        strategy: FORCE,
        ..Step::default()
    };
    let steps = [
        Step {
            strategy: FORCE | GIT_CHECKOUT_CONFLICT_STYLE_DIFF3,
            target: Some(0),
            labels: [Some(c"base"), Some(c"mine"), Some(c"yours")],
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_CONFLICT_STYLE_MERGE,
            target: Some(1),
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_CONFLICT_STYLE_ZDIFF3,
            target: Some(2),
            labels: [Some(c"anc"), None, Some(c"theirs!")],
            ..forced
        },
        Step {
            strategy: GIT_CHECKOUT_ALLOW_CONFLICTS,
            target: Some(3),
            labels: [None, Some(c"left"), Some(c"right")],
            notify: Notify::On,
            notify_flags: ALL,
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_USE_OURS,
            target: Some(4),
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_USE_THEIRS,
            target: Some(5),
            ..forced
        },
        Step {
            strategy: FORCE | GIT_CHECKOUT_SKIP_UNMERGED,
            target: Some(6),
            ..forced
        },
    ];
    let outcomes = compare("conflict", true, &steps);
    let content = |o: &Outcome, name: &str| {
        let (_, c, _) = o.files.iter().find(|(n, ..)| n == name).unwrap();
        String::from_utf8(c.clone().unwrap()).unwrap()
    };
    let diff3 = content(&outcomes[0], "t0/f.txt");
    assert!(
        diff3.contains("<<<<<<< mine") && diff3.contains("||||||| base"),
        "{diff3}"
    );
    assert!(diff3.contains(">>>>>>> yours"), "{diff3}");
    // The directory/file conflicts' file sides are written under their labels.
    assert_eq!(content(&outcomes[0], "t0/g~mine"), "g1\n");
    assert_eq!(content(&outcomes[0], "t0/k~yours"), "k1\n");
    assert_eq!(content(&outcomes[1], "t1/k~theirs"), "k1\n");
    assert_eq!(content(&outcomes[1], "t1/g~ours"), "g1\n");
    assert_eq!(content(&outcomes[3], "t3/g~left"), "g1\n");
    let merge = content(&outcomes[1], "t1/f.txt");
    assert!(
        merge.contains("<<<<<<< ours") && merge.contains(">>>>>>> theirs"),
        "{merge}"
    );
    let zdiff3 = content(&outcomes[2], "t2/f.txt");
    assert!(
        zdiff3.contains("||||||| anc") && zdiff3.contains(">>>>>>> theirs!"),
        "{zdiff3}"
    );
    assert_eq!(content(&outcomes[4], "t4/f.txt"), "b\nx\n");
    assert_eq!(content(&outcomes[5], "t5/f.txt"), "c\nx\n");
}

/// A scratch root for a bare repository, and the target directory beside it.
fn bare_root(tag: &str) -> (PathBuf, CString) {
    let root = std::env::temp_dir().join(format!(
        "crustify-checkout-equiv-{}-bare-{tag}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let target = CString::new(root.join("out").as_os_str().as_bytes()).unwrap();
    (root, target)
}

/// Checks a one-file tree out of a fresh bare repository under `root` with the options
/// at `options`, and reports the result, the error and what the root then holds.
fn bare_checkout(
    root: &Path,
    options: *const ffi::git_checkout_options,
) -> (c_int, Option<(c_int, String)>, Vec<PathBuf>) {
    init();
    let c_git = CString::new(root.join("repo.git").as_os_str().as_bytes()).unwrap();
    let mut repo = ptr::null_mut();
    // SAFETY: a live slot and a NUL-terminated path; `1` asks for a bare repository.
    let rc = unsafe { ffi::git_repository_init(&mut repo, c_git.as_ptr(), 1) };
    assert_eq!(rc, 0);
    let mut blob = ffi::git_oid::default();
    let mut id = ffi::git_oid::default();
    let mut builder = ptr::null_mut();
    let mut tree = ptr::null_mut();
    // SAFETY: every out-parameter is live and every input read for the call; the
    // builder is freed once, after its last use.
    unsafe {
        let rc = ffi::git_blob_create_from_buffer(&mut blob, repo, c"x\n".as_ptr().cast(), 2);
        assert_eq!(rc, 0);
        assert_eq!(ffi::git_treebuilder_new(&mut builder, repo, ptr::null()), 0);
        let rc = ffi::git_treebuilder_insert(
            ptr::null_mut(),
            builder,
            c"x.txt".as_ptr(),
            &blob,
            ffi::GIT_FILEMODE_BLOB,
        );
        assert_eq!(rc, 0);
        assert_eq!(ffi::git_treebuilder_write(&mut id, builder), 0);
        ffi::git_treebuilder_free(builder);
        assert_eq!(ffi::git_tree_lookup(&mut tree, repo, &id), 0);
    }
    // SAFETY: the repository and tree are live, and so is everything `options` points
    // at, which the caller keeps alive across this call.
    let rc = unsafe { ffi::git_checkout_tree(repo, tree.cast(), options) };
    let error = (rc != 0).then(|| c_last(root));
    let mut files: Vec<PathBuf> = walkdir(root)
        .into_iter()
        .map(|p| p.strip_prefix(root).unwrap().to_owned())
        .filter(|p| !p.starts_with("repo.git"))
        .collect();
    files.sort();
    // SAFETY: the tree and repository are released once, after their last use.
    unsafe {
        ffi::git_tree_free(tree);
        ffi::git_repository_free(repo);
    }
    let _ = fs::remove_dir_all(root);
    (rc, error, files)
}

/// Every path under `dir`.
fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walkdir(&path));
        }
        out.push(path);
    }
    out
}

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, where no other thread calls libgit2.
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
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

/// A checkout from a bare repository runs alone: its ignore lookup writes the NUL
/// terminator into libgit2's shared empty string (`git_attr_path__init` over an empty
/// path, attr_file.c), which any other thread reading an empty `git_str` races (TSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_bare_repository_needs_a_target_directory() {
    isolated(
        concat!(
            module_path!(),
            "::a_bare_repository_needs_a_target_directory"
        ),
        bare_repository_needs_a_target_directory,
    );
}

fn bare_repository_needs_a_target_directory() {
    for with_target in [false, true] {
        let tag = if with_target { "target" } else { "none" };
        let (c_root, c_target) = bare_root(&format!("{tag}-c"));
        let mut c = ffi::git_checkout_options::default();
        let version = ffi::GIT_CHECKOUT_OPTIONS_VERSION;
        // SAFETY: `c` is writable storage for the struct; version 1 is accepted.
        let rc = unsafe { ffi::git_checkout_options_init(&mut c, version) };
        assert_eq!(rc, 0);
        c.checkout_strategy = GIT_CHECKOUT_FORCE;
        if with_target {
            c.target_directory = c_target.as_ptr();
        }
        let expected = bare_checkout(&c_root, &c);

        let (rust_root, rust_target) = bare_root(&format!("{tag}-rust"));
        let mut rust = CheckoutOptions::new();
        rust.as_mut().set_strategy(GIT_CHECKOUT_FORCE);
        if with_target {
            rust.as_mut().set_target_directory(Some(&rust_target));
        }
        let actual = bare_checkout(&rust_root, c_ptr(&rust));
        assert_eq!(actual, expected, "{tag}");
        if with_target {
            assert_eq!(actual.0, 0);
            assert!(actual.2.contains(&PathBuf::from("out/x.txt")), "{actual:?}");
        } else {
            assert!(actual.0 < 0, "a bare repository has no working directory");
        }
    }
}

/// The process's file-creation mask.
fn umask() -> u32 {
    // SAFETY: `umask` cannot fail; the mask is restored at once.
    unsafe {
        let mask = libc::umask(0o022);
        libc::umask(mask);
        mask as u32
    }
}
