//! Equivalence tests for `git2/refdb.h`.
//!
//! The `git_refdb` owners: [`Refdb::open`] against `git_refdb_open`, and
//! [`Refdb::new`] against `git_refdb_new` followed by `git_refdb_set_backend`, each
//! on its own repository of identical content. After construction, both refdbs run
//! the same sequence of compactions (`git_refdb_compress`, through each owner's
//! pointer) between reference writes made through each repository, and the storage
//! the two leave is compared after every step: the return codes, every reference
//! read back, and the files the backend keeps (`packed-refs` and the loose
//! references; the reftable stack's table count). Dropping the Rust owner and
//! freeing the C refdb must leave both repositories reading the same references.
//!
//! Creating a reference reaches `git_signature_now` for its reflog entry, whose glibc
//! timezone lock `ThreadSanitizer` cannot see, so the tests that write references run
//! alone in a child process.
//!
//! A refused backend fails both alike, with the same error class and message, and
//! stays its caller's: neither frees it (`ASan` would flag the caller's free as a
//! double free).
//!
//! Then the enum `RefdbType` (`git_refdb_t`), in `refdb_type`.

mod owner {
    use std::ffi::{CStr, CString, c_int};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::ptr::{self, NonNull};

    use libgit2::ffi;
    use libgit2::libgit2::refs::refdb::{Refdb, RefdbBox, RefdbType};
    use libgit2::libgit2::repository::repository::{
        InitFlag, InitOptions, Repository, RepositoryBox,
    };
    use libgit2::libgit2::util::errors::ErrorBuf;

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process, so no other test calls libgit2 meanwhile.
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

    fn init() {
        // SAFETY: balanced by nothing: the reference lives for the process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
    }

    fn c_last() -> (c_int, CString) {
        {
            // SAFETY: the runtime is initialised; the error is copied before any other call.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
            let klass = unsafe { (*e).klass };
            // SAFETY: as above.
            let message2 = unsafe { (*e).message };
            // SAFETY: as above.
            unsafe { (klass, CStr::from_ptr(message2).to_owned()) }
        }
    }

    /// What the repository reads back for each name: the lookup's return and the target.
    type Lookups = Vec<(c_int, Option<[u8; 32]>)>;

    /// What a constructor reported: success, or the error class and message.
    type Reported = Result<(), (c_int, CString)>;

    fn c_reported(rc: c_int) -> Reported {
        if rc < 0 { Err(c_last()) } else { Ok(()) }
    }

    fn rust_reported<T>(result: &Result<T, ErrorBuf>) -> Reported {
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err((e.raw_class(), e.message().to_owned())),
        }
    }

    const NAMES: [&CStr; 4] = [
        c"refs/heads/main",
        c"refs/heads/topic",
        c"refs/tags/v1",
        c"refs/heads/missing",
    ];

    /// A fresh bare repository with reference storage `format` and one blob, freed and
    /// removed on drop.
    #[expect(
        clippy::struct_field_names,
        reason = "the fields mirror the names of what they hold"
    )]
    struct Repo {
        path: PathBuf,
        repo: RepositoryBox,
        blob: ffi::git_oid,
    }

    impl Repo {
        fn new(tag: &str, format: ffi::git_refdb_t) -> Self {
            init();
            let path = std::env::temp_dir()
                .join(format!("crustify-refdb-equiv-{}-{tag}", std::process::id()));
            drop(fs::remove_dir_all(&path));
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let mut opts = InitOptions::new();
            let mut m = opts.as_mut();
            m.set_flags(InitFlag::Bare | InitFlag::Mkdir);
            m.set_refdb_type(Some(RefdbType::from_raw(format).unwrap()));
            let repo = Repository::init_ext(&c_path, opts.as_mut())
                .unwrap()
                .into_c();
            let mut odb = ptr::null_mut();
            let mut blob = ffi::git_oid::default();
            let data = b"refdb equivalence\n";
            {
                // SAFETY: live slots and the live repository; the data is read for the call
                // only, and the odb reference is released right after.
                unsafe { assert_eq!(ffi::git_repository_odb(&raw mut odb, repo), 0) };
                // SAFETY: as above.
                let rc = unsafe {
                    ffi::git_odb_write(
                        &raw mut blob,
                        odb,
                        data.as_ptr().cast(),
                        data.len(),
                        ffi::GIT_OBJECT_BLOB,
                    )
                };
                // SAFETY: as above.
                unsafe { ffi::git_odb_free(odb) };
                assert_eq!(rc, 0);
            }
            Self {
                path,
                // SAFETY: the fresh repository the call wrote, owned here alone.
                repo: unsafe { RepositoryBox::from_c(repo) }.unwrap(),
                blob,
            }
        }

        /// Creates `name` over the blob, through the repository's own refdb.
        fn create(&self, name: &CStr) -> c_int {
            let mut out = ptr::null_mut();
            {
                // SAFETY: a live slot, the live repository, a NUL-terminated name and the
                // blob's id, read for the call; the reference is freed right after.
                let rc = unsafe {
                    ffi::git_reference_create(
                        &raw mut out,
                        self.repo.as_c_ptr(),
                        name.as_ptr(),
                        &raw const self.blob,
                        1,
                        ptr::null(),
                    )
                };
                // SAFETY: as above.
                unsafe { ffi::git_reference_free(out) };
                rc
            }
        }

        /// What the repository reads back for every name, and what its backend keeps on
        /// disk.
        fn state(&self) -> (Lookups, Vec<String>) {
            let refs = NAMES
                .iter()
                .map(|name| {
                    let mut out = ptr::null_mut();
                    {
                        // SAFETY: a live slot, the live repository and a NUL-terminated name;
                        // the reference is read, then freed.
                        let rc = unsafe {
                            ffi::git_reference_lookup(
                                &raw mut out,
                                self.repo.as_c_ptr(),
                                name.as_ptr(),
                            )
                        };
                        let id = (rc == 0).then(|| {
                            // SAFETY: as above.
                            let target = unsafe { ffi::git_reference_target(out) };
                            // SAFETY: as above.
                            unsafe { (*target).id }
                        });
                        // SAFETY: as above.
                        unsafe { ffi::git_reference_free(out) };
                        (rc, id)
                    }
                })
                .collect();
            (refs, storage(&self.path))
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            // The repository is freed after (refdbs of it are gone); freeing touches no
            // file.
            drop(fs::remove_dir_all(&self.path));
        }
    }

    /// The reference files under `path`: the loose references and `packed-refs` with
    /// their contents, and the reftable stack's table count.
    fn storage(path: &Path) -> Vec<String> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    walk(&p, root, out);
                } else {
                    let rel = p.strip_prefix(root).unwrap().display().to_string();
                    out.push(format!("{rel}: {}", fs::read_to_string(&p).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(&path.join("refs"), path, &mut out);
        if let Ok(packed) = fs::read_to_string(path.join("packed-refs")) {
            out.push(format!("packed-refs: {packed}"));
        }
        if let Ok(list) = fs::read_to_string(path.join("reftable/tables.list")) {
            out.push(format!("tables: {}", list.lines().count()));
        }
        out.sort();
        out
    }

    fn compress(db: *mut ffi::git_refdb) -> c_int {
        // SAFETY: a live refdb with a backend, of a live repository.
        unsafe { ffi::git_refdb_compress(db) }
    }

    /// Runs the same writes and compactions on both pairs, comparing after each step.
    fn same_sequence(c: (&Repo, *mut ffi::git_refdb), rust: (&Repo, &RefdbBox<'_>)) {
        let step = |what: &str| {
            assert_eq!(c.0.state(), rust.0.state(), "after {what}");
        };
        step("construction");
        assert_eq!(compress(c.1), compress(rust.1.as_c_ptr()));
        step("compacting an empty database");
        for name in &NAMES[..2] {
            assert_eq!(c.0.create(name), rust.0.create(name));
        }
        step("two writes");
        assert_eq!(compress(c.1), compress(rust.1.as_c_ptr()));
        step("compacting two references");
        assert_eq!(c.0.create(NAMES[2]), rust.0.create(NAMES[2]));
        assert_eq!(compress(c.1), compress(rust.1.as_c_ptr()));
        step("compacting a third");
        assert_eq!(compress(c.1), compress(rust.1.as_c_ptr()));
        step("compacting again");
    }

    fn formats() -> [(&'static str, ffi::git_refdb_t); 2] {
        [
            ("files", ffi::GIT_REFDB_FILES),
            ("reftable", ffi::GIT_REFDB_REFTABLE),
        ]
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn open_matches_git_refdb_open() {
        isolated(
            concat!(module_path!(), "::open_matches_git_refdb_open"),
            open_matches,
        );
    }

    fn open_matches() {
        for (tag, format) in formats() {
            let c_repo = Repo::new(&format!("open-c-{tag}"), format);
            let rust_repo = Repo::new(&format!("open-rust-{tag}"), format);
            let mut c_db = ptr::null_mut();
            // SAFETY: a writable slot and the live repository.
            let c_rc = unsafe { ffi::git_refdb_open(&raw mut c_db, c_repo.repo.as_c_ptr()) };
            let rust_db = Refdb::open(rust_repo.repo.as_ref());
            assert_eq!(c_reported(c_rc), rust_reported(&rust_db));
            let rust_db = rust_db.unwrap();
            same_sequence((&c_repo, c_db), (&rust_repo, &rust_db));
            // SAFETY: the refdb `git_refdb_open` gave out, freed once.
            unsafe { ffi::git_refdb_free(c_db) };
            drop(rust_db);
            assert_eq!(c_repo.state(), rust_repo.state(), "after release ({tag})");
        }
    }

    /// A fresh files backend of `repo`.
    fn fs_backend(repo: &Repo) -> NonNull<ffi::git_refdb_backend> {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and the live repository.
        let rc = unsafe { ffi::git_refdb_backend_fs(&raw mut out, repo.repo.as_c_ptr()) };
        assert_eq!(rc, 0);
        NonNull::new(out).unwrap()
    }

    /// `git_refdb_new` then `git_refdb_set_backend`, as `Refdb::new` runs them: the refdb,
    /// or the error, with the refdb freed.
    fn c_new(
        repo: &Repo,
        backend: NonNull<ffi::git_refdb_backend>,
    ) -> Result<*mut ffi::git_refdb, (c_int, CString)> {
        let mut db = ptr::null_mut();
        // SAFETY: a writable slot and the live repository.
        c_reported(unsafe { ffi::git_refdb_new(&raw mut db, repo.repo.as_c_ptr()) })?;
        // SAFETY: the fresh refdb, without a backend, and a live backend, taken only on
        // success.
        let rc = unsafe { ffi::git_refdb_set_backend(db, backend.as_ptr()) };
        if let Err(e) = c_reported(rc) {
            // SAFETY: the fresh refdb, freed once; it has no backend to free.
            unsafe { ffi::git_refdb_free(db) };
            return Err(e);
        }
        Ok(db)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_git_refdb_new_and_set_backend() {
        isolated(
            concat!(
                module_path!(),
                "::new_matches_git_refdb_new_and_set_backend"
            ),
            new_matches,
        );
    }

    fn new_matches() {
        let c_repo = Repo::new("new-c", ffi::GIT_REFDB_FILES);
        let rust_repo = Repo::new("new-rust", ffi::GIT_REFDB_FILES);
        let c_db = c_new(&c_repo, fs_backend(&c_repo));
        // SAFETY: the live repository, freed after the refdb, and a fresh backend of it,
        // given to the refdb.
        let rust_db = unsafe { Refdb::new(rust_repo.repo.as_ref(), fs_backend(&rust_repo)) };
        assert_eq!(c_db.clone().map(|_| ()), rust_reported(&rust_db));
        let (c_db, rust_db) = (c_db.unwrap(), rust_db.unwrap());
        same_sequence((&c_repo, c_db), (&rust_repo, &rust_db));
        // SAFETY: the refdb `c_new` built, freed once with its backend.
        unsafe { ffi::git_refdb_free(c_db) };
        drop(rust_db);
        assert_eq!(c_repo.state(), rust_repo.state(), "after release");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn refused_backends_fail_alike_and_stay_the_callers() {
        let c_repo = Repo::new("refused-c", ffi::GIT_REFDB_FILES);
        let rust_repo = Repo::new("refused-rust", ffi::GIT_REFDB_FILES);
        let mut c_backend = ffi::git_refdb_backend::default();
        let mut rust_backend = ffi::git_refdb_backend::default();
        for version in [0, 1, 2, i32::MAX as u32, u32::MAX] {
            c_backend.version = version;
            rust_backend.version = version;
            let c = c_new(&c_repo, NonNull::from(&mut c_backend)).map(|_| ());
            // SAFETY: the live repository and a live backend, refused (every member is
            // unset), so it stays the caller's.
            let rust =
                unsafe { Refdb::new(rust_repo.repo.as_ref(), NonNull::from(&mut rust_backend)) };
            assert!(c.is_err());
            assert_eq!(c, rust_reported(&rust), "version {version}");
        }
        // A real backend whose `lock` has no `unlock`: refused, then freed by its owner.
        let (c_backend, rust_backend) = (fs_backend(&c_repo), fs_backend(&rust_repo));
        // SAFETY: both backends are live and only this test reaches them.
        let as_mut = unsafe { c_backend.as_ptr().as_mut() };
        // SAFETY: as above.
        let unlocks = unsafe {
            (
                as_mut.unwrap().unlock.take(),
                rust_backend.as_ptr().as_mut().unwrap().unlock.take(),
            )
        };
        let c = c_new(&c_repo, c_backend).map(|_| ());
        // SAFETY: the live repository and a live backend, refused, so it stays ours.
        let rust = unsafe { Refdb::new(rust_repo.repo.as_ref(), rust_backend) };
        assert!(c.is_err());
        assert_eq!(c, rust_reported(&rust), "lock without unlock");
        for (backend, unlock) in [(c_backend, unlocks.0), (rust_backend, unlocks.1)] {
            {
                let b = backend.as_ptr();
                // SAFETY: the live backend libgit2 refused, restored and released once
                // through its own `free`.
                unsafe { (*b).unlock = unlock };
                // SAFETY: as above.
                let free2 = unsafe { (*b).free };
                // SAFETY: as above.
                unsafe { (free2.unwrap())(b) };
            }
        }
    }
}

mod refdb_type {
    //! Equivalence of `git2/refdb.h`'s [`RefdbType`] with the values libgit2 accepts as a
    //! reference database format.
    //!
    //! libgit2 observes a `git_refdb_t` through `git_repository_init_ext`, which takes it
    //! in `git_repository_init_options.refdb_type`, records it as the
    //! `extensions.refstorage` setting, and opens the repository it built, reading the
    //! setting back. Each probe initialises a fresh bare repository with one raw value,
    //! then opens it again and looks its `HEAD` up; the Rust conversion alone predicts
    //! every step: the result and error of the initialisation, the setting and directory
    //! it leaves on disk, the reopening, and the lookup. A zero value is the default
    //! format, exactly as [`RefdbType::default`] is.
    //!
    //! Recorded C behaviour, outside the type: a value the header does not name is
    //! written as `refstorage = unknown`, so the initialisation fails with `GIT_EINVALID`
    //! ("unknown refstorage format 'unknown'") but leaves that unopenable repository on
    //! disk.

    use std::ffi::{CStr, CString, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::{fs, ptr};

    use libgit2::ffi;
    use libgit2::libgit2::refs::refdb::RefdbType;

    /// Initialises libgit2; never released.
    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// The calling thread's last error class and message.
    fn c_last() -> (c_int, String) {
        {
            // SAFETY: `git_error_last` never returns NULL after init, and its message is a
            // NUL-terminated string, both live until the next libgit2 call on this thread,
            // after the copy.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
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
    }

    /// What libgit2 did with one `refdb_type`, step by step.
    #[derive(Debug, PartialEq, Eq)]
    struct Outcome {
        /// `git_repository_init_ext`'s result, and its error when it failed.
        init: Result<(), (c_int, (c_int, String))>,
        /// The `extensions.refstorage` value the repository's config records.
        refstorage: Option<String>,
        /// Whether a `reftable/` directory was made.
        reftable_dir: bool,
        /// `git_repository_open_ext`'s result, and its error when it failed.
        open: Result<(), (c_int, (c_int, String))>,
        /// `HEAD`'s target, looked up through the reopened repository.
        head: Option<String>,
    }

    /// A probe directory, removed on drop.
    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.0));
        }
    }

    /// The `refstorage` line of the `[extensions]` section of a config file.
    fn refstorage(config: &str) -> Option<String> {
        config
            .lines()
            .map(str::trim)
            .skip_while(|l| *l != "[extensions]")
            .find_map(|l| l.strip_prefix("refstorage = "))
            .map(str::to_owned)
    }

    /// Runs the C steps for `raw` in a fresh directory.
    fn c_outcome(tag: &str, raw: ffi::git_refdb_t) -> (Outcome, String) {
        let path =
            std::env::temp_dir().join(format!("crustify-refdb-equiv-{}-{tag}", std::process::id()));
        drop(fs::remove_dir_all(&path));
        let scratch = Scratch(path);
        let c_path = CString::new(scratch.0.as_os_str().as_bytes()).unwrap();
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &raw mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
        opts.refdb_type = raw;
        let mut repo = ptr::null_mut();
        // SAFETY: a live slot, a NUL-terminated path and live options, read for the call;
        // the result is a fresh repository on success, NULL otherwise.
        let rc =
            unsafe { ffi::git_repository_init_ext(&raw mut repo, c_path.as_ptr(), &raw mut opts) };
        let init = if rc == 0 { Ok(()) } else { Err((rc, c_last())) };
        // SAFETY: the repository just built, or NULL, which the free accepts.
        unsafe { ffi::git_repository_free(repo) };

        let config = fs::read_to_string(scratch.0.join("config")).unwrap_or_default();
        let reftable_dir = scratch.0.join("reftable").is_dir();

        let mut repo = ptr::null_mut();
        // SAFETY: a live slot and a NUL-terminated path, read for the call; no ceiling.
        let rc = unsafe {
            ffi::git_repository_open_ext(
                &raw mut repo,
                c_path.as_ptr(),
                ffi::GIT_REPOSITORY_OPEN_BARE | ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
                ptr::null(),
            )
        };
        let open = if rc == 0 { Ok(()) } else { Err((rc, c_last())) };
        let head = (rc == 0).then(|| {
            let mut head = ptr::null_mut();
            // SAFETY: a live slot, the open repository and a NUL-terminated name.
            let rc = unsafe { ffi::git_reference_lookup(&raw mut head, repo, c"HEAD".as_ptr()) };
            assert_eq!(rc, 0, "{:?}", c_last());
            {
                // SAFETY: a live symbolic reference: its target lives as long as it, copied
                // before the reference is freed once.
                let reference_symbolic_target = unsafe { ffi::git_reference_symbolic_target(head) };
                // SAFETY: as above.
                let target = unsafe {
                    CStr::from_ptr(reference_symbolic_target)
                        .to_string_lossy()
                        .into_owned()
                };
                // SAFETY: as above.
                unsafe { ffi::git_reference_free(head) };
                target
            }
        });
        // SAFETY: the reopened repository, or NULL.
        unsafe { ffi::git_repository_free(repo) };
        (
            Outcome {
                init,
                refstorage: refstorage(&config),
                reftable_dir,
                open,
                head,
            },
            config,
        )
    }

    /// What the Rust conversion predicts for `raw`, given the branch a fresh repository's
    /// `HEAD` names.
    fn predicted(raw: ffi::git_refdb_t, head: &str) -> Outcome {
        let format = if raw == 0 {
            Some(RefdbType::default())
        } else {
            RefdbType::from_raw(raw)
        };
        let unknown = || {
            Err((
                ffi::GIT_EINVALID,
                (
                    ffi::GIT_ERROR_REPOSITORY.cast_signed(),
                    "unknown refstorage format 'unknown'".to_owned(),
                ),
            ))
        };
        match format {
            Some(format) => Outcome {
                init: Ok(()),
                refstorage: (format == RefdbType::Reftable).then(|| "reftable".to_owned()),
                reftable_dir: format == RefdbType::Reftable,
                open: Ok(()),
                head: Some(head.to_owned()),
            },
            None => Outcome {
                init: unknown(),
                refstorage: Some("unknown".to_owned()),
                reftable_dir: false,
                open: unknown(),
                head: None,
            },
        }
    }

    /// Every value around the declared ones, and the extremes.
    fn probes() -> impl Iterator<Item = ffi::git_refdb_t> {
        (0..=5).chain([100, ffi::git_refdb_t::MAX - 1, ffi::git_refdb_t::MAX])
    }

    /// Each raw value initialises, records, reopens and resolves `HEAD` exactly as the
    /// Rust conversion predicts.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn init_follows_the_rust_type() {
        init();
        let (files, _) = c_outcome("head", RefdbType::Files.raw());
        let head = files.head.expect("a files repository opens");
        for raw in probes() {
            let (outcome, _) = c_outcome(&format!("probe-{raw}"), raw);
            assert_eq!(outcome, predicted(raw, &head), "refdb_type {raw}");
        }
    }

    /// A zeroed `refdb_type` builds the same repository as the default variant's value.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn zero_is_the_default_format() {
        init();
        let (zero, zero_config) = c_outcome("zero", 0);
        let (default, default_config) = c_outcome("default", RefdbType::default().raw());
        assert_eq!(zero, default);
        assert_eq!(zero_config, default_config);
        assert_eq!(zero.refstorage, None);
    }
}
