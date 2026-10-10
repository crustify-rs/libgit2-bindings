//! Safe wrappers over `include/git2/sys/path.h`.
//!
//! A [`GitFile`] names one of the git-specific files `git_path_is_gitfile` checks a
//! path component against: `.gitignore`, `.gitmodules` or `.gitattributes`, in the
//! forms filesystems may spell them. libgit2 indexes a table of their names with the
//! raw value and refuses every other value ("invalid gitfile for path validation"),
//! so a raw value is converted fallibly ([`GitFile::from_raw`]) and never transmuted.
//!
//! libgit2 compares a path component with a git-specific file name (`.gitmodules` and
//! friends) the way the filesystem it protects would: NTFS also matches 8.3 short names
//! and trailing dots and spaces, HFS+ ignores Unicode code points it treats as
//! ignorable. [`Filesystem`] (`git_path_fs`) chooses which rules apply, and
//! [`is_gitfile`] applies them to a path component.

use core::ffi::CStr;
use core::fmt;

use crate::ffi;

/// A git-specific file whose name `git_path_is_gitfile` recognises, in any spelling a
/// filesystem treats as that name.
///
/// [`from_raw`](Self::from_raw) gives `None` for every value the header does not name,
/// all of which `git_path_is_gitfile` refuses.
///
/// Wraps: git_path_gitfile
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum GitFile {
    /// `.gitignore`.
    Gitignore = ffi::GIT_PATH_GITFILE_GITIGNORE,
    /// `.gitmodules`.
    Gitmodules = ffi::GIT_PATH_GITFILE_GITMODULES,
    /// `.gitattributes`.
    Gitattributes = ffi::GIT_PATH_GITFILE_GITATTRIBUTES,
}

impl GitFile {
    /// The variant whose C value is `raw`; `None` for every value the header does not
    /// name.
    pub const fn from_raw(raw: ffi::git_path_gitfile) -> Option<Self> {
        match raw {
            ffi::GIT_PATH_GITFILE_GITIGNORE => Some(Self::Gitignore),
            ffi::GIT_PATH_GITFILE_GITMODULES => Some(Self::Gitmodules),
            ffi::GIT_PATH_GITFILE_GITATTRIBUTES => Some(Self::Gitattributes),
            _ => None,
        }
    }

    /// The C value, as `git_path_is_gitfile` takes it.
    pub const fn raw(self) -> ffi::git_path_gitfile {
        self as ffi::git_path_gitfile
    }
}

impl TryFrom<ffi::git_path_gitfile> for GitFile {
    type Error = UnnamedGitFile;

    fn try_from(raw: ffi::git_path_gitfile) -> Result<Self, UnnamedGitFile> {
        Self::from_raw(raw).ok_or(UnnamedGitFile(raw))
    }
}

impl From<GitFile> for ffi::git_path_gitfile {
    fn from(value: GitFile) -> ffi::git_path_gitfile {
        value.raw()
    }
}

/// A `git_path_gitfile` value that libgit2's header does not name, which
/// `git_path_is_gitfile` refuses. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedGitFile(ffi::git_path_gitfile);

impl UnnamedGitFile {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_path_gitfile {
        self.0
    }
}

impl fmt::Display for UnnamedGitFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unnamed git file {}", self.0)
    }
}

impl std::error::Error for UnnamedGitFile {}

/// Which filesystem's name-matching rules a path check applies (`git_path_fs`).
///
/// `git_path_is_gitfile` refuses any value without a variant, failing with
/// `GIT_ERROR_OS`, "invalid filesystem for path validation: " and the text of whatever
/// `errno` held; [`from_raw`](Self::from_raw) gives `None` for exactly those.
///
/// Wraps: git_path_fs
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Filesystem {
    /// Both NTFS's and HFS+'s rules: a name matches if either filesystem would treat it
    /// as the file.
    Generic = ffi::GIT_PATH_FS_GENERIC,
    /// NTFS's rules only: 8.3 short names (`GITMOD~1`) and trailing dots and spaces.
    Ntfs = ffi::GIT_PATH_FS_NTFS,
    /// HFS+'s rules only: case folding and ignorable Unicode code points.
    Hfs = ffi::GIT_PATH_FS_HFS,
}

impl Filesystem {
    /// The variant whose C value is `raw`; `None` for any value libgit2's header does not
    /// name.
    pub const fn from_raw(raw: ffi::git_path_fs) -> Option<Self> {
        match raw {
            ffi::GIT_PATH_FS_GENERIC => Some(Self::Generic),
            ffi::GIT_PATH_FS_NTFS => Some(Self::Ntfs),
            ffi::GIT_PATH_FS_HFS => Some(Self::Hfs),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take it.
    pub const fn raw(self) -> ffi::git_path_fs {
        self as ffi::git_path_fs
    }
}

impl TryFrom<ffi::git_path_fs> for Filesystem {
    type Error = UnnamedFilesystem;

    fn try_from(raw: ffi::git_path_fs) -> Result<Self, UnnamedFilesystem> {
        Self::from_raw(raw).ok_or(UnnamedFilesystem(raw))
    }
}

impl From<Filesystem> for ffi::git_path_fs {
    fn from(value: Filesystem) -> ffi::git_path_fs {
        value.raw()
    }
}

/// A `git_path_fs` value that libgit2's header does not name, and its path checks
/// refuse. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedFilesystem(ffi::git_path_fs);

impl UnnamedFilesystem {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_path_fs {
        self.0
    }
}

impl fmt::Display for UnnamedFilesystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 filesystem {} is not named by git2/sys/path.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedFilesystem {}

/// Whether the path component `component` names `file` on `fs`: the file's own name
/// (`.gitmodules`), or any spelling that filesystem resolves to it, such as NTFS's 8.3
/// short names (`GITMOD~1`, `gi7eba~1`) and trailing dots and spaces, or HFS+'s case
/// folding and ignorable Unicode code points. [`Filesystem::Generic`] matches either.
///
/// `component` is one component, without separators; it is NUL-terminated because
/// libgit2's NTFS rules read up to its NUL whatever length they are given.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::repository::sys::path::{Filesystem, GitFile, is_gitfile};
///
/// assert!(is_gitfile(c"GITMOD~1", GitFile::Gitmodules, Filesystem::Ntfs));
/// assert!(!is_gitfile(c"GITMOD~1", GitFile::Gitmodules, Filesystem::Hfs));
/// assert!(is_gitfile(c".gitignore", GitFile::Gitignore, Filesystem::Generic));
/// ```
///
/// Wraps: git_path_is_gitfile
pub fn is_gitfile(component: &CStr, file: GitFile, fs: Filesystem) -> bool {
    let len = component.count_bytes();
    // SAFETY: `component` is NUL-terminated and holds `len` bytes before its NUL, so
    // both the length-bounded HFS+ rules and the NUL-bounded NTFS ones read within it,
    // during the call only. `file` and `fs` are named values, which libgit2 accepts
    // without touching the error state, so no runtime pin is needed.
    let rc = unsafe { ffi::git_path_is_gitfile(component.as_ptr(), len, file.raw(), fs.raw()) };
    debug_assert!(
        rc >= 0,
        "git_path_is_gitfile refused a named file or filesystem"
    );
    rc > 0
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn named_values_round_trip() {
        for file in [
            GitFile::Gitignore,
            GitFile::Gitmodules,
            GitFile::Gitattributes,
        ] {
            assert_eq!(GitFile::from_raw(file.raw()), Some(file));
            assert_eq!(GitFile::try_from(file.raw()), Ok(file));
            assert_eq!(ffi::git_path_gitfile::from(file), file.raw());
        }
        // The order `path.c`'s table of names relies on.
        assert_eq!(GitFile::Gitignore.raw(), 0);
        assert_eq!(GitFile::Gitmodules.raw(), 1);
        assert_eq!(GitFile::Gitattributes.raw(), 2);
    }

    #[test]
    fn unnamed_values_are_refused() {
        for raw in [3, 4, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
            assert_eq!(GitFile::from_raw(raw), None);
            let err = GitFile::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(err.to_string(), format!("unnamed git file {raw}"));
        }
    }

    #[test]
    fn option_niche_holds() {
        assert_eq!(size_of::<Option<GitFile>>(), size_of::<GitFile>());
        assert_eq!(size_of::<GitFile>(), size_of::<ffi::git_path_gitfile>());
    }

    mod filesystem {
        use super::super::*;

        #[test]
        fn conversions_round_trip_the_named_values() {
            for fs in [Filesystem::Generic, Filesystem::Ntfs, Filesystem::Hfs] {
                let raw = ffi::git_path_fs::from(fs);
                assert_eq!(raw, fs.raw());
                assert_eq!(Filesystem::from_raw(raw), Some(fs));
                assert_eq!(Filesystem::try_from(raw), Ok(fs));
            }
            assert_eq!(Filesystem::Generic.raw(), 0);
            assert_eq!(Filesystem::Hfs.raw(), 2);
        }

        #[test]
        fn unnamed_values_are_kept_and_described() {
            for raw in [3, 4, 0x8000_0000, u32::MAX] {
                let err = Filesystem::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert_eq!(
                    err.to_string(),
                    format!("libgit2 filesystem {raw} is not named by git2/sys/path.h")
                );
                let _: &dyn std::error::Error = &err;
            }
        }
    }
}
