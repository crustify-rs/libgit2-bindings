//! Safe wrappers over `include/git2/sys/path.h`.
//!
//! A [`GitFile`] names one of the git-specific files `git_path_is_gitfile` checks a
//! path component against: `.gitignore`, `.gitmodules` or `.gitattributes`, in the
//! forms filesystems may spell them. libgit2 indexes a table of their names with the
//! raw value and refuses every other value ("invalid gitfile for path validation"),
//! so a raw value is converted fallibly ([`GitFile::from_raw`]) and never transmuted.

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
}
