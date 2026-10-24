//! Equivalence tests for `git2/sys/path.h`.
//!
//! [`GitFile`] must name exactly the values `git_path_is_gitfile` accepts, each the
//! file whose name it recognises: a probe over every value around the declared ones
//! predicts from [`GitFile::from_raw`] alone whether C refuses the value ("invalid
//! gitfile for path validation"), and which of the three names, in its plain form
//! and its NTFS short forms, C matches for each variant.
//!
//! [`Filesystem`] is compared in `mod filesystem`.

mod git_file {
    use std::ffi::{CStr, c_int};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::path::GitFile;

    fn init() {
        // SAFETY: balanced by nothing: the reference lives for the process.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
    }

    fn c_last() -> (c_int, String) {
        {
            // SAFETY: the runtime is initialised; the error is copied before any other call.
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
        let probes = (0..=64).chain([0x7FFF_FFFF, 0x8000_0000, u32::MAX - 1, u32::MAX]);
        for raw in probes {
            let c = is_gitfile(c".gitmodules", raw);
            if let Some(file) = GitFile::from_raw(raw) {
                assert_eq!(file.raw(), raw);
                assert_eq!(c, (c_int::from(file == GitFile::Gitmodules), None), "{raw}");
            } else {
                // A `GIT_ERROR_OS` message carries the thread's `errno` after it.
                let (rc, err) = c;
                let (class, message) = err.unwrap();
                assert_eq!((rc, class), (-1, ffi::GIT_ERROR_OS.cast_signed()), "{raw}");
                assert!(message.starts_with("invalid gitfile for path validation"));
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
                    assert_eq!(rc == 1, file == other, "{file:?} against {name:?}");
                }
            }
            // Not a git file at all.
            assert_eq!(is_gitfile(c"README", file.raw()), (0, None));
        }
    }
}

mod filesystem {
    //! [`Filesystem`] (`git_path_fs`) against `git_path_is_gitfile`, its one reader.
    //!
    //! [`Filesystem::from_raw`] must accept exactly the values the routine accepts: every
    //! other value fails with `GIT_ERROR_OS`, "invalid filesystem for path validation: "
    //! (and `errno`'s text, cleared here), for every git file. Each variant must apply the rules
    //! its documentation names, on names that tell them apart: [`Filesystem::Ntfs`] matches 8.3
    //! short names and trailing dots and spaces, [`Filesystem::Hfs`] skips ignorable code
    //! points, both fold ASCII case, and [`Filesystem::Generic`] matches what either one
    //! matches. The git file is a [`GitFile`].

    use std::ffi::{CStr, CString, c_int};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::path::{Filesystem, GitFile, UnnamedFilesystem};

    /// Takes a `git_libgit2_init` reference that is never released.
    fn init() {
        let _ =
            core::mem::ManuallyDrop::new(libgit2::libgit2::src::libgit2::libgit2::init().unwrap());
    }

    /// `git_path_is_gitfile(name, gitfile, fs)`, with the thread's error on failure.
    ///
    /// `errno` is cleared first: a `GIT_ERROR_OS` message ends with `strerror(errno)`
    /// whenever `errno` is set, whatever set it.
    fn is_gitfile(
        name: &str,
        gitfile: GitFile,
        fs: ffi::git_path_fs,
    ) -> Result<c_int, (c_int, String)> {
        let name = CString::new(name).unwrap();
        let len = name.as_bytes().len();
        // SAFETY: the calling thread's `errno`, valid for writes.
        let value2 = unsafe { libc::__errno_location() };
        // SAFETY: as above.
        unsafe { value2.write(0) };
        // SAFETY: `name` is NUL-terminated and `len` long, read for the call only (the
        // NTFS check reads past `len` up to the NUL, which the `CString` supplies).
        let rc = unsafe { ffi::git_path_is_gitfile(name.as_ptr(), len, gitfile.raw(), fs) };
        if rc >= 0 {
            return Ok(rc);
        }
        // SAFETY: libgit2 is initialised; the result is live until the next libgit2 call
        // on this thread, and is copied before one.
        let e = unsafe { ffi::git_error_last() };
        // SAFETY: as above.
        let klass = unsafe { (*e).klass };
        // SAFETY: as above.
        let message = unsafe { (*e).message };
        // SAFETY: as above, a NUL-terminated message; it is copied.
        Err((
            klass,
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned(),
        ))
    }

    /// `.gitmodules` spellings and whether NTFS's and HFS+'s rules match each one.
    const GITMODULES: &[(&str, bool, bool)] = &[
        (".gitmodules", true, true),
        (".GitModules", true, true),
        ("GITMOD~1", true, false),
        ("gitmod~4", true, false),
        ("gi7eba~9", true, false),
        (".gitmodules.", true, false),
        (".gitmodules . ", true, false),
        (".gitmodules:$DATA", true, false),
        (".git\u{200c}modules", false, true),
        (".gitmodules\u{feff}", false, true),
        ("\u{200e}.GITMODULES", false, true),
        (".gitmodule", false, false),
        (".gitmodulesx", false, false),
        ("gitmodules", false, false),
        ("", false, false),
    ];

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn variants_match_as_their_rules_say() {
        init();
        for &(name, ntfs, hfs) in GITMODULES {
            let seen =
                |fs: Filesystem| is_gitfile(name, GitFile::Gitmodules, fs.raw()).unwrap() > 0;
            assert_eq!(seen(Filesystem::Ntfs), ntfs, "{name:?} on NTFS");
            assert_eq!(seen(Filesystem::Hfs), hfs, "{name:?} on HFS+");
            assert_eq!(seen(Filesystem::Generic), ntfs || hfs, "{name:?} on both");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_raw_accepts_what_git_path_is_gitfile_accepts() {
        init();
        let invalid = (
            ffi::GIT_ERROR_OS.cast_signed(),
            "invalid filesystem for path validation: ".to_owned(),
        );
        let probes = (0..=16).chain([0x7FFF_FFFF, 0x8000_0000, u32::MAX - 1, u32::MAX]);
        for raw in probes {
            for gitfile in [
                GitFile::Gitignore,
                GitFile::Gitmodules,
                GitFile::Gitattributes,
            ] {
                for name in [".gitignore", ".gitmodules", ".gitattributes", "other"] {
                    let c = is_gitfile(name, gitfile, raw);
                    match Filesystem::from_raw(raw) {
                        Some(fs) => {
                            assert_eq!(fs.raw(), raw);
                            assert!(matches!(c, Ok(0 | 1)), "{raw} {gitfile:?} {name}: {c:?}");
                        }
                        None => assert_eq!(c, Err(invalid.clone()), "{raw} {gitfile:?} {name}"),
                    }
                }
            }
            assert_eq!(
                Filesystem::try_from(raw).map_err(UnnamedFilesystem::raw),
                Filesystem::from_raw(raw).ok_or(raw)
            );
        }
    }
}

/// [`is_gitfile`] against `git_path_is_gitfile` given the same NUL-terminated
/// component and its full length: every variant pair over a corpus of each file's
/// spellings (plain, case-folded, NTFS 8.3 short names and trailing dots, spaces and
/// streams, HFS+ ignorable code points), near misses, prefixes and long names, and a
/// seeded sweep of short names drawn from the bytes those rules look at.
mod is_gitfile {
    use std::ffi::{CStr, CString};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::path::{Filesystem, GitFile, is_gitfile};

    const FILES: [GitFile; 3] = [
        GitFile::Gitignore,
        GitFile::Gitmodules,
        GitFile::Gitattributes,
    ];
    const FILESYSTEMS: [Filesystem; 3] = [Filesystem::Generic, Filesystem::Ntfs, Filesystem::Hfs];

    /// C's answer: whether it matched; it never refuses a named pair.
    fn c_is_gitfile(component: &CStr, file: GitFile, fs: Filesystem) -> bool {
        // SAFETY: a live NUL-terminated component with its full length; named values.
        let rc = unsafe {
            ffi::git_path_is_gitfile(
                component.as_ptr(),
                component.to_bytes().len(),
                file.raw(),
                fs.raw(),
            )
        };
        assert!(rc >= 0, "C refused a named pair");
        rc > 0
    }

    fn check(component: &CStr) {
        for file in FILES {
            for fs in FILESYSTEMS {
                assert_eq!(
                    is_gitfile(component, file, fs),
                    c_is_gitfile(component, file, fs),
                    "{component:?}, {file:?}, {fs:?}"
                );
            }
        }
    }

    /// The spellings of each file, near misses and the degenerate components.
    fn corpus() -> Vec<CString> {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for (name, short, hash) in [
            ("gitignore", "gitign", "gi250a"),
            ("gitmodules", "gitmod", "gi7eba"),
            ("gitattributes", "gitatt", "gi7d29"),
        ] {
            let upper = name.to_uppercase();
            for base in [
                name.to_owned(),
                upper.clone(),
                format!("{}{}", &upper[..3], &name[3..]),
            ] {
                for suffix in ["", ".", " ", ". .", ":", ":stream", "x", "\\", "/"] {
                    out.push(format!(".{base}{suffix}").into_bytes());
                    out.push(format!("{base}{suffix}").into_bytes());
                }
            }
            for prefix in [short, hash, &short.to_uppercase(), &hash.to_uppercase()] {
                for tail in [
                    "~1", "~4", "~5", "~9", "~0", "~", "~1 .", "~1:x", "~12", "~1x", "~a",
                ] {
                    out.push(format!("{prefix}{tail}").into_bytes());
                }
            }
            // HFS+ ignorable code points (ZWNJ, ZWJ, LRM, the BOM) between letters.
            for ignorable in ["\u{200c}", "\u{200d}", "\u{200e}", "\u{feff}", "\u{206a}"] {
                out.push(format!(".{ignorable}{name}").into_bytes());
                out.push(format!(".{}{ignorable}{}", &name[..3], &name[3..]).into_bytes());
                out.push(format!(".{name}{ignorable}").into_bytes());
            }
            out.push(format!(".{}", &name[..name.len() - 1]).into_bytes());
            out.push(format!(".{name}{name}").into_bytes());
        }
        for other in [
            "", ".", "..", ".git", ".GIT", "git~1", "a", "~1", "\u{e9}", "\u{ff}x", "\x7f",
        ] {
            out.push(other.as_bytes().to_vec());
        }
        out.push(vec![0xFF; 3]);
        out.push(vec![b'a'; 4096]);
        out.into_iter().map(|b| CString::new(b).unwrap()).collect()
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn corpus_matches_c() {
        for component in corpus() {
            check(&component);
        }
    }

    /// Short names of the bytes the rules branch on, from a seeded generator; a stride
    /// keeps `BorrowSanitizer`'s run short.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn seeded_short_names_match_c() {
        const ALPHABET: &[u8] = b".~ :\\/gitGITmodulesattr1249aAeEbB\xc2\xa0\xe2\x80\x8c";
        let rounds = if cfg!(bsan) { 4_000 / 97 } else { 4_000 };
        let mut seed: u64 = 0x5EED_1E55;
        for _ in 0..rounds {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let len = (seed % 16) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|i| ALPHABET[(usize::try_from(seed >> (i * 4)).unwrap() ^ i) % ALPHABET.len()])
                .collect();
            check(&CString::new(bytes).unwrap());
        }
    }
}
