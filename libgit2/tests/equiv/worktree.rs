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
