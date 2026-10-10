//! Equivalence of `git2/worktree.h`'s wrappers with libgit2.

/// [`PruneFlag`], [`PruneFlags`] and [`PruneOptions`] against `git_worktree_prune_t`,
/// `git_worktree_prune_options_init` and the routines that read the options.
///
/// [`PruneFlag`] must name exactly the `git_worktree_prune_t` constants, each with its
/// C value. [`PruneOptions::new`] must build what `git_worktree_prune_options_init`
/// writes for [`PruneOptions::VERSION`], which is the header's
/// `GIT_WORKTREE_PRUNE_OPTIONS_INIT`; C refuses every other version, and Rust offers
/// no constructor for one.
///
/// The options' observable behaviour is what `git_worktree_is_prunable` and
/// `git_worktree_prune` do with them. Each scenario runs twice, on two independently
/// owned repositories whose worktrees are laid out by hand (a gitdir under
/// `.git/worktrees/<name>`, a working directory with a `.git` link, a `locked` file):
/// one side builds every step's options through C (`git_worktree_prune_options_init`,
/// then raw field writes with the C constants), the other through the Rust API
/// ([`PruneOptions::new`], the handle setters, [`PruneFlags`]), passing C the
/// converted value. After each step the two must agree on the return, the thread's
/// last error (its paths relative to the side's root), and which gitdirs and working
/// directories exist.
mod prune_options {
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::repository::worktree::{
        PruneFlag, PruneFlags, PruneOptions, UnnamedPruneFlag,
    };

    /// Every `git_worktree_prune_t` constant and the variant carrying it.
    const FLAGS: [(ffi::git_worktree_prune_t, PruneFlag); 3] = [
        (ffi::GIT_WORKTREE_PRUNE_VALID, PruneFlag::Valid),
        (ffi::GIT_WORKTREE_PRUNE_LOCKED, PruneFlag::Locked),
        (ffi::GIT_WORKTREE_PRUNE_WORKING_TREE, PruneFlag::WorkingTree),
    ];

    /// Every value up to past the highest flag bit, each single bit and its
    /// neighbours, and the extremes.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=64)
            .chain((0..32).flat_map(|k| {
                let bit = 1u32 << k;
                [bit - 1, bit, bit + 1]
            }))
            .chain([u32::MAX - 1, u32::MAX])
    }

    /// Initialises libgit2, whose error state the consumers write; never released.
    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// The calling thread's last error, as C reports it.
    fn c_last() -> (c_int, String) {
        // SAFETY: the runtime is initialised; `git_error_last` never returns NULL, and
        // its message is NULL or a C string, live until the next libgit2 call on this
        // thread, after the copy.
        unsafe {
            let e = ffi::git_error_last();
            let message = (*e).message;
            let message = if message.is_null() {
                String::new()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            };
            ((*e).klass, message)
        }
    }

    /// `git_worktree_prune_options_init(version)` over a struct filled with a sentinel
    /// pattern, and what it left there.
    fn c_init(version: u32) -> (c_int, ffi::git_worktree_prune_options) {
        let mut raw = ffi::git_worktree_prune_options {
            version: 0xdead_beef,
            flags: 0xdead_beef,
        };
        // SAFETY: `raw` is a live struct the call may write; any version is accepted
        // as input (a wrong one is refused with an error).
        let rc = unsafe { ffi::git_worktree_prune_options_init(&mut raw, version) };
        (rc, raw)
    }

    #[test]
    fn flag_from_raw_names_exactly_the_c_constants() {
        for (c, flag) in FLAGS {
            assert_eq!(flag.raw(), c, "{flag:?}");
            assert_eq!(ffi::git_worktree_prune_t::from(flag), c, "{flag:?}");
            assert_eq!(PruneFlags::from(flag).bits(), c, "{flag:?}");
        }
        assert_eq!(PruneFlag::ALL.map(PruneFlag::raw), FLAGS.map(|(c, _)| c));
        for raw in probes() {
            let c = FLAGS.iter().find(|(c, _)| *c == raw).map(|&(_, f)| f);
            assert_eq!(PruneFlag::from_raw(raw), c, "{raw}");
            assert_eq!(
                PruneFlag::try_from(raw).map_err(UnnamedPruneFlag::raw),
                c.ok_or(raw),
                "{raw}"
            );
            // A set reads each bit as C's `flags & GIT_WORKTREE_PRUNE_*` does.
            let set = PruneFlags::from_bits_retain(raw);
            for (c, flag) in FLAGS {
                assert_eq!(set.contains(flag), raw & c != 0, "{raw:#x} {flag:?}");
            }
            let named = FLAGS.iter().fold(0, |b, &(c, _)| b | c);
            assert_eq!(set.unnamed_bits(), raw & !named, "{raw:#x}");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_git_worktree_prune_options_init() {
        assert_eq!(
            PruneOptions::VERSION,
            ffi::GIT_WORKTREE_PRUNE_OPTIONS_VERSION
        );
        // Repeated: the routine keeps no state, and overwrites whatever was there.
        for _ in 0..4 {
            let (rc, c) = c_init(ffi::GIT_WORKTREE_PRUNE_OPTIONS_VERSION);
            assert_eq!(rc, 0);
            let rust = ffi::git_worktree_prune_options::from(PruneOptions::new());
            assert_eq!((rust.version, rust.flags), (c.version, c.flags));
            // `GIT_WORKTREE_PRUNE_OPTIONS_INIT`: `{VERSION, 0}`.
            assert_eq!((c.version, c.flags), (PruneOptions::VERSION, 0));
            let r = PruneOptions::default();
            assert_eq!(r.as_ref().version(), c.version);
            assert_eq!(r.as_ref().flags().bits(), c.flags);
            assert!(r.as_ref().flags().is_empty());
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn git_worktree_prune_options_init_refuses_every_other_version() {
        // The versions Rust offers no constructor for: C refuses each, leaving the
        // struct as it was.
        init();
        for version in [0, 2, 3, u32::MAX] {
            let (rc, c) = c_init(version);
            assert_eq!(rc, -1, "{version}");
            assert_eq!(
                (c.version, c.flags),
                (0xdead_beef, 0xdead_beef),
                "{version}"
            );
            assert_eq!(
                c_last(),
                (
                    ffi::GIT_ERROR_INVALID as c_int,
                    format!(
                        "invalid version {} on git_worktree_prune_options",
                        version as c_int
                    )
                ),
            );
        }
    }

    /// The options of one step: a version other than the default, named flags and
    /// unnamed bits.
    #[derive(Clone, Copy, Debug)]
    struct Spec {
        version: Option<u32>,
        flags: &'static [PruneFlag],
        unnamed: u32,
    }

    const fn flags(flags: &'static [PruneFlag]) -> Spec {
        Spec {
            version: None,
            flags,
            unnamed: 0,
        }
    }

    const DEFAULT: Spec = flags(&[]);

    /// The step's options, built through C and its constants.
    fn c_options(spec: Spec) -> ffi::git_worktree_prune_options {
        let (rc, mut raw) = c_init(ffi::GIT_WORKTREE_PRUNE_OPTIONS_VERSION);
        assert_eq!(rc, 0);
        for flag in spec.flags {
            let c = FLAGS.iter().find(|(_, f)| f == flag).unwrap().0;
            raw.flags |= c;
        }
        raw.flags |= spec.unnamed;
        if let Some(version) = spec.version {
            raw.version = version;
        }
        raw
    }

    /// The step's options, built through the Rust API.
    fn rust_options(spec: Spec) -> ffi::git_worktree_prune_options {
        let mut opts = PruneOptions::new();
        let mut m = opts.as_mut();
        let mut set: PruneFlags = spec.flags.iter().copied().collect();
        set |= PruneFlags::from_bits_retain(spec.unnamed);
        m.set_flags(set);
        if let Some(version) = spec.version {
            m.set_version(version);
        }
        assert_eq!(opts.as_ref().flags(), set);
        ffi::git_worktree_prune_options::from(opts)
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// Lay out a valid worktree `name` by hand.
        Add(&'static str),
        /// Write `name`'s `locked` file with the reason.
        Lock(&'static str, &'static str),
        /// Make `name`'s `locked` entry a directory, which the lock check cannot read.
        LockUnreadable(&'static str),
        /// Point `name`'s `commondir` at the side's root: the worktree stays valid, but
        /// has no gitdir under that common directory.
        MoveCommondir(&'static str),
        /// Delete `name`'s working directory, which invalidates it.
        RemoveWorkdir(&'static str),
        IsPrunable(&'static str, Spec),
        Prune(&'static str, Spec),
    }

    /// One step's record: its return (or the failed lookup's), the thread's last
    /// error, and each worktree's gitdir and working directory existence.
    type Outcome = (c_int, (c_int, String), Vec<(&'static str, bool, bool)>);

    const NAMES: [&str; 5] = ["wt1", "wt2", "wt3", "wt4", "wt5"];

    /// A repository with a working directory, and its worktrees beside it.
    struct Side {
        root: PathBuf,
        repo: *mut ffi::git_repository,
        build: fn(Spec) -> ffi::git_worktree_prune_options,
    }

    impl Side {
        fn new(tag: &str, build: fn(Spec) -> ffi::git_worktree_prune_options) -> Self {
            let root = std::env::temp_dir().join(format!(
                "crustify-worktree-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            let path = CString::new(root.join("repo").as_os_str().as_bytes()).unwrap();
            let mut opts = ffi::git_repository_init_options::default();
            // SAFETY: `opts` is a live options struct, filled with the version's
            // defaults.
            let rc = unsafe {
                ffi::git_repository_init_options_init(
                    &mut opts,
                    ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                )
            };
            assert_eq!(rc, 0);
            opts.flags = ffi::GIT_REPOSITORY_INIT_MKPATH;
            let mut repo = ptr::null_mut();
            // SAFETY: `repo` is a live slot, `path` NUL-terminated and `opts` a live
            // options struct, all read for the call only; the result is a fresh
            // repository, freed by `drop`.
            let rc = unsafe { ffi::git_repository_init_ext(&mut repo, path.as_ptr(), &mut opts) };
            assert_eq!(rc, 0, "{:?}", c_last());
            Self { root, repo, build }
        }

        fn gitdir(&self, name: &str) -> PathBuf {
            self.root.join("repo/.git/worktrees").join(name)
        }

        fn workdir(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }

        fn add(&self, name: &str) {
            let gitdir = self.gitdir(name);
            let workdir = self.workdir(name);
            fs::create_dir_all(&gitdir).unwrap();
            fs::create_dir_all(&workdir).unwrap();
            fs::write(gitdir.join("commondir"), "../..\n").unwrap();
            let link = workdir.join(".git");
            fs::write(gitdir.join("gitdir"), format!("{}\n", link.display())).unwrap();
            fs::write(gitdir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(&link, format!("gitdir: {}\n", gitdir.display())).unwrap();
        }

        /// Looks `name` up, runs `call` on it and frees it; the lookup's return if it
        /// fails.
        fn with_worktree(
            &self,
            name: &str,
            call: impl FnOnce(*mut ffi::git_worktree) -> c_int,
        ) -> c_int {
            let c_name = CString::new(name).unwrap();
            let mut wt = ptr::null_mut();
            // SAFETY: `wt` is a live slot, `repo` the live repository and `c_name`
            // NUL-terminated; the result is NULL or a fresh worktree.
            let rc = unsafe { ffi::git_worktree_lookup(&mut wt, self.repo, c_name.as_ptr()) };
            if rc != 0 {
                return rc;
            }
            let rc = call(wt);
            // SAFETY: `wt` is the live worktree the lookup produced, released once.
            unsafe { ffi::git_worktree_free(wt) };
            rc
        }

        fn run(&self, step: Step) -> Outcome {
            // SAFETY: no arguments; clears this thread's error state.
            unsafe { ffi::git_error_clear() };
            let rc = match step {
                Step::Add(name) => {
                    self.add(name);
                    0
                }
                Step::Lock(name, reason) => {
                    fs::write(self.gitdir(name).join("locked"), reason).unwrap();
                    0
                }
                Step::LockUnreadable(name) => {
                    fs::create_dir(self.gitdir(name).join("locked")).unwrap();
                    0
                }
                Step::MoveCommondir(name) => {
                    fs::write(self.gitdir(name).join("commondir"), "../../../..\n").unwrap();
                    0
                }
                Step::RemoveWorkdir(name) => {
                    fs::remove_dir_all(self.workdir(name)).unwrap();
                    0
                }
                Step::IsPrunable(name, spec) => {
                    let mut opts = (self.build)(spec);
                    // SAFETY: `wt` is a live worktree and `opts` a live options struct,
                    // which the call only reads.
                    self.with_worktree(name, |wt| unsafe {
                        ffi::git_worktree_is_prunable(wt, &mut opts)
                    })
                }
                Step::Prune(name, spec) => {
                    let mut opts = (self.build)(spec);
                    // SAFETY: as above; the call deletes directories of the side's
                    // own root only.
                    self.with_worktree(name, |wt| unsafe { ffi::git_worktree_prune(wt, &mut opts) })
                }
            };
            let (klass, message) = c_last();
            let root = self.root.display().to_string();
            let state = NAMES
                .iter()
                .map(|&n| (n, self.gitdir(n).exists(), self.workdir(n).exists()))
                .collect();
            (rc, (klass, message.replace(&root, "<root>")), state)
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            // SAFETY: the repository `new` produced, released once.
            unsafe { ffi::git_repository_free(self.repo) };
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Runs `steps` on a C side and a Rust side, comparing after every step; returns
    /// the C side's outcomes.
    fn compare(tag: &str, steps: &[Step]) -> Vec<Outcome> {
        init();
        let c = Side::new(&format!("{tag}-c"), c_options);
        let rust = Side::new(&format!("{tag}-rust"), rust_options);
        steps
            .iter()
            .map(|&step| {
                let expected = c.run(step);
                assert_eq!(rust.run(step), expected, "{step:?}");
                expected
            })
            .collect()
    }

    const VALID: &[PruneFlag] = &[PruneFlag::Valid];
    const VALID_LOCKED: &[PruneFlag] = &[PruneFlag::Valid, PruneFlag::Locked];

    /// `spec` with `version` instead of the default.
    const fn versioned(version: u32, spec: Spec) -> Spec {
        Spec {
            version: Some(version),
            ..spec
        }
    }

    /// `spec` with the `unnamed` bits added.
    const fn unnamed(unnamed: u32, spec: Spec) -> Spec {
        Spec { unnamed, ..spec }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn is_prunable_reads_the_options_alike() {
        let steps = [
            Step::Add("wt1"),
            Step::IsPrunable("wt1", DEFAULT),
            Step::IsPrunable("wt1", flags(VALID)),
            Step::IsPrunable("wt1", flags(&[PruneFlag::WorkingTree])),
            Step::Lock("wt1", "in use"),
            Step::IsPrunable("wt1", flags(VALID)),
            Step::IsPrunable("wt1", flags(VALID_LOCKED)),
            Step::IsPrunable("wt1", unnamed(1 << 3 | 1 << 31, flags(VALID_LOCKED))),
            Step::IsPrunable("wt1", unnamed(!0b111, DEFAULT)),
            Step::IsPrunable("wt1", versioned(0, flags(VALID_LOCKED))),
            Step::IsPrunable("wt1", versioned(2, flags(VALID_LOCKED))),
            Step::IsPrunable("wt1", versioned(u32::MAX, DEFAULT)),
            // Invalid, then locked with no reason.
            Step::Add("wt2"),
            Step::RemoveWorkdir("wt2"),
            Step::IsPrunable("wt2", DEFAULT),
            Step::Lock("wt2", ""),
            Step::IsPrunable("wt2", DEFAULT),
            Step::IsPrunable("wt2", flags(&[PruneFlag::Locked])),
            // Without `Locked` the lock is read, and reading it fails.
            Step::Add("wt3"),
            Step::LockUnreadable("wt3"),
            Step::IsPrunable("wt3", flags(VALID)),
            Step::IsPrunable("wt3", flags(VALID_LOCKED)),
            // With `Valid` the gitdir is looked for under the common directory.
            Step::Add("wt4"),
            Step::MoveCommondir("wt4"),
            Step::IsPrunable("wt4", DEFAULT),
            Step::IsPrunable("wt4", flags(VALID)),
        ];
        let outcomes = compare("is-prunable", &steps);
        let rc: Vec<_> = outcomes.iter().map(|o| o.0).collect();
        let expected = [
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            1,
            0,
            -1,
            -1,
            -1, // wt1
            0,
            0,
            1,
            0,
            0,
            1, // wt2
            0,
            0,
            ffi::GIT_ENOTFOUND,
            1, // wt3
            0,
            0,
            0,
            0, // wt4
        ];
        assert_eq!(rc, expected);
        let message = |i: usize| outcomes[i].1.1.as_str();
        assert_eq!(message(1), "not pruning valid working tree");
        assert_eq!(message(5), "not pruning locked working tree: 'in use'");
        assert_eq!(
            outcomes[9].1,
            (
                ffi::GIT_ERROR_INVALID as c_int,
                "invalid version 0 on git_worktree_prune_options".to_owned()
            )
        );
        assert_eq!(message(16), "not pruning locked working tree: 'is locked'");
        assert_eq!(message(20), "requested file is a directory");
        assert_eq!(message(24), "not pruning valid working tree");
        assert!(
            message(25).starts_with("worktree gitdir ('<root>")
                && message(25).ends_with("worktrees/wt4') does not exist"),
            "{}",
            message(25)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn prune_reads_the_options_alike() {
        let steps = [
            Step::Add("wt1"),
            Step::Add("wt2"),
            Step::Add("wt3"),
            // Refused: valid, then locked, then a bad version; nothing is removed.
            Step::Prune("wt1", DEFAULT),
            Step::Lock("wt1", "busy"),
            Step::Prune("wt1", flags(VALID)),
            Step::Prune("wt1", versioned(0, flags(VALID_LOCKED))),
            // Only the gitdir goes; the lookup then fails.
            Step::Prune("wt1", flags(VALID_LOCKED)),
            Step::Prune("wt1", flags(VALID_LOCKED)),
            // The working directory goes too.
            Step::Prune("wt2", flags(&[PruneFlag::Valid, PruneFlag::WorkingTree])),
            // Invalid already: no flag needed, and no working directory to delete.
            Step::RemoveWorkdir("wt3"),
            Step::Prune("wt3", unnamed(1 << 30, flags(&[PruneFlag::WorkingTree]))),
            // `git_worktree_prune` reads `git_worktree_is_prunable`'s error as prunable:
            // a lock it cannot read does not stop it, and the removal clears the error.
            Step::Add("wt4"),
            Step::LockUnreadable("wt4"),
            Step::Prune("wt4", flags(VALID)),
            // Not prunable under the moved common directory: nothing is removed.
            Step::Add("wt5"),
            Step::MoveCommondir("wt5"),
            Step::Prune("wt5", flags(&[PruneFlag::Valid, PruneFlag::WorkingTree])),
        ];
        let outcomes = compare("prune", &steps);
        let rc: Vec<_> = outcomes.iter().map(|o| o.0).collect();
        let expected = [
            0,
            0,
            0,
            -1,
            0,
            -1,
            -1,
            0,
            ffi::GIT_ENOTFOUND,
            0,
            0,
            0, // wt1 to wt3
            0,
            0,
            0, // wt4
            0,
            0,
            -1, // wt5
        ];
        assert_eq!(rc, expected);
        // Whether `name`'s gitdir and working directory exist after step `i`.
        let state = |i: usize, name: &str| {
            let &(_, gitdir, workdir) = outcomes[i].2.iter().find(|s| s.0 == name).unwrap();
            (gitdir, workdir)
        };
        for name in ["wt1", "wt2", "wt3"] {
            assert_eq!(state(6, name), (true, true), "{name}");
        }
        assert_eq!(state(7, "wt1"), (false, true));
        assert_eq!(state(7, "wt2"), (true, true));
        assert_eq!(state(9, "wt2"), (false, false));
        assert_eq!(state(9, "wt3"), (true, true));
        assert_eq!(state(11, "wt3"), (false, false));
        assert_eq!(state(14, "wt4"), (false, true));
        assert_eq!(outcomes[14].1, (0, "no error".to_owned()));
        assert_eq!(state(17, "wt5"), (true, true));
        assert!(
            outcomes[17].1.1.ends_with("worktrees/wt5') does not exist"),
            "{:?}",
            outcomes[17].1
        );
    }
}

/// Equivalence of `git2/worktree.h`'s [`WorktreeBox`] with the `git_worktree` C code
/// looks up, reads and releases with `git_worktree_free`.
///
/// Each side lays out its own repository with linked worktrees on disk (as
/// `git_worktree_add` leaves them), at paths of equal length, and runs the same
/// sequence: a worktree from `git_worktree_lookup` and one from
/// `git_worktree_open_from_repository` (over the worktree's own repository), each read
/// through `git_worktree_name` / `git_worktree_path`, read again once every repository
/// it came from is freed, then released; then constructions that fail: a lookup of a
/// missing worktree, and `git_worktree_open_from_repository` over a repository that is
/// no worktree's, over a worktree whose administrative directory lost its `HEAD`, and
/// over one whose common directory is gone. The C side keeps the
/// raw pointer and calls `git_worktree_free`; the Rust side adopts each result into a
/// [`WorktreeBox`] (standing in for the constructors' wrappers) and drops it. After
/// every step both record the return codes, what the worktree reads (paths relative to
/// the side's directory) and the libgit2 allocations the step left live and released,
/// by size; the records are compared step by step. Every test reruns alone in a child
/// process that installs the tracing allocator before `git_libgit2_init`.
mod worktree_box {
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::ptr;
    use std::sync::Mutex;

    use libgit2::ffi;
    use libgit2::libgit2::repository::worktree::WorktreeBox;

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process, so no other test shares libgit2's globals.
    fn isolated(test: &str, body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
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

    // --- the tracing allocator ------------------------------------------------------

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// The live allocations libgit2 made through the tracing allocator, by address, each
    /// with its size and the number of the step that made it; and the sizes released
    /// during the current step.
    struct Trace {
        live: HashMap<usize, (usize, usize)>,
        freed: Vec<usize>,
        step: usize,
    }

    static TRACE: Mutex<Option<Trace>> = Mutex::new(None);

    fn with_trace(f: impl FnOnce(&mut Trace)) {
        if let Some(t) = TRACE.lock().unwrap().as_mut() {
            f(t);
        }
    }

    unsafe extern "C" fn traced_malloc(len: usize, _: *const c_char, _: c_int) -> *mut c_void {
        // SAFETY: plain libc allocation.
        let ptr = unsafe { malloc(len) };
        if !ptr.is_null() {
            with_trace(|t| {
                t.live.insert(ptr as usize, (len, t.step));
            });
        }
        ptr
    }

    unsafe extern "C" fn traced_realloc(
        from: *mut c_void,
        len: usize,
        _: *const c_char,
        _: c_int,
    ) -> *mut c_void {
        // SAFETY: `from` is NULL or came from this allocator, i.e. libc.
        let ptr = unsafe { realloc(from, len) };
        if !ptr.is_null() {
            with_trace(|t| {
                t.live.remove(&(from as usize));
                t.live.insert(ptr as usize, (len, t.step));
            });
        }
        ptr
    }

    unsafe extern "C" fn traced_free(ptr: *mut c_void) {
        if !ptr.is_null() {
            with_trace(|t| {
                if let Some((len, _)) = t.live.remove(&(ptr as usize)) {
                    t.freed.push(len);
                }
            });
        }
        // SAFETY: `ptr` came from libc through this allocator, or is NULL.
        unsafe { free(ptr) }
    }

    /// Installs the tracing allocator, then initialises libgit2 on top of it.
    fn install_tracing_allocator() {
        *TRACE.lock().unwrap() = Some(Trace {
            live: HashMap::new(),
            freed: Vec::new(),
            step: 0,
        });
        let mut allocator = ffi::git_allocator {
            gmalloc: Some(traced_malloc),
            grealloc: Some(traced_realloc),
            gfree: Some(traced_free),
        };
        let option = ffi::GIT_OPT_SET_ALLOCATOR as c_int;
        // SAFETY: `GIT_OPT_SET_ALLOCATOR` takes a `git_allocator *` with every member set,
        // which it copies; nothing is allocated yet, so no storage changes allocator.
        let rc = unsafe { ffi::git_libgit2_opts(option, &raw mut allocator) };
        assert_eq!(rc, 0);
        // SAFETY: no arguments; refcounted, never released in this process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
    }

    // --- the sides ---------------------------------------------------------------------

    /// What one step left behind: its return code, the worktree's name and path (relative
    /// to the side's directory) when it read them, the sizes of the allocations it made
    /// that are still live, and the sizes it released (both sorted).
    #[derive(Debug, PartialEq, Eq)]
    struct Record {
        rc: c_int,
        read: Option<(CString, PathBuf)>,
        kept: Vec<usize>,
        released: Vec<usize>,
    }

    /// One side's directory: a non-bare repository at `<base>/repo` with the worktrees
    /// `alpha` and `beta` at `<base>/wt/<name>`.
    struct Side {
        base: PathBuf,
        records: Vec<Record>,
    }

    impl Side {
        /// `tag` must have the same length on both sides, so their paths do.
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "crustify-worktree-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&base);
            let side = Self {
                base,
                records: Vec::new(),
            };
            let repo = side.init();
            for name in ["alpha", "beta"] {
                let admin = side.base.join("repo/.git/worktrees").join(name);
                let workdir = side.base.join("wt").join(name);
                fs::create_dir_all(&admin).unwrap();
                fs::create_dir_all(&workdir).unwrap();
                fs::write(admin.join("commondir"), "../..\n").unwrap();
                let gitlink = workdir.join(".git");
                fs::write(admin.join("gitdir"), format!("{}\n", gitlink.display())).unwrap();
                fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
                fs::write(gitlink, format!("gitdir: {}\n", admin.display())).unwrap();
            }
            close(repo);
            side
        }

        fn init(&self) -> *mut ffi::git_repository {
            let c_path = CString::new(self.base.join("repo").as_os_str().as_bytes()).unwrap();
            let mut opts = ffi::git_repository_init_options::default();
            // SAFETY: `opts` is a live options struct, filled with the version's defaults.
            let rc = unsafe {
                ffi::git_repository_init_options_init(
                    &mut opts,
                    ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                )
            };
            assert_eq!(rc, 0);
            opts.flags = ffi::GIT_REPOSITORY_INIT_MKPATH;
            let mut repo = ptr::null_mut();
            // SAFETY: a live slot, a NUL-terminated path and live options, read for the call.
            let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
            assert_eq!(rc, 0);
            repo
        }

        /// Opens the repository at `<base>/<rel>`: the main one, or a worktree's.
        fn open(&self, rel: &str) -> *mut ffi::git_repository {
            let c_path = CString::new(self.base.join(rel).as_os_str().as_bytes()).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: a live slot and a NUL-terminated path; no ceiling directories.
            let rc = unsafe {
                ffi::git_repository_open_ext(
                    &mut repo,
                    c_path.as_ptr(),
                    ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                    ptr::null(),
                )
            };
            assert_eq!(rc, 0);
            repo
        }

        /// Removes the worktree `name`'s `HEAD` from its administrative directory, which
        /// then no longer reads as a worktree's.
        fn break_admin(&self, name: &str) {
            fs::remove_file(
                self.base
                    .join("repo/.git/worktrees")
                    .join(name)
                    .join("HEAD"),
            )
            .unwrap();
        }

        /// Removes the main repository, the worktrees' common directory, from disk.
        fn remove_repo(&self) {
            fs::remove_dir_all(self.base.join("repo")).unwrap();
        }

        /// Runs one step under tracing. `read` gives the worktree to read afterwards, as
        /// a raw pointer that stays the side's.
        fn step(&mut self, f: impl FnOnce() -> (c_int, *mut ffi::git_worktree)) {
            with_trace(|t| {
                t.step += 1;
                t.freed.clear();
            });
            let (rc, wt) = f();
            let read = (!wt.is_null()).then(|| {
                // SAFETY: `wt` is the side's live worktree, only read; the strings live as
                // long as it does and are copied out at once.
                let (name, path) = unsafe {
                    (
                        CStr::from_ptr(ffi::git_worktree_name(wt)).to_owned(),
                        CStr::from_ptr(ffi::git_worktree_path(wt)).to_owned(),
                    )
                };
                let path = PathBuf::from(path.to_str().unwrap());
                let rel = path.strip_prefix(&self.base).unwrap().to_path_buf();
                (name, rel)
            });
            let mut record = None;
            with_trace(|t| {
                let mut kept: Vec<usize> = t
                    .live
                    .values()
                    .filter(|(_, step)| *step == t.step)
                    .map(|(len, _)| *len)
                    .collect();
                kept.sort_unstable();
                let mut released = std::mem::take(&mut t.freed);
                released.sort_unstable();
                record = Some(Record {
                    rc,
                    read: read.clone(),
                    kept,
                    released,
                });
            });
            self.records.push(record.unwrap());
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn close(repo: *mut ffi::git_repository) {
        // SAFETY: a repository the side opened, freed once; no worktree borrows it.
        unsafe { ffi::git_repository_free(repo) };
    }

    /// `git_worktree_lookup(name)` on `repo`: the code and the slot.
    fn lookup(repo: *mut ffi::git_repository, name: &CStr) -> (c_int, *mut ffi::git_worktree) {
        let mut out = ptr::null_mut();
        // SAFETY: a live slot, a live repository and a NUL-terminated name, only read.
        let rc = unsafe { ffi::git_worktree_lookup(&mut out, repo, name.as_ptr()) };
        (rc, out)
    }

    /// `git_worktree_open_from_repository(repo)`: the code and the slot, which starts NULL
    /// since libgit2 leaves it alone on failure.
    fn open_from(repo: *mut ffi::git_repository) -> (c_int, *mut ffi::git_worktree) {
        let mut out = ptr::null_mut();
        // SAFETY: a live slot and a live repository, only read.
        let rc = unsafe { ffi::git_worktree_open_from_repository(&mut out, repo) };
        (rc, out)
    }

    /// The C side: raw pointers, released with `git_worktree_free`.
    fn c_side() -> Vec<Record> {
        let mut side = Side::new("c-side");
        let repo = side.open("repo");
        let mut wt = ptr::null_mut();
        side.step(|| {
            let (rc, out) = lookup(repo, c"alpha");
            wt = out;
            (rc, out)
        });
        side.step(|| {
            close(repo);
            (0, wt)
        });
        side.step(|| {
            // SAFETY: the worktree looked up above, freed once.
            unsafe { ffi::git_worktree_free(wt) };
            (0, ptr::null_mut())
        });
        let wt_repo = side.open("wt/beta");
        side.step(|| {
            let (rc, out) = open_from(wt_repo);
            wt = out;
            (rc, out)
        });
        side.step(|| {
            close(wt_repo);
            (0, wt)
        });
        side.step(|| {
            // SAFETY: the worktree opened above, freed once.
            unsafe { ffi::git_worktree_free(wt) };
            (0, ptr::null_mut())
        });
        let repo = side.open("repo");
        side.step(|| lookup(repo, c"gamma"));
        side.step(|| {
            let (rc, out) = open_from(repo);
            assert!(out.is_null());
            (rc, out)
        });
        close(repo);
        let wt_repo = side.open("wt/alpha");
        side.break_admin("alpha");
        side.step(|| open_from(wt_repo));
        side.remove_repo();
        side.step(|| open_from(wt_repo));
        close(wt_repo);
        std::mem::take(&mut side.records)
    }

    /// The Rust side: each result adopted into a `WorktreeBox`, released on drop.
    fn rust_side() -> Vec<Record> {
        let mut side = Side::new("r-side");
        let repo = side.open("repo");
        let mut wt: Option<WorktreeBox> = None;
        // The raw pointer only for reading, while the box owns the worktree.
        let raw =
            |wt: &Option<WorktreeBox>| wt.as_ref().map_or(ptr::null_mut(), WorktreeBox::as_c_ptr);
        side.step(|| {
            let (rc, out) = lookup(repo, c"alpha");
            // SAFETY: a fresh worktree the caller owns, released by `git_worktree_free`.
            wt = unsafe { WorktreeBox::from_c(out) };
            (rc, raw(&wt))
        });
        side.step(|| {
            close(repo);
            (0, raw(&wt))
        });
        side.step(|| {
            drop(wt.take());
            (0, ptr::null_mut())
        });
        let wt_repo = side.open("wt/beta");
        side.step(|| {
            let (rc, out) = open_from(wt_repo);
            // SAFETY: as above.
            wt = unsafe { WorktreeBox::from_c(out) };
            (rc, raw(&wt))
        });
        side.step(|| {
            close(wt_repo);
            (0, raw(&wt))
        });
        side.step(|| {
            drop(wt.take());
            (0, ptr::null_mut())
        });
        let repo = side.open("repo");
        side.step(|| {
            let (rc, out) = lookup(repo, c"gamma");
            // SAFETY: NULL, or a fresh worktree the caller owns.
            wt = unsafe { WorktreeBox::from_c(out) };
            assert!(wt.is_none());
            (rc, raw(&wt))
        });
        side.step(|| {
            let (rc, out) = open_from(repo);
            // SAFETY: as above.
            wt = unsafe { WorktreeBox::from_c(out) };
            assert!(wt.is_none());
            (rc, raw(&wt))
        });
        close(repo);
        let wt_repo = side.open("wt/alpha");
        side.break_admin("alpha");
        let adopt_failed = || {
            let (rc, out) = open_from(wt_repo);
            // SAFETY: as above.
            let wt = unsafe { WorktreeBox::from_c(out) };
            assert!(wt.is_none());
            (rc, out)
        };
        side.step(adopt_failed);
        side.remove_repo();
        side.step(adopt_failed);
        close(wt_repo);
        std::mem::take(&mut side.records)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopted_worktrees_match_c_lifecycle() {
        isolated(
            "worktree::worktree_box::adopted_worktrees_match_c_lifecycle",
            || {
                install_tracing_allocator();
                let c = c_side();
                let rust = rust_side();
                assert_eq!(c.len(), rust.len());
                for (i, (c, rust)) in c.iter().zip(&rust).enumerate() {
                    assert_eq!(c, rust, "step {i}");
                }
                // The baseline the comparison relies on: each constructor leaves the struct
                // and its six strings live (the parent's path is set for a repository with a
                // working directory, and for a worktree's own repository), freeing the
                // repository releases none of them, and the release frees exactly those.
                let lookup = &c[0];
                assert_eq!(lookup.rc, 0);
                assert_eq!(lookup.kept.len(), 7, "{lookup:?}");
                assert!(lookup.read.as_ref().is_some_and(
                    |(n, p)| n.as_c_str() == c"alpha" && p == &PathBuf::from("wt/alpha")
                ));
                assert_eq!(c[1].read, lookup.read);
                assert_eq!(c[2].released, lookup.kept);
                let opened = &c[3];
                assert_eq!(opened.rc, 0);
                assert_eq!(opened.kept.len(), 7, "{opened:?}");
                assert_eq!(c[5].released, opened.kept);
                assert_eq!(c[6].rc, ffi::GIT_ENOTFOUND);
                assert!(c[6].kept.is_empty());
                // `git_worktree_open_from_repository`'s failures: a repository that is no
                // worktree's, a worktree whose administrative directory is broken, and one
                // whose common directory is gone. None leaves anything to release.
                for (i, step) in c.iter().enumerate().skip(7) {
                    assert!(step.rc < 0, "step {i}: {step:?}");
                    assert!(
                        step.kept.is_empty() && step.read.is_none(),
                        "step {i}: {step:?}"
                    );
                }
            },
        );
    }
}

/// [`AddOptions`] against `git_worktree_add_options_init` and `git_worktree_add`, the one
/// routine reading the options.
///
/// [`AddOptions::new`] must build what `git_worktree_add_options_init` writes for
/// [`AddOptions::VERSION`] (the header's `GIT_WORKTREE_ADD_OPTIONS_INIT`), field by
/// field, the embedded checkout options included; C refuses every other version, and
/// Rust offers no constructor for one.
///
/// The options' observable behaviour is what `git_worktree_add` does with them. A
/// sequence of adds runs twice, on two independently owned non-bare repositories
/// written identically (an empty-tree commit on `main`, its `HEAD`, and further
/// branches and a tag on it): one side builds every step's options through C
/// (`git_worktree_add_options_init`, raw field writes, a raw `git_reference_lookup`),
/// the other through the Rust API ([`AddOptions::new`], the setters, a [`ReferenceBox`]
/// lent with `set_reference`, the checkout options' handle). After each step the two
/// must agree on the options' fields, the return, the thread's last error, and the
/// files the step left (the worktrees' administrative directories, working directories
/// and checkout target, paths relative to each side's root). No step makes libgit2
/// create a branch: that takes a reflog signature at the current time, whose glibc
/// timezone lookup ThreadSanitizer reports against concurrent tests.
mod add_options {
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::refs::refs::ReferenceBox;
    use libgit2::libgit2::repository::worktree::{AddOptions, AddOptionsRef};

    /// Initialises libgit2, whose error state the consumers write; never released.
    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// The calling thread's last error, as C reports it, with `base` replaced by
    /// `<base>`.
    fn c_last(base: &Path) -> (c_int, String) {
        // SAFETY: the runtime is initialised; `git_error_last` never returns NULL, and
        // its message is NULL or a C string, live until the next libgit2 call on this
        // thread, after the copy.
        let (klass, message) = unsafe {
            let e = ffi::git_error_last();
            let message = (*e).message;
            let message = if message.is_null() {
                String::new()
            } else {
                CStr::from_ptr(message).to_string_lossy().into_owned()
            };
            ((*e).klass, message)
        };
        (klass, message.replace(base.to_str().unwrap(), "<base>"))
    }

    /// `git_worktree_add_options_init(version)` over storage filled with `0xa5` bytes:
    /// its return, and the bytes it left.
    fn c_init_bytes(version: u32) -> (c_int, Vec<u8>) {
        let mut raw = MaybeUninit::<ffi::git_worktree_add_options>::uninit();
        let size = size_of::<ffi::git_worktree_add_options>();
        // SAFETY: `raw` is writable storage of `size` bytes.
        unsafe { ptr::write_bytes(raw.as_mut_ptr().cast::<u8>(), 0xa5, size) };
        // SAFETY: `raw` is writable storage for the struct; a refused version leaves it
        // untouched.
        let rc = unsafe { ffi::git_worktree_add_options_init(raw.as_mut_ptr(), version) };
        // SAFETY: every byte of the storage is initialised (the fill, then possibly the
        // template), and is read as bytes.
        let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), size) };
        (rc, bytes.to_vec())
    }

    /// `git_worktree_add_options_init(GIT_WORKTREE_ADD_OPTIONS_VERSION)`.
    fn c_init() -> ffi::git_worktree_add_options {
        let mut raw = ffi::git_worktree_add_options::default();
        // SAFETY: `raw` is a live struct the call overwrites.
        let rc = unsafe {
            ffi::git_worktree_add_options_init(&mut raw, ffi::GIT_WORKTREE_ADD_OPTIONS_VERSION)
        };
        assert_eq!(rc, 0);
        raw
    }

    /// The fields of the options and of their checkout options that the Rust API reads,
    /// as C stores them: version, lock, checkout_existing, the reference's name, then
    /// the checkout options' version, strategy, modes, open flags, notify flags and
    /// target directory.
    type Fields = (
        u32,
        c_int,
        c_int,
        Option<CString>,
        (u32, u32, u32, u32, c_int, u32, Option<CString>),
    );

    fn name_of(reference: *const ffi::git_reference) -> Option<CString> {
        // SAFETY: NULL or a live reference; its name lives as long as it and is copied.
        (!reference.is_null())
            .then(|| unsafe { CStr::from_ptr(ffi::git_reference_name(reference)) }.to_owned())
    }

    fn c_fields(raw: &ffi::git_worktree_add_options) -> Fields {
        let c = &raw.checkout_options;
        let target = c.target_directory;
        (
            raw.version,
            raw.lock,
            raw.checkout_existing,
            name_of(raw.ref_),
            (
                c.version,
                c.checkout_strategy,
                c.dir_mode,
                c.file_mode,
                c.file_open_flags,
                c.notify_flags,
                // SAFETY: NULL or a C string the side keeps alive.
                (!target.is_null()).then(|| unsafe { CStr::from_ptr(target) }.to_owned()),
            ),
        )
    }

    /// The Rust getters' view; the reference wrappers have no name getter yet, so a set
    /// reference reads as the name of `lent`, the reference the test lent (whose
    /// identity with the getter's result the unit tests check).
    fn rust_fields(o: AddOptionsRef<'_, '_>, lent: Option<&ReferenceBox<'_>>) -> Fields {
        let c = o.checkout_options();
        let reference = o.reference().map(|_| lent.unwrap().as_c_ptr().cast_const());
        (
            o.version(),
            c_int::from(o.lock()),
            c_int::from(o.checkout_existing()),
            reference.and_then(name_of),
            (
                c.version(),
                c.strategy(),
                c.dir_mode(),
                c.file_mode(),
                c.file_open_flags(),
                c.notify_flags(),
                c.target_directory().map(CStr::to_owned),
            ),
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_git_worktree_add_options_init() {
        assert_eq!(AddOptions::VERSION, ffi::GIT_WORKTREE_ADD_OPTIONS_VERSION);
        // Repeated: the routine keeps no state, and overwrites whatever was there.
        for _ in 0..4 {
            let c = c_init();
            let rust = AddOptions::new();
            assert_eq!(rust_fields(rust.as_ref(), None), c_fields(&c));
            assert_eq!(
                rust_fields(AddOptions::default().as_ref(), None),
                c_fields(&c)
            );
            // `GIT_WORKTREE_ADD_OPTIONS_INIT`: `{VERSION, 0, 0, NULL, CHECKOUT_INIT}`.
            assert_eq!(
                c_fields(&c),
                (
                    1,
                    0,
                    0,
                    None,
                    (ffi::GIT_CHECKOUT_OPTIONS_VERSION, 0, 0, 0, 0, 0, None)
                )
            );
            assert!(c.checkout_options.paths.strings.is_null());
            assert!(c.checkout_options.baseline.is_null());
            assert!(c.checkout_options.notify_cb.is_none());
            assert_eq!(c_init_bytes(AddOptions::VERSION).0, 0);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn git_worktree_add_options_init_refuses_every_other_version() {
        // The versions Rust offers no constructor for: C refuses each, leaving the
        // struct as it was.
        init();
        let untouched = vec![0xa5; size_of::<ffi::git_worktree_add_options>()];
        for version in [0, 2, 3, u32::MAX] {
            assert_eq!(c_init_bytes(version), (-1, untouched.clone()), "{version}");
            assert_eq!(
                c_last(Path::new("/nonexistent")),
                (
                    ffi::GIT_ERROR_INVALID as c_int,
                    format!(
                        "invalid version {} on git_worktree_add_options",
                        version as c_int
                    )
                ),
            );
        }
    }

    /// The options of one add.
    #[derive(Clone, Copy, Debug, Default)]
    struct Spec {
        lock: bool,
        checkout_existing: bool,
        /// The reference to look up and lend.
        reference: Option<&'static CStr>,
        /// The checkout's target directory, relative to the side's root.
        target: Option<&'static str>,
        /// The checkout strategy bits.
        strategy: u32,
    }

    /// `git_worktree_add`'s steps: the worktree's name and the options.
    const STEPS: [(&str, Spec); 11] = [
        // An existing branch named after the worktree.
        (
            "alpha",
            Spec {
                checkout_existing: true,
                ..SPEC
            },
        ),
        // A borrowed branch, locked.
        (
            "beta",
            Spec {
                lock: true,
                reference: Some(c"refs/heads/feature"),
                ..SPEC
            },
        ),
        // The same branch, now checked out.
        (
            "gamma",
            Spec {
                reference: Some(c"refs/heads/feature"),
                ..SPEC
            },
        ),
        // Not a branch.
        (
            "delta",
            Spec {
                lock: true,
                reference: Some(c"refs/tags/v1"),
                ..SPEC
            },
        ),
        // The worktree exists: its existing branch is checked out there.
        (
            "alpha",
            Spec {
                checkout_existing: true,
                ..SPEC
            },
        ),
        // A checkout into a target directory, which it creates.
        (
            "epsilon",
            Spec {
                reference: Some(c"refs/heads/topic"),
                target: Some("target"),
                ..SPEC
            },
        ),
        // The repository's own `HEAD`.
        (
            "zeta",
            Spec {
                reference: Some(c"refs/heads/main"),
                ..SPEC
            },
        ),
        // The reference wins over `checkout_existing`; a dry-run checkout
        // (`GIT_CHECKOUT_DRY_RUN`).
        (
            "eta",
            Spec {
                lock: true,
                checkout_existing: true,
                reference: Some(c"refs/heads/spare"),
                strategy: 1 << 24,
                ..SPEC
            },
        ),
        // An administrative directory left by an earlier worktree of that name.
        (
            "beta",
            Spec {
                reference: Some(c"refs/heads/spare2"),
                ..SPEC
            },
        ),
        // The embedded checkout options fail the checkout (a target directory that
        // is a file), after the worktree's files are on disk.
        (
            "theta",
            Spec {
                checkout_existing: true,
                target: Some("blocked"),
                ..SPEC
            },
        ),
        // What that left behind holds the branch checked out, so the same worktree
        // with corrected options is refused.
        (
            "theta",
            Spec {
                checkout_existing: true,
                ..SPEC
            },
        ),
    ];

    const SPEC: Spec = Spec {
        lock: false,
        checkout_existing: false,
        reference: None,
        target: None,
        strategy: 0,
    };

    /// What one step left: its return, the last error, and the files under the side's
    /// root but the repository's own `.git` contents outside `worktrees/` (paths and
    /// contents relative to the root; index and log files, which hold times, skipped).
    type Outcome = (
        c_int,
        Option<(c_int, String)>,
        Vec<(String, Option<String>)>,
    );

    /// One side's directory: a non-bare repository at `<base>/repo`, its worktrees at
    /// `<base>/wt/<name>`.
    struct Side {
        base: PathBuf,
        repo: *mut ffi::git_repository,
        /// The checkout target directories the C side's options point at.
        targets: Vec<CString>,
    }

    impl Side {
        /// `tag` must have the same length on both sides, so their paths do.
        fn new(tag: &str) -> Self {
            init();
            let base = std::env::temp_dir().join(format!(
                "crustify-worktree-add-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&base);
            let path = base.join("repo");
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mut repo = ptr::null_mut();
            // SAFETY: a live slot and a NUL-terminated path, read for the call.
            let rc = unsafe { ffi::git_repository_init(&mut repo, c_path.as_ptr(), 0) };
            assert_eq!(rc, 0);
            let mut odb = ptr::null_mut();
            // SAFETY: a live slot and the live repository; released below.
            assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, repo) }, 0);
            let write = |data: &[u8], kind: ffi::git_object_t| {
                let mut id = ffi::git_oid::default();
                let (bytes, len) = (data.as_ptr().cast(), data.len());
                // SAFETY: a live slot, the live odb and `data` readable for its length at
                // a non-NULL address.
                let rc = unsafe { ffi::git_odb_write(&mut id, odb, bytes, len, kind) };
                assert_eq!(rc, 0);
                let mut hex = [0 as std::ffi::c_char; 65];
                // SAFETY: `hex` bounds the write; `id` is live.
                unsafe { ffi::git_oid_tostr(hex.as_mut_ptr(), hex.len(), &id) };
                // SAFETY: `git_oid_tostr` NUL-terminates within `hex`.
                unsafe { CStr::from_ptr(hex.as_ptr()) }
                    .to_str()
                    .unwrap()
                    .to_owned()
            };
            const WHO: &str = "A U Thor <author@example.com> 1700000000 +0000";
            let tree = write(b"", ffi::GIT_OBJECT_TREE);
            let commit = format!("tree {tree}\nauthor {WHO}\ncommitter {WHO}\n\nfirst\n");
            let commit = write(commit.as_bytes(), ffi::GIT_OBJECT_COMMIT);
            // SAFETY: the odb reference taken above, released once.
            unsafe { ffi::git_odb_free(odb) };
            for name in [
                "heads/main",
                "heads/alpha",
                "heads/feature",
                "heads/topic",
                "heads/spare",
                "heads/spare2",
                "heads/theta",
                "tags/v1",
            ] {
                fs::write(path.join(".git/refs").join(name), format!("{commit}\n")).unwrap();
            }
            fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::create_dir(base.join("wt")).unwrap();
            // A checkout target directory that cannot be one.
            fs::write(base.join("blocked"), "not a directory\n").unwrap();
            Self {
                base,
                repo,
                targets: Vec::new(),
            }
        }

        fn c_path(&self, rel: &str) -> CString {
            CString::new(self.base.join(rel).as_os_str().as_bytes()).unwrap()
        }

        /// `git_reference_lookup(name)`, raw.
        fn lookup(&self, name: &CStr) -> *mut ffi::git_reference {
            let mut out = ptr::null_mut();
            // SAFETY: a live slot, the live repository and a NUL-terminated name.
            let rc = unsafe { ffi::git_reference_lookup(&mut out, self.repo, name.as_ptr()) };
            assert_eq!(rc, 0, "{name:?}");
            out
        }

        /// `git_worktree_add(name, <base>/wt/<name>, opts)` and what it left.
        fn add(&self, name: &str, opts: *const ffi::git_worktree_add_options) -> Outcome {
            let c_name = CString::new(name).unwrap();
            let dir = self.c_path(&format!("wt/{name}"));
            let mut out = ptr::null_mut();
            // SAFETY: a live slot, the live repository, two NUL-terminated strings and
            // live options whose reference and strings the caller keeps alive.
            let rc = unsafe {
                ffi::git_worktree_add(&mut out, self.repo, c_name.as_ptr(), dir.as_ptr(), opts)
            };
            let error = (rc < 0).then(|| c_last(&self.base));
            assert_eq!(out.is_null(), rc < 0);
            // SAFETY: NULL or the fresh worktree the caller owns, freed once.
            unsafe { ffi::git_worktree_free(out) };
            (rc, error, self.snapshot())
        }

        fn snapshot(&self) -> Vec<(String, Option<String>)> {
            fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Option<String>)>) {
                let Ok(entries) = fs::read_dir(dir) else {
                    return;
                };
                let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
                entries.sort();
                for path in entries {
                    let file = path.file_name().unwrap().to_str().unwrap();
                    if file == "index" || file == "logs" {
                        continue;
                    }
                    let rel = path
                        .strip_prefix(base)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned();
                    if path.is_dir() {
                        out.push((rel, None));
                        walk(base, &path, out);
                    } else {
                        let content = fs::read_to_string(&path)
                            .unwrap()
                            .replace(base.to_str().unwrap(), "<base>");
                        out.push((rel, Some(content)));
                    }
                }
            }
            let mut out = Vec::new();
            for dir in ["repo/.git/worktrees", "wt", "target"] {
                walk(&self.base, &self.base.join(dir), &mut out);
            }
            out
        }
    }

    impl Drop for Side {
        fn drop(&mut self) {
            // SAFETY: the repository this side opened, freed once.
            unsafe { ffi::git_repository_free(self.repo) };
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adds_match_c_built_options_step_by_step() {
        let mut c = Side::new("c-side");
        let rust = Side::new("r-side");
        let mut c_log = Vec::new();
        let mut rust_log = Vec::new();
        let mut c_messages = Vec::new();
        for (step, (name, spec)) in STEPS.iter().enumerate() {
            // C: the init routine, then raw writes; the reference stays C's until freed.
            let mut raw = c_init();
            raw.lock = c_int::from(spec.lock);
            raw.checkout_existing = c_int::from(spec.checkout_existing);
            raw.checkout_options.checkout_strategy = spec.strategy;
            let c_ref = spec.reference.map_or(ptr::null_mut(), |n| c.lookup(n));
            raw.ref_ = c_ref;
            if let Some(target) = spec.target {
                c.targets.push(c.c_path(target));
                raw.checkout_options.target_directory = c.targets.last().unwrap().as_ptr();
            }
            let c_outcome = c.add(name, &raw);
            let c_seen = c_fields(&raw);
            // SAFETY: NULL or the reference looked up above, freed once.
            unsafe { ffi::git_reference_free(c_ref) };

            // Rust: the safe constructor and setters, lending an owned reference.
            let reference = spec.reference.map(|n| {
                // SAFETY: a fresh attached reference the test owns, released by
                // `git_reference_free`; the side's repository outlives it.
                unsafe { ReferenceBox::from_c(rust.lookup(n)) }.unwrap()
            });
            let target = spec.target.map(|t| rust.c_path(t));
            let mut opts = AddOptions::new();
            {
                let mut m = opts.as_mut();
                m.set_lock(spec.lock);
                m.set_checkout_existing(spec.checkout_existing);
                m.set_reference(reference.as_ref().map(|r| r.as_ref()));
                let mut checkout = m.checkout_options();
                checkout.set_strategy(spec.strategy);
                checkout.set_target_directory(target.as_deref());
            }
            let rust_outcome = rust.add(name, ptr::from_ref(&opts).cast());
            let mut rust_seen = rust_fields(opts.as_ref(), reference.as_ref());
            drop(reference);

            // The target directories differ by the side's root only.
            let relative = |f: &mut Fields, base: &Path| {
                if let Some(t) = f.4.6.take() {
                    let t = t
                        .to_str()
                        .unwrap()
                        .replace(base.to_str().unwrap(), "<base>");
                    f.4.6 = Some(CString::new(t).unwrap());
                }
            };
            let mut c_seen = c_seen;
            relative(&mut c_seen, &c.base);
            relative(&mut rust_seen, &rust.base);
            assert_eq!(rust_seen, c_seen, "step {step} ({name}): options");
            assert_eq!(rust_outcome, c_outcome, "step {step} ({name})");
            c_log.push(c_outcome.0);
            rust_log.push(rust_outcome.0);
            c_messages.push(c_outcome.1.map(|(_, message)| message));
        }
        // Both sides took every branch the steps aim at.
        let eexists = ffi::GIT_EEXISTS;
        assert_eq!(
            c_log,
            [0, 0, -1, -1, -1, 0, -1, 0, eexists, eexists, -1],
            "{c_messages:#?}"
        );
        assert_eq!(rust_log, c_log);
        // The failed checkout left a worktree on disk that libgit2 finds by name and
        // that holds its branch checked out, though nothing was checked out into it:
        // its administrative directory, `.git` file and `HEAD`.
        assert_eq!(
            c_messages[9].as_deref(),
            Some("failed to make directory '<base>/blocked': directory exists")
        );
        assert_eq!(
            c_messages[10].as_deref(),
            Some("reference refs/heads/theta is already checked out")
        );
        for side in [&c, &rust] {
            assert_eq!(
                fs::read_to_string(side.base.join("repo/.git/worktrees/theta/HEAD")).unwrap(),
                "ref: refs/heads/theta\n"
            );
            assert!(side.base.join("wt/theta/.git").is_file());
            assert!(fs::read_dir(side.base.join("wt/theta")).unwrap().count() == 1);
        }
    }
}

/// The worktree routines against libgit2: `Worktree::lookup` and
/// `open_from_repository`, `list`, and the readers and lock routines on the handles,
/// against `git_worktree_lookup`, `git_worktree_open_from_repository`,
/// `git_worktree_list`, `git_worktree_name` / `_path` / `_validate` / `_is_locked` /
/// `_is_prunable` / `_prune` and `git_worktree_lock` / `_unlock`.
///
/// Each side owns a repository with two linked worktrees laid out by hand, under its own
/// root; both run the same sequence of steps, the C side through the raw routines, the
/// Rust side through the safe API, each holding the worktrees it looked up across the
/// steps. After every step they must agree on the result (the return, or the value the
/// wrapper reads out of it: a name, a path relative to the root, a lock reason, a list
/// of names), on the error (class and message, the root masked; for `is_prunable`'s
/// "not prunable" through the thread's last error), and on which administrative
/// directories, working directories and `locked` files exist.
///
/// Where libgit2 fails without setting an error, the Rust error is the wrapper's own
/// (see the module docs of `worktree`): those steps compare the return code and check
/// that C set none. The corrected behaviours (`list`'s skipped entries, `prune` after a
/// failed check, repositories without paths) have their regressions in the unit tests;
/// `list` is compared here on directories where libgit2 checks every entry (one entry
/// that is not a worktree, or none).
mod routines {
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
    use libgit2::libgit2::repository::worktree::{
        PruneFlag, PruneFlags, PruneOptions, Worktree, WorktreeBox, WorktreeError, list,
    };
    use libgit2::libgit2::util::errors::last;

    const NAMES: [&str; 2] = ["wt1", "wt2"];

    #[derive(Clone, Copy, Debug)]
    enum Step {
        Lookup(&'static str),
        OpenFromMain,
        OpenFromWorktree(&'static str),
        Name(usize),
        Path(usize),
        Validate(usize),
        IsLocked(usize),
        LockReason(usize),
        Lock(usize, Option<&'static CStr>),
        Unlock(usize),
        IsPrunable(usize, u32),
        Prune(usize, u32),
        List,
        AddJunk(&'static str),
        RemoveWorkdir(&'static str),
        LockDir(&'static str),
        /// Removes the worktree's administrative directory.
        RemoveAdmin(&'static str),
        /// Points the worktree's `commondir` file at a directory that does not exist.
        PointCommondir(&'static str),
    }

    /// What a step returned: the code, the value read out of it, and the thread's error
    /// (`None` for `GIT_ERROR_NONE`), the root masked.
    #[derive(Debug, PartialEq)]
    struct Record {
        code: c_int,
        value: Option<String>,
        error: Option<(c_int, String)>,
        state: Vec<(&'static str, bool, bool, bool)>,
    }

    /// The thread's last error as C reports it; `None` for `GIT_ERROR_NONE`.
    fn c_last() -> Option<(c_int, String)> {
        // SAFETY: the runtime is initialised; `git_error_last` never returns NULL, and
        // its message is a C string, live until the next libgit2 call on this thread.
        unsafe {
            let e = ffi::git_error_last();
            if (*e).klass == ffi::GIT_ERROR_NONE as c_int {
                return None;
            }
            let message = CStr::from_ptr((*e).message).to_string_lossy().into_owned();
            Some(((*e).klass, message))
        }
    }

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    fn options(flags: u32) -> PruneOptions {
        let mut opts = PruneOptions::new();
        opts.as_mut().set_flags(PruneFlags::from_bits_retain(flags));
        opts
    }

    /// The layout both sides share: a repository and the worktrees in [`NAMES`].
    struct Layout {
        root: PathBuf,
    }

    impl Layout {
        fn new(tag: &str) -> Self {
            init();
            let root = std::env::temp_dir().join(format!(
                "crustify-worktree-routines-equiv-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn repo_path(&self) -> CString {
            CString::new(self.root.join("repo").as_os_str().as_bytes()).unwrap()
        }

        fn admin(&self, name: &str) -> PathBuf {
            self.root.join("repo/.git/worktrees").join(name)
        }

        fn workdir(&self, name: &str) -> PathBuf {
            self.root.join(name)
        }

        fn add(&self, name: &str) {
            let admin = self.admin(name);
            let workdir = self.workdir(name);
            fs::create_dir_all(&admin).unwrap();
            fs::create_dir_all(&workdir).unwrap();
            fs::write(admin.join("commondir"), "../..\n").unwrap();
            let link = workdir.join(".git");
            fs::write(admin.join("gitdir"), format!("{}\n", link.display())).unwrap();
            fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(&link, format!("gitdir: {}\n", admin.display())).unwrap();
        }

        /// The steps that only change the file system; `None` for the others.
        fn touch(&self, step: Step) -> Option<()> {
            match step {
                Step::AddJunk(name) => fs::create_dir_all(self.admin(name)).unwrap(),
                Step::RemoveWorkdir(name) => fs::remove_dir_all(self.workdir(name)).unwrap(),
                Step::LockDir(name) => fs::create_dir(self.admin(name).join("locked")).unwrap(),
                Step::RemoveAdmin(name) => fs::remove_dir_all(self.admin(name)).unwrap(),
                Step::PointCommondir(name) => {
                    let missing = self.root.join("missing-commondir");
                    fs::write(
                        self.admin(name).join("commondir"),
                        format!("{}\n", missing.display()),
                    )
                    .unwrap()
                }
                _ => return None,
            }
            Some(())
        }

        fn mask(&self, s: &str) -> String {
            s.replace(&self.root.display().to_string(), "<root>")
        }

        fn record(
            &self,
            code: c_int,
            value: Option<String>,
            error: Option<(c_int, String)>,
        ) -> Record {
            Record {
                code,
                value: value.map(|v| self.mask(&v)),
                error: error.map(|(k, m)| (k, self.mask(&m))),
                state: NAMES
                    .iter()
                    .map(|&n| {
                        (
                            n,
                            self.admin(n).exists(),
                            self.workdir(n).exists(),
                            self.admin(n).join("locked").exists(),
                        )
                    })
                    .collect(),
            }
        }
    }

    impl Drop for Layout {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Whether `step` returning `code` reports a failure, whose error is compared: a
    /// negative code, or "not prunable". A success may leave an older error in place.
    fn failure(step: Step, code: c_int) -> bool {
        code < 0 || (matches!(step, Step::IsPrunable(..)) && code == 0)
    }

    fn sorted_names(mut names: Vec<String>) -> Option<String> {
        names.sort();
        Some(names.join(","))
    }

    // --- the C side -----------------------------------------------------------------

    struct CSide {
        layout: Layout,
        repo: *mut ffi::git_repository,
        worktrees: Vec<*mut ffi::git_worktree>,
    }

    impl CSide {
        fn new(tag: &str) -> Self {
            let layout = Layout::new(tag);
            let mut repo = ptr::null_mut();
            let path = layout.repo_path();
            // SAFETY: a writable slot and a NUL-terminated path; the result is freed by
            // `drop`.
            let rc = unsafe { ffi::git_repository_init(&mut repo, path.as_ptr(), 0) };
            assert_eq!(rc, 0);
            for n in NAMES {
                layout.add(n);
            }
            Self {
                layout,
                repo,
                worktrees: Vec::new(),
            }
        }

        fn keep(&mut self, rc: c_int, wt: *mut ffi::git_worktree) -> (c_int, Option<String>) {
            if rc != 0 {
                return (rc, None);
            }
            self.worktrees.push(wt);
            // SAFETY: the live worktree just kept; its name is a C string it owns.
            let name = unsafe { CStr::from_ptr(ffi::git_worktree_name(wt)) };
            (rc, Some(name.to_string_lossy().into_owned()))
        }

        fn run(&mut self, step: Step) -> Record {
            // SAFETY: no arguments; clears this thread's error state.
            unsafe { ffi::git_error_clear() };
            if self.layout.touch(step).is_some() {
                return self.layout.record(0, None, None);
            }
            let wt = |i: usize, s: &Self| s.worktrees[i];
            // SAFETY: every call below takes live worktrees and repositories this side
            // owns, live out slots and NUL-terminated strings, read for the call.
            let (code, value) = unsafe {
                match step {
                    Step::Lookup(name) => {
                        let c_name = CString::new(name).unwrap();
                        let mut out = ptr::null_mut();
                        let rc = ffi::git_worktree_lookup(&mut out, self.repo, c_name.as_ptr());
                        self.keep(rc, out)
                    }
                    Step::OpenFromMain => {
                        let mut out = ptr::null_mut();
                        let rc = ffi::git_worktree_open_from_repository(&mut out, self.repo);
                        self.keep(rc, out)
                    }
                    Step::OpenFromWorktree(name) => {
                        let dir =
                            CString::new(self.layout.workdir(name).as_os_str().as_bytes()).unwrap();
                        let mut repo = ptr::null_mut();
                        assert_eq!(ffi::git_repository_open(&mut repo, dir.as_ptr()), 0);
                        let mut out = ptr::null_mut();
                        let rc = ffi::git_worktree_open_from_repository(&mut out, repo);
                        ffi::git_repository_free(repo);
                        self.keep(rc, out)
                    }
                    Step::Name(i) => {
                        let name = CStr::from_ptr(ffi::git_worktree_name(wt(i, self)));
                        (0, Some(name.to_string_lossy().into_owned()))
                    }
                    Step::Path(i) => {
                        let path = CStr::from_ptr(ffi::git_worktree_path(wt(i, self)));
                        (0, Some(path.to_string_lossy().into_owned()))
                    }
                    Step::Validate(i) => (ffi::git_worktree_validate(wt(i, self)), None),
                    Step::IsLocked(i) => (
                        ffi::git_worktree_is_locked(ptr::null_mut(), wt(i, self)),
                        None,
                    ),
                    Step::LockReason(i) => {
                        let mut buf = ffi::git_buf::default();
                        let rc = ffi::git_worktree_is_locked(&mut buf, wt(i, self));
                        let reason = (rc > 0).then(|| {
                            let bytes = std::slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.size);
                            String::from_utf8_lossy(bytes).into_owned()
                        });
                        if buf.reserved > 0 {
                            ffi::git_buf_dispose(&mut buf);
                        }
                        (rc, reason)
                    }
                    Step::Lock(i, reason) => {
                        let reason = reason.map_or(ptr::null(), CStr::as_ptr);
                        (ffi::git_worktree_lock(wt(i, self), reason), None)
                    }
                    Step::Unlock(i) => (ffi::git_worktree_unlock(wt(i, self)), None),
                    Step::IsPrunable(i, flags) => {
                        let mut opts = ffi::git_worktree_prune_options {
                            version: ffi::GIT_WORKTREE_PRUNE_OPTIONS_VERSION,
                            flags,
                        };
                        (ffi::git_worktree_is_prunable(wt(i, self), &mut opts), None)
                    }
                    Step::Prune(i, flags) => {
                        let mut opts = ffi::git_worktree_prune_options {
                            version: ffi::GIT_WORKTREE_PRUNE_OPTIONS_VERSION,
                            flags,
                        };
                        (ffi::git_worktree_prune(wt(i, self), &mut opts), None)
                    }
                    Step::List => {
                        let mut array = ffi::git_strarray::default();
                        let rc = ffi::git_worktree_list(&mut array, self.repo);
                        let names = (0..array.count)
                            .map(|k| {
                                CStr::from_ptr(*array.strings.add(k))
                                    .to_string_lossy()
                                    .into_owned()
                            })
                            .collect();
                        ffi::git_strarray_dispose(&mut array);
                        (rc, sorted_names(names))
                    }
                    Step::AddJunk(_)
                    | Step::RemoveWorkdir(_)
                    | Step::LockDir(_)
                    | Step::RemoveAdmin(_)
                    | Step::PointCommondir(_) => unreachable!(),
                }
            };
            self.layout
                .record(code, value, failure(step, code).then(c_last).flatten())
        }
    }

    impl Drop for CSide {
        fn drop(&mut self) {
            for wt in self.worktrees.drain(..) {
                // SAFETY: each worktree this side looked up, released once.
                unsafe { ffi::git_worktree_free(wt) };
            }
            // SAFETY: the repository `new` made, released once.
            unsafe { ffi::git_repository_free(self.repo) };
        }
    }

    // --- the Rust side --------------------------------------------------------------

    struct RustSide {
        worktrees: Vec<WorktreeBox>,
        repo: RepositoryBox,
        layout: Layout,
    }

    /// A Rust failure as the C side records it: the code, and the error unless the
    /// wrapper supplied it for a routine that set none (`own_error`).
    fn failed(e: WorktreeError) -> (c_int, Option<String>, Option<(c_int, String)>, bool) {
        let error = (e.raw_class(), e.message().to_string_lossy().into_owned());
        (e.raw_code(), None, Some(error), true)
    }

    impl RustSide {
        fn new(tag: &str) -> Self {
            let layout = Layout::new(tag);
            let repo = Repository::init(&layout.repo_path(), false).unwrap();
            for n in NAMES {
                layout.add(n);
            }
            Self {
                worktrees: Vec::new(),
                repo,
                layout,
            }
        }

        fn keep(
            &mut self,
            w: Result<WorktreeBox, WorktreeError>,
        ) -> (c_int, Option<String>, Option<(c_int, String)>, bool) {
            match w {
                Ok(w) => {
                    let name = w.as_ref().name().to_string_lossy().into_owned();
                    self.worktrees.push(w);
                    (0, Some(name), None, false)
                }
                Err(e) => failed(e),
            }
        }

        /// The record, and whether a failure's error is one the wrapper may have set.
        fn run(&mut self, step: Step) -> (Record, bool) {
            if self.layout.touch(step).is_some() {
                return (self.layout.record(0, None, None), false);
            }
            let unit = |r: Result<(), WorktreeError>| match r {
                Ok(()) => (0, None, None, false),
                Err(e) => failed(e),
            };
            let (code, value, error, own) = match step {
                Step::Lookup(name) => {
                    let name = CString::new(name).unwrap();
                    let w = Worktree::lookup(self.repo.as_ref(), &name);
                    self.keep(w)
                }
                Step::OpenFromMain => {
                    let w = Worktree::open_from_repository(self.repo.as_ref());
                    self.keep(w)
                }
                Step::OpenFromWorktree(name) => {
                    let dir =
                        CString::new(self.layout.workdir(name).as_os_str().as_bytes()).unwrap();
                    let repo = Repository::open(&dir).unwrap();
                    let w = Worktree::open_from_repository(repo.as_ref());
                    drop(repo);
                    self.keep(w)
                }
                Step::Name(i) => {
                    let name = self.worktrees[i].as_ref().name();
                    (0, Some(name.to_string_lossy().into_owned()), None, false)
                }
                Step::Path(i) => {
                    let path = self.worktrees[i].as_ref().path();
                    (0, Some(path.to_string_lossy().into_owned()), None, false)
                }
                Step::Validate(i) => unit(self.worktrees[i].as_ref().validate()),
                Step::IsLocked(i) => match self.worktrees[i].as_ref().is_locked() {
                    Ok(locked) => (c_int::from(locked), None, None, false),
                    Err(e) => failed(e),
                },
                Step::LockReason(i) => match self.worktrees[i].as_ref().lock_reason() {
                    Ok(reason) => {
                        let reason = reason
                            .map(|b| String::from_utf8_lossy(b.as_ref().as_bytes()).into_owned());
                        (c_int::from(reason.is_some()), reason, None, false)
                    }
                    Err(e) => failed(e),
                },
                Step::Lock(i, reason) => unit(self.worktrees[i].as_mut().lock(reason)),
                Step::Unlock(i) => match self.worktrees[i].as_mut().unlock() {
                    Ok(was_locked) => (c_int::from(!was_locked), None, None, false),
                    Err(e) => failed(e),
                },
                Step::IsPrunable(i, flags) => {
                    let opts = options(flags);
                    match self.worktrees[i].as_ref().is_prunable(opts.as_ref()) {
                        Ok(true) => (1, None, None, false),
                        // The reason is the thread's last error, as C leaves it.
                        Ok(false) => {
                            let e = last();
                            let error = (e.raw_class(), e.message().to_string_lossy().into_owned());
                            (0, None, Some(error), false)
                        }
                        Err(e) => failed(e),
                    }
                }
                Step::Prune(i, flags) => {
                    let opts = options(flags);
                    unit(self.worktrees[i].as_ref().prune(opts.as_ref()))
                }
                Step::List => match list(self.repo.as_ref()) {
                    Ok(names) => {
                        let names = names
                            .as_ref()
                            .iter()
                            .map(|n| n.to_string_lossy().into_owned())
                            .collect();
                        (0, sorted_names(names), None, false)
                    }
                    Err(e) => failed(e),
                },
                Step::AddJunk(_)
                | Step::RemoveWorkdir(_)
                | Step::LockDir(_)
                | Step::RemoveAdmin(_)
                | Step::PointCommondir(_) => unreachable!(),
            };
            let error = if failure(step, code) { error } else { None };
            (self.layout.record(code, value, error), own)
        }
    }

    /// Runs `steps` on both sides, comparing after every step.
    fn compare(tag: &str, steps: &[Step]) {
        let mut c = CSide::new(&format!("{tag}-c"));
        let mut rust = RustSide::new(&format!("{tag}-rust"));
        for (k, &step) in steps.iter().enumerate() {
            let expected = c.run(step);
            let (mut got, own) = rust.run(step);
            if own && expected.error.is_none() {
                // libgit2 set no error: the wrapper's own stands in for it.
                assert!(got.error.is_some(), "step {k} {step:?}");
                got.error = None;
            }
            assert_eq!(got, expected, "step {k} {step:?}");
        }
    }

    const VALID: u32 = PruneFlag::Valid.raw();
    const LOCKED: u32 = PruneFlag::Locked.raw();
    const WORKING_TREE: u32 = PruneFlag::WorkingTree.raw();

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lookups_and_readers_match_c() {
        use Step::*;
        compare(
            "readers",
            &[
                List,
                Lookup("wt1"),
                Lookup("absent"),
                Lookup(""),
                Name(0),
                Path(0),
                Validate(0),
                OpenFromMain,
                OpenFromWorktree("wt2"),
                Name(1),
                Path(1),
                Validate(1),
                AddJunk("junk"),
                List,
                Lookup("junk"),
                RemoveWorkdir("wt1"),
                Validate(0),
                Path(0),
                List,
            ],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn lock_sequences_match_c() {
        use Step::*;
        compare(
            "locks",
            &[
                Lookup("wt1"),
                Lookup("wt1"),
                IsLocked(0),
                LockReason(0),
                Lock(0, Some(c"moving disks")),
                IsLocked(0),
                IsLocked(1),
                LockReason(1),
                Lock(1, None),
                Lock(0, Some(c"again")),
                Unlock(1),
                Unlock(0),
                Lock(0, None),
                LockReason(0),
                Lock(1, Some(c"")),
                Unlock(1),
                IsLocked(0),
                LockDir("wt2"),
                Lookup("wt2"),
                IsLocked(2),
                LockReason(2),
                Lock(2, None),
            ],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn prune_sequences_match_c() {
        use Step::*;
        compare(
            "prune",
            &[
                Lookup("wt1"),
                Lookup("wt2"),
                IsPrunable(0, 0),
                Prune(0, 0),
                Lock(0, Some(c"kept")),
                IsPrunable(0, VALID),
                Prune(0, VALID),
                IsPrunable(0, VALID | LOCKED),
                Prune(0, VALID | LOCKED),
                Validate(0),
                IsPrunable(0, VALID | LOCKED),
                Prune(0, VALID | LOCKED),
                List,
                Lookup("wt1"),
                RemoveWorkdir("wt2"),
                IsPrunable(1, 0),
                Prune(1, WORKING_TREE),
                List,
                Name(1),
                Lookup("wt2"),
            ],
        );
        compare(
            "prune-tree",
            &[
                Lookup("wt1"),
                Prune(0, VALID | WORKING_TREE),
                Prune(0, VALID | WORKING_TREE),
                Lookup("wt2"),
                Prune(1, VALID | WORKING_TREE | 1 << 9),
                List,
            ],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failures_on_a_changed_disk_match_c() {
        use Step::*;
        compare(
            "changed",
            &[
                Lookup("wt1"),
                RemoveAdmin("wt1"),
                Lock(0, Some(c"gone")),
                Unlock(0),
                IsLocked(0),
                LockReason(0),
                Validate(0),
                IsPrunable(0, VALID),
                Prune(0, VALID),
                PointCommondir("wt2"),
                Lookup("wt2"),
                Validate(1),
                IsPrunable(1, 0),
                IsPrunable(1, VALID),
                Prune(1, VALID),
                List,
            ],
        );
    }
}
