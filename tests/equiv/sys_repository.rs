//! Equivalence of `git2/sys/repository.h`'s wrappers with the C values and routines
//! they stand for, one section per batch.

/// [`NewOptions`] against the `git_repository_new_options` C code builds.
///
/// [`NewOptions::new`] must build what `git_repository_new_options_init` and
/// `GIT_REPOSITORY_NEW_OPTIONS_INIT` do, and the raw conversions must keep exactly the
/// values C accepts. Then one sequence of option changes runs twice: once on a C struct
/// written field by field, once on a [`NewOptions`] through its setters, each
/// persisting across the steps. After every step the getters must read what the C
/// struct holds, and [`Repository::new_ext`] with the Rust options must behave as
/// `git_repository_new_ext` with the C struct: the return, the error of a failure, the
/// new repository's object id type and bareness, and the options left unchanged.
/// [`Repository::with_oid_type`], which builds its options through [`NewOptions`], must
/// match `git_repository_new_ext` with C's.
///
/// [`NewOptions`]: libgit2::libgit2::repository::sys::repository::NewOptions
/// [`NewOptions::new`]: libgit2::libgit2::repository::sys::repository::NewOptions::new
/// [`Repository::new_ext`]: libgit2::libgit2::repository::repository::Repository::new_ext
/// [`Repository::with_oid_type`]: libgit2::libgit2::repository::repository::Repository::with_oid_type
mod new_options {
    use std::ffi::{CStr, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::Repository;
    use libgit2::libgit2::repository::sys::repository::{NewOptions, NewOptionsRef};

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// `git_repository_new_options_init(version)` into a zeroed struct.
    fn c_options(version: u32) -> (c_int, ffi::git_repository_new_options) {
        let mut opts = ffi::git_repository_new_options::default();
        // SAFETY: `opts` is a live options struct the call may overwrite.
        let rc = unsafe { ffi::git_repository_new_options_init(&raw mut opts, version) };
        (rc, opts)
    }

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "callers hold a reference to the value"
    )]
    fn fields(o: &NewOptions) -> (u32, u32) {
        let r = o.as_ref();
        (r.version(), r.oid_type().map_or(0, OidType::raw))
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_the_c_initialisers() {
        let (rc, c) = c_options(ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION);
        assert_eq!(rc, 0);
        let rust = NewOptions::new();
        assert_eq!(fields(&rust), (c.version, c.oid_type));
        // `GIT_REPOSITORY_NEW_OPTIONS_INIT` is `{VERSION}`.
        assert_eq!(fields(&rust), (ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION, 0));
        assert_eq!(rust.as_ref().version(), NewOptions::VERSION);
        // C refuses other versions, leaving the struct as it was; `new` has none.
        for version in [0, 2] {
            let (rc, c) = c_options(version);
            assert_eq!((rc, c.version), (-1, 0), "version {version}");
        }
    }

    #[test]
    fn conversions_keep_exactly_what_c_accepts() {
        for oid_type in [0, ffi::GIT_OID_SHA1, ffi::GIT_OID_SHA256, 3, u32::MAX] {
            for version in [0, 1, 2] {
                let raw = ffi::git_repository_new_options { version, oid_type };
                // `git_repository_new_ext` asserts a non-zero type is valid.
                let valid = oid_type == 0 || OidType::from_raw(oid_type).is_some();
                match NewOptions::try_from(raw) {
                    Ok(o) => {
                        assert!(valid, "{oid_type}");
                        assert_eq!(fields(&o), (version, oid_type));
                        let back = ffi::git_repository_new_options::from(o);
                        assert_eq!((back.version, back.oid_type), (version, oid_type));
                    }
                    Err(e) => assert_eq!((valid, e.raw()), (false, oid_type)),
                }
            }
        }
    }

    /// One call's record: the return, the class and message of a failure, and the new
    /// repository's object id type and bareness.
    type Call = (c_int, Option<(c_int, String)>, Option<(u32, c_int)>);

    /// `git_repository_new_ext(opts)`, freeing the repository it builds.
    fn new_ext(opts: *mut ffi::git_repository_new_options) -> Call {
        let mut repo = ptr::null_mut();
        // SAFETY: `repo` is a live slot and `opts` live options, only read.
        let rc = unsafe { ffi::git_repository_new_ext(&raw mut repo, opts) };
        let error = (rc != 0).then(|| {
            // SAFETY: after a failure `git_error_last` is this thread's error, live until
            // the next libgit2 call on this thread.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above, non-NULL; it is copied out first.
            let e = unsafe { &*e };
            // SAFETY: a set error's message is a NUL-terminated string.
            let message = unsafe { CStr::from_ptr(e.message) }.to_string_lossy();
            (e.klass, message.into_owned())
        });
        let repo_state = (!repo.is_null()).then(|| {
            // SAFETY: `repo` is the live repository the call built, only read.
            let repository_oid_type = unsafe { ffi::git_repository_oid_type(repo) };
            // SAFETY: as above.
            unsafe { (repository_oid_type, ffi::git_repository_is_bare(repo)) }
        });
        // SAFETY: `repo` is NULL or the fresh repository, released once here.
        unsafe { ffi::git_repository_free(repo) };
        (rc, error, repo_state)
    }

    /// `Repository::new_ext(opts)`, as [`new_ext`] records it.
    fn rust_new_ext(opts: NewOptionsRef<'_>) -> Call {
        match Repository::new_ext(opts) {
            Ok(repo) => {
                let r = repo.as_ref();
                (
                    0,
                    None,
                    Some((r.oid_type().raw(), c_int::from(r.is_bare()))),
                )
            }
            Err(e) => {
                let message = e.message().to_string_lossy().into_owned();
                (e.raw_code(), Some((e.raw_class(), message)), None)
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    #[expect(
        clippy::items_after_statements,
        reason = "each helper item sits beside the steps that use it"
    )]
    fn new_ext_reads_rust_options_as_c_options() {
        init();
        type Change = (
            &'static str,
            fn(&mut ffi::git_repository_new_options),
            fn(&mut NewOptions),
        );
        let steps: [Change; 8] = [
            ("the defaults", |_| (), |_| ()),
            (
                "SHA-256",
                |c| c.oid_type = ffi::GIT_OID_SHA256,
                |r| r.as_mut().set_oid_type(Some(OidType::Sha256)),
            ),
            ("the same options again", |_| (), |_| ()),
            (
                "SHA-1",
                |c| c.oid_type = ffi::GIT_OID_SHA1,
                |r| r.as_mut().set_oid_type(Some(OidType::Sha1)),
            ),
            (
                "version 0",
                |c| c.version = 0,
                |r| r.as_mut().set_version(0),
            ),
            (
                "version 2",
                |c| c.version = 2,
                |r| r.as_mut().set_version(2),
            ),
            (
                "version 1, the default type",
                |c| (c.version, c.oid_type) = (1, 0),
                |r| {
                    let mut m = r.as_mut();
                    m.set_version(1);
                    m.set_oid_type(None);
                },
            ),
            (
                "the highest version",
                |c| c.version = u32::MAX,
                |r| r.as_mut().set_version(u32::MAX),
            ),
        ];
        let mut c = c_options(ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION).1;
        let mut rust = NewOptions::new();
        let mut calls = Vec::new();
        for (tag, c_change, rust_change) in steps {
            c_change(&mut c);
            rust_change(&mut rust);
            assert_eq!(fields(&rust), (c.version, c.oid_type), "{tag}: fields");
            let c_call = new_ext(&raw mut c);
            let rust_call = rust_new_ext(rust.as_ref());
            assert_eq!(rust_call, c_call, "{tag}");
            assert_eq!(
                fields(&rust),
                (c.version, c.oid_type),
                "{tag}: after the call"
            );
            calls.push(c_call);
        }
        let summary: Vec<_> = calls.iter().map(|(rc, _, repo)| (*rc, *repo)).collect();
        let (sha1, sha256) = (Some((ffi::GIT_OID_SHA1, 1)), Some((ffi::GIT_OID_SHA256, 1)));
        assert_eq!(
            summary,
            [
                (0, sha1),
                (0, sha256),
                (0, sha256),
                (0, sha1),
                (-1, None),
                (-1, None),
                (0, sha1),
                (-1, None)
            ]
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn with_oid_type_matches_new_ext() {
        init();
        for kind in [OidType::Sha1, OidType::Sha256] {
            let mut c = c_options(ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION).1;
            c.oid_type = kind.raw();
            let (rc, error, repo) = new_ext(&raw mut c);
            assert_eq!((rc, error), (0, None));
            let rust = Repository::with_oid_type(kind).unwrap();
            // SAFETY: the live repository, only read.
            let repository_oid_type = unsafe { ffi::git_repository_oid_type(rust.as_c_ptr()) };
            // SAFETY: as above.
            let rust_state = unsafe {
                (
                    repository_oid_type,
                    ffi::git_repository_is_bare(rust.as_c_ptr()),
                )
            };
            assert_eq!(Some(rust_state), repo, "{kind:?}");
        }
    }
}

/// `RepositoryMut::cache_submodules` and `clear_submodule_cache` against
/// `git_repository_submodule_cache_all` and `git_repository_submodule_cache_clear`.
///
/// Each side owns a repository with a working directory whose `.gitmodules` the steps
/// rewrite; the C side loads and clears the cache through the raw routines, the Rust
/// side through the wrappers, and both observe it through `git_submodule_lookup`,
/// which answers from the cache while there is one. After every step the return, the
/// error and what a lookup of each submodule gives (its code, or its `ignore` setting)
/// must agree: a loaded cache keeps answering with what `.gitmodules` said when it was
/// loaded, a cleared one hands lookups back to the repository.
///
/// The wrapper releases a loaded cache before loading another, where libgit2 leaks it
/// and keeps it when the load fails; those sequences diverge on purpose (the unit tests
/// of `repository/sys/repository.rs` cover them), so loads here start without a cache.
mod submodule_cache {
    use std::ffi::{CStr, CString, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::{fs, ptr};

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// Rewrites `.gitmodules`: `sub` with this `ignore` setting, and `other` when
        /// asked.
        Modules(&'static str, bool),
        /// Rewrites `.gitmodules` with a syntax error.
        Break,
        Cache,
        Clear,
    }

    type Lookup = Result<ffi::git_submodule_ignore_t, c_int>;
    type Outcome = (c_int, Option<(c_int, String)>, Lookup, Lookup);

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// The thread's last error as C reports it.
    fn c_last() -> (c_int, String) {
        {
            // SAFETY: the runtime is initialised; `git_error_last` never returns NULL, and
            // its message is a C string, live until the next libgit2 call on this thread.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
            let message2 = unsafe { (*e).message };
            // SAFETY: as above.
            let message = unsafe { CStr::from_ptr(message2).to_string_lossy().into_owned() };
            // SAFETY: as above.
            unsafe { ((*e).klass, message) }
        }
    }

    /// A working directory of its own, removed on drop.
    struct Root(PathBuf);

    impl Root {
        fn new(tag: &str) -> Self {
            init();
            let path = std::env::temp_dir().join(format!(
                "crustify-sys-repository-equiv-{}-{tag}",
                std::process::id()
            ));
            drop(fs::remove_dir_all(&path));
            Self(path)
        }

        fn c_path(&self) -> CString {
            CString::new(self.0.as_os_str().as_bytes()).unwrap()
        }

        fn write(&self, step: Step) {
            let modules = match step {
                Step::Modules(ignore, other) => {
                    let mut s = format!(
                        "[submodule \"sub\"]\n\tpath = sub\n\turl = https://example.com/sub\n\
                         \tignore = {ignore}\n"
                    );
                    if other {
                        s += "[submodule \"other\"]\n\tpath = other\n\
                              \turl = https://example.com/other\n\tignore = dirty\n";
                    }
                    s
                }
                Step::Break => "[submodule \"sub\"\n".to_owned(),
                Step::Cache | Step::Clear => unreachable!(),
            };
            fs::write(self.0.join(".gitmodules"), modules).unwrap();
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.0));
        }
    }

    /// `git_submodule_lookup(name)`: the submodule's `ignore` setting, or the code.
    fn lookup(repo: *mut ffi::git_repository, name: &CStr) -> Lookup {
        let mut sm = ptr::null_mut();
        {
            // SAFETY: a writable slot, a live repository on this thread and a
            // NUL-terminated name; the submodule reference is released once.
            let rc = unsafe { ffi::git_submodule_lookup(&raw mut sm, repo, name.as_ptr()) };
            if rc != 0 {
                return Err(rc);
            }
            // SAFETY: as above.
            let ignore = unsafe { ffi::git_submodule_ignore(sm) };
            // SAFETY: as above.
            unsafe { ffi::git_submodule_free(sm) };
            Ok(ignore)
        }
    }

    fn observe(
        root: &Root,
        repo: *mut ffi::git_repository,
        rc: c_int,
        error: Option<(c_int, String)>,
    ) -> Outcome {
        let root_str = root.0.display().to_string();
        let error = error.map(|(k, m)| (k, m.replace(&root_str, "<root>")));
        (rc, error, lookup(repo, c"sub"), lookup(repo, c"other"))
    }

    fn compare(tag: &str, steps: &[Step]) {
        let (c_root, rust_root) = (
            Root::new(&format!("{tag}-c")),
            Root::new(&format!("{tag}-rust")),
        );
        let mut c = ptr::null_mut();
        let path = c_root.c_path();
        // SAFETY: a writable slot and a NUL-terminated path; the repository is freed at
        // the end.
        let rc = unsafe { ffi::git_repository_init(&raw mut c, path.as_ptr(), 0) };
        assert_eq!(rc, 0);
        let mut rust: RepositoryBox = Repository::init(&rust_root.c_path(), false).unwrap();
        for (k, &step) in steps.iter().enumerate() {
            let (c_out, rust_out) = match step {
                Step::Cache | Step::Clear => {
                    // SAFETY: no arguments; clears this thread's error state.
                    unsafe { ffi::git_error_clear() };
                    let rc = if matches!(step, Step::Cache) {
                        // SAFETY: the C side's live repository, on this thread.
                        unsafe { ffi::git_repository_submodule_cache_all(c) }
                    } else {
                        // SAFETY: as above.
                        unsafe { ffi::git_repository_submodule_cache_clear(c) }
                    };
                    let c_out = observe(&c_root, c, rc, (rc < 0).then(c_last));
                    let (rc, error) = if matches!(step, Step::Cache) {
                        match rust.as_mut().cache_submodules() {
                            Ok(()) => (0, None),
                            Err(e) => (
                                e.raw_code(),
                                Some((e.raw_class(), e.message().to_string_lossy().into_owned())),
                            ),
                        }
                    } else {
                        rust.as_mut().clear_submodule_cache();
                        (0, None)
                    };
                    (c_out, observe(&rust_root, rust.as_c_ptr(), rc, error))
                }
                step => {
                    c_root.write(step);
                    rust_root.write(step);
                    (
                        observe(&c_root, c, 0, None),
                        observe(&rust_root, rust.as_c_ptr(), 0, None),
                    )
                }
            };
            assert_eq!(rust_out, c_out, "step {k} {step:?}");
        }
        drop(rust);
        // SAFETY: the C side's repository, released once (with any cache it holds).
        unsafe { ffi::git_repository_free(c) };
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn cache_sequences_match_c() {
        use Step::*;
        compare(
            "sequence",
            &[
                Clear,
                Cache,
                Modules("none", false),
                Clear,
                Cache,
                Modules("all", true),
                Clear,
                Modules("untracked", true),
                Cache,
                Modules("dirty", false),
                Clear,
                Clear,
                Cache,
            ],
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn failed_loads_match_c() {
        use Step::*;
        compare(
            "failed",
            &[
                Break,
                Cache,
                Modules("all", false),
                Cache,
                Break,
                Clear,
                Cache,
            ],
        );
    }
}
