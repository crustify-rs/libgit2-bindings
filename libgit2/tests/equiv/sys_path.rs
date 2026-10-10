//! Equivalence tests for `git2/sys/path.h`.
//!
//! [`GitFile`] must name exactly the values `git_path_is_gitfile` accepts, each the
//! file whose name it recognises: a probe over every value around the declared ones
//! predicts from [`GitFile::from_raw`] alone whether C refuses the value ("invalid
//! gitfile for path validation"), and which of the three names, in its plain form
//! and its NTFS short forms, C matches for each variant.

mod git_file {
    use std::ffi::{CStr, c_int};

    use libgit2::ffi;
    use libgit2::libgit2::repository::sys::path::GitFile;

    fn init() {
        // SAFETY: balanced by nothing: the reference lives for the process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
    }

    fn c_last() -> (c_int, String) {
        // SAFETY: the runtime is initialised; the error is copied before any other call.
        unsafe {
            let e = ffi::git_error_last();
            (
                (*e).klass,
                CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
            )
        }
    }

    /// `git_path_is_gitfile(component, raw)` with the generic checks, and the error a
    /// refusal sets.
    ///
    /// The component is NUL-terminated: C's NTFS checks read up to a NUL whatever the
    /// length says.
    fn is_gitfile(
        component: &CStr,
        raw: ffi::git_path_gitfile,
    ) -> (c_int, Option<(c_int, String)>) {
        // SAFETY: `component` is a live NUL-terminated string, read for the call only.
        let rc = unsafe {
            ffi::git_path_is_gitfile(
                component.as_ptr(),
                component.to_bytes().len(),
                raw,
                ffi::GIT_PATH_FS_GENERIC,
            )
        };
        (rc, (rc < 0).then(c_last))
    }

    /// The names each variant means, in their plain and NTFS short forms.
    fn names(file: GitFile) -> [&'static CStr; 3] {
        match file {
            GitFile::Gitignore => [c".gitignore", c"GITIGN~1", c"gi250a~9"],
            GitFile::Gitmodules => [c".gitmodules", c"gitmod~4", c"GI7EBA~1"],
            GitFile::Gitattributes => [c".gitattributes", c"gitatt~2", c"gi7d29~3"],
        }
    }

    const ALL: [GitFile; 3] = [
        GitFile::Gitignore,
        GitFile::Gitmodules,
        GitFile::Gitattributes,
    ];

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn variants_are_what_git_path_is_gitfile_accepts() {
        init();
        let probes = (0..=64).chain([0x7fff_ffff, 0x8000_0000, u32::MAX - 1, u32::MAX]);
        for raw in probes {
            let c = is_gitfile(c".gitmodules", raw);
            match GitFile::from_raw(raw) {
                Some(file) => {
                    assert_eq!(file.raw(), raw);
                    assert_eq!(c, (c_int::from(file == GitFile::Gitmodules), None), "{raw}");
                }
                None => {
                    // A `GIT_ERROR_OS` message carries the thread's `errno` after it.
                    let (rc, err) = c;
                    let (class, message) = err.unwrap();
                    assert_eq!((rc, class), (-1, ffi::GIT_ERROR_OS as c_int), "{raw}");
                    assert!(message.starts_with("invalid gitfile for path validation"));
                }
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn each_variant_recognises_its_own_file() {
        init();
        for file in ALL {
            for other in ALL {
                for name in names(other) {
                    let (rc, err) = is_gitfile(name, file.into());
                    assert_eq!(err, None);
                    assert_eq!(rc == 1, file == other, "{file:?} against {:?}", name);
                }
            }
            // Not a git file at all.
            assert_eq!(is_gitfile(c"README", file.raw()), (0, None));
        }
    }
}
