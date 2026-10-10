//! Equivalence of `git2/sys/repository.h`'s wrappers with the C values and routines
//! they stand for, one section per batch.

/// [`NewOptions`] against the `git_repository_new_options` C code builds.
///
/// [`NewOptions::new`] must build what `git_repository_new_options_init` and
/// `GIT_REPOSITORY_NEW_OPTIONS_INIT` do, and the raw conversions must keep exactly the
/// values C accepts. Then one sequence of option changes runs twice: once on a C struct
/// written field by field, once on a [`NewOptions`] through its setters, each
/// persisting across the steps. After every step the getters must read what the C
/// struct holds, and `git_repository_new_ext` must behave the same with either: its
/// return, the error of a failure, the new repository's object id type and bareness,
/// and the options left unchanged. [`Repository::with_oid_type`], which builds its
/// options through [`NewOptions`], must match `git_repository_new_ext` with C's.
///
/// [`NewOptions`]: libgit2::libgit2::repository::sys::repository::NewOptions
/// [`NewOptions::new`]: libgit2::libgit2::repository::sys::repository::NewOptions::new
/// [`Repository::with_oid_type`]: libgit2::libgit2::repository::repository::Repository::with_oid_type
mod new_options {
    use std::ffi::{CStr, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::repository::repository::Repository;
    use libgit2::libgit2::repository::sys::repository::NewOptions;

    fn init() {
        // SAFETY: no arguments; the reference is never released.
        assert!(unsafe { ffi::git_libgit2_init() } >= 1);
    }

    /// `git_repository_new_options_init(version)` into a zeroed struct.
    fn c_options(version: u32) -> (c_int, ffi::git_repository_new_options) {
        let mut opts = ffi::git_repository_new_options::default();
        // SAFETY: `opts` is a live options struct the call may overwrite.
        let rc = unsafe { ffi::git_repository_new_options_init(&mut opts, version) };
        (rc, opts)
    }

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
        let rc = unsafe { ffi::git_repository_new_ext(&mut repo, opts) };
        let error = (rc != 0).then(|| {
            // SAFETY: after a failure `git_error_last` is this thread's error, live
            // until the next libgit2 call on this thread; it is copied out first.
            let e = unsafe { &*ffi::git_error_last() };
            // SAFETY: a set error's message is a NUL-terminated string.
            let message = unsafe { CStr::from_ptr(e.message) }.to_string_lossy();
            (e.klass, message.into_owned())
        });
        let repo_state = (!repo.is_null()).then(|| {
            // SAFETY: `repo` is the live repository the call built, only read.
            unsafe {
                (
                    ffi::git_repository_oid_type(repo),
                    ffi::git_repository_is_bare(repo),
                )
            }
        });
        // SAFETY: `repo` is NULL or the fresh repository, released once here.
        unsafe { ffi::git_repository_free(repo) };
        (rc, error, repo_state)
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
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
            let c_call = new_ext(&mut c);
            let rust_ptr = ptr::from_mut(&mut rust).cast::<ffi::git_repository_new_options>();
            let rust_call = new_ext(rust_ptr);
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
            let (rc, error, repo) = new_ext(&mut c);
            assert_eq!((rc, error), (0, None));
            let rust = Repository::with_oid_type(kind).unwrap();
            // SAFETY: the live repository, only read.
            let rust_state = unsafe {
                (
                    ffi::git_repository_oid_type(rust.as_c_ptr()),
                    ffi::git_repository_is_bare(rust.as_c_ptr()),
                )
            };
            assert_eq!(Some(rust_state), repo, "{kind:?}");
        }
    }
}
