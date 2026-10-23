//! UB tests for `git2/sys/path.h`: the enums `GitFile` and, in `mod filesystem`,
//! `Filesystem`.
//!
//! `GitFile` is a fieldless enum over C's `git_path_gitfile`, whose raw value libgit2
//! uses as an index into a table of three names. The only safe way in from a raw value
//! is `from_raw` / `try_from`, which must reject every value without a variant rather
//! than produce an invalid discriminant (which Miri and UBSan would flag on the next
//! read); an `as` cast to it does not compile (`compile_fail/`). None of them calls
//! libgit2, so they also run under Miri.

use std::collections::HashSet;
use std::hint::black_box;
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::repository::sys::path::GitFile;

/// Every value around the declared ones, then a coarse stride across the whole type.
fn probes() -> impl Iterator<Item = ffi::git_path_gitfile> {
    (0..=300)
        .chain((0..=ffi::git_path_gitfile::MAX).step_by(1 << 26))
        .chain([ffi::git_path_gitfile::MAX - 1, ffi::git_path_gitfile::MAX])
}

#[test]
fn from_raw_yields_only_valid_files() {
    let mut seen = HashSet::new();
    for raw in probes() {
        // `black_box` keeps the conversion from being folded at compile time.
        if let Some(file) = GitFile::from_raw(black_box(raw)) {
            // Reading the discriminant back validates it.
            assert_eq!(file.raw(), raw);
            assert_eq!(ffi::git_path_gitfile::from(file), raw);
            assert!(!format!("{file:?}").is_empty());
            seen.insert(file);
        } else {
            let err = GitFile::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert!(!err.to_string().is_empty());
        }
    }
    assert_eq!(seen.len(), 3);
}

#[test]
fn option_niche_holds_for_every_value() {
    for raw in probes() {
        let file = black_box(GitFile::from_raw(raw));
        assert_eq!(file.is_some(), raw < 3);
        assert_eq!(file.map(GitFile::raw), file.is_some().then_some(raw));
    }
}

#[test]
fn files_cross_threads() {
    let files = [
        GitFile::Gitignore,
        GitFile::Gitmodules,
        GitFile::Gitattributes,
    ];
    let handles: Vec<_> = files
        .into_iter()
        .map(|f| thread::spawn(move || GitFile::from_raw(f.raw())))
        .collect();
    for (h, f) in handles.into_iter().zip(files) {
        assert_eq!(h.join().unwrap(), Some(f));
    }
}

mod filesystem {
    //! [`Filesystem`] (`git_path_fs`): its raw conversions yield only valid variants
    //! (Miri checks the enum's validity), and it crosses threads.

    use libgit2::ffi;
    use libgit2::libgit2::repository::sys::path::Filesystem;

    #[test]
    fn every_raw_value_converts_to_a_variant_or_none() {
        let edges = [u32::MAX, u32::MAX - 1, 0x8000_0000, 0x7fff_ffff];
        for raw in (0..=300).chain(edges) {
            match Filesystem::from_raw(raw) {
                Some(fs) => {
                    assert_eq!(fs.raw(), raw);
                    assert!(matches!(
                        fs,
                        Filesystem::Generic | Filesystem::Ntfs | Filesystem::Hfs
                    ));
                }
                None => assert!(raw > ffi::GIT_PATH_FS_HFS),
            }
            assert_eq!(Filesystem::try_from(raw).ok(), Filesystem::from_raw(raw));
        }
    }

    #[test]
    fn filesystems_cross_threads() {
        fn send_sync<T: Send + Sync + Copy>() {}
        send_sync::<Filesystem>();
        let fs = std::thread::spawn(|| Filesystem::from_raw(1))
            .join()
            .unwrap();
        assert_eq!(fs, Some(Filesystem::Ntfs));
    }
}

/// [`is_gitfile`] reads within its component: one cut from a longer buffer, empty,
/// long, non-UTF-8 and short-name-shaped components, under ASan; and from threads
/// before anything initialised libgit2, which it needs no runtime for.
mod is_gitfile {
    use std::ffi::{CStr, CString};
    use std::thread;

    use libgit2::libgit2::repository::sys::path::{Filesystem, GitFile, is_gitfile};

    use crate::fixture::isolated;

    const FILES: [GitFile; 3] = [
        GitFile::Gitignore,
        GitFile::Gitmodules,
        GitFile::Gitattributes,
    ];
    const FILESYSTEMS: [Filesystem; 3] = [Filesystem::Generic, Filesystem::Ntfs, Filesystem::Hfs];

    fn components() -> Vec<CString> {
        let mut out: Vec<CString> = [
            &b""[..],
            b".",
            b".gitmodules",
            b"GITMOD~1",
            b"gi7eba~1",
            b"gitmod~",
            b"gitmod",
            b"GITMOD~1 . :",
            b".gitmodules\xe2\x80\x8c",
            b"\xe2\x80",
            b"\xff\xfe",
            b"~",
            b"~1",
        ]
        .iter()
        .map(|b| CString::new(*b).unwrap())
        .collect();
        out.push(CString::new(vec![b'.'; 8192]).unwrap());
        out.push(CString::new(vec![b'~'; 9]).unwrap());
        out
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn reads_stop_at_the_nul() {
        // Bytes past the NUL that would match, if read.
        let buffer = b"gitmod\0~1\0.gitmodules";
        let cut = CStr::from_bytes_until_nul(buffer).unwrap();
        for file in FILES {
            for fs in FILESYSTEMS {
                assert!(!is_gitfile(cut, file, fs));
                for component in components() {
                    let _ = is_gitfile(&component, file, fs);
                }
            }
        }
        // The heap copies end right at their NUL, for ASan to see any read past it.
        for component in components() {
            let boxed: Box<CStr> = component.into_boxed_c_str();
            for fs in FILESYSTEMS {
                let _ = is_gitfile(&boxed, GitFile::Gitmodules, fs);
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn threads_check_before_initialisation() {
        isolated(
            concat!(module_path!(), "::threads_check_before_initialisation"),
            || {
                thread::scope(|s| {
                    for fs in FILESYSTEMS {
                        s.spawn(move || {
                            for component in components() {
                                for file in FILES {
                                    let _ = is_gitfile(&component, file, fs);
                                }
                            }
                            assert!(is_gitfile(
                                c"GITMOD~1",
                                GitFile::Gitmodules,
                                Filesystem::Ntfs
                            ));
                        });
                    }
                });
            },
        );
    }
}
