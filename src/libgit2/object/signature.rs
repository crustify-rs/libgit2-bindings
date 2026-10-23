//! Safe wrappers over `include/git2/signature.h`.
//!
//! The signature type, its owner and the constructors that need no repository live
//! with the other shared types, in [`types`](crate::libgit2::core::types#signatures):
//! [`Signature`], [`SignatureBox`], [`Signature::new`], [`Signature::now`],
//! [`Signature::parse`]. This module adds the constructors that read an identity from
//! a repository: [`Signature::default`] from its configuration (`user.name` and
//! `user.email`, at the current time), and [`Signature::default_from_env`],
//! [`Signature::default_author_from_env`] and
//! [`Signature::default_committer_from_env`] from git's environment variables first.
//! Each reads a snapshot of the repository's configuration, every level of it, and
//! builds a signature of its own through `git_signature_now` or `git_signature_new`,
//! so the result is a [`SignatureBox`] that outlives the repository.
//!
//! # The environment
//!
//! `git_signature_default_from_env` reads `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL` and
//! `GIT_AUTHOR_DATE` for the author, the `GIT_COMMITTER_*` ones for the committer, and
//! `EMAIL` when neither the variable nor `user.email` gives an email. Its C signature
//! takes an optional slot for each, which Rust spells as three functions: both, the
//! author alone, the committer alone. Asked for both, it reads the current time once,
//! so a pair without dates shares one time.
//!
//! libgit2 releases a built author with `git__free` rather than `git_signature_free`
//! when the committer then fails, leaking the author's name and email.
//! [`Signature::default_from_env`] first asks for the committer alone, and asks for
//! both only once that succeeds; see there.
//!
//! The variables are read with `getenv`, which races with a concurrent `setenv`:
//! Rust's `std::env::set_var` is `unsafe` for that reason, and its caller must exclude
//! these calls as it excludes any other reader of the environment.

use core::ffi::c_int;
use core::ptr;

use crate::ffi;
use crate::libgit2::core::types::{Signature, SignatureBox};
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::sys::errors::set_str;

impl Signature {
    /// A signature of the identity `repo`'s configuration names, `user.name` and
    /// `user.email` (read from a snapshot of every level), at the current time in the
    /// local timezone, as [`Signature::now`] builds it.
    ///
    /// Concurrent calls are sound, and race as [`Signature::now`] does under
    /// `ThreadSanitizer`, on glibc's copy of `TZ`.
    ///
    /// Wraps: `git_signature_default`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`](crate::libgit2::util::errors::ErrorCode::NotFound)
    /// (class `GIT_ERROR_CONFIG`, "config value '…' was not found") when either is not
    /// set, with libgit2's error when the configuration cannot be read, and as
    /// [`Signature::now`] fails for a value it refuses (an empty one, or one holding an
    /// angle bracket).
    pub fn default(repo: RepositoryRef<'_>) -> Result<SignatureBox, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success; `repo` is a live
        // repository on this thread, whose configuration libgit2 loads and snapshots
        // through this shared handle, as `RepositoryRef` allows, keeping nothing of it.
        let rc = unsafe { ffi::git_signature_default(&raw mut out, repo.as_ptr().cast_mut()) };
        // SAFETY: NULL on failure, or a fresh signature from `git_signature_now` the
        // caller owns alone, released by `git_signature_free` as `SignatureBox` does.
        adopted(rc, unsafe { SignatureBox::from_c(out) })
    }

    /// The author and the committer git would record in `repo`, from the environment
    /// first: for the author, the name in `GIT_AUTHOR_NAME`, else `user.name`; the
    /// email in `GIT_AUTHOR_EMAIL`, else `user.email`, else `EMAIL`; and the time in
    /// `GIT_AUTHOR_DATE` (any format git's date parser reads, a zone included), else
    /// the current time in the local timezone. The committer likewise, from the
    /// `GIT_COMMITTER_*` variables. Without dates, both share one current time.
    ///
    /// libgit2 leaks the author's strings when the committer fails after the author
    /// was built. So the committer is first built alone, and dropped: only once that
    /// succeeds does libgit2 build both, together. When it fails, the author is built
    /// alone to tell whose failure libgit2 would report, and the committer's error is
    /// then set again as the thread's last error, as libgit2 would leave it. A
    /// configuration or environment changed between the two calls can still reach the
    /// leak.
    ///
    /// Concurrent calls are sound, and race as [`Signature::now`] does under
    /// `ThreadSanitizer`, on glibc's copy of `TZ`.
    ///
    /// Wraps: `git_signature_default_from_env`
    ///
    /// # Errors
    ///
    /// Fails as [`default_author_from_env`](Self::default_author_from_env) fails for the
    /// author, else as [`default_committer_from_env`](Self::default_committer_from_env)
    /// fails for the committer.
    pub fn default_from_env(
        repo: RepositoryRef<'_>,
    ) -> Result<(SignatureBox, SignatureBox), RepositoryError> {
        if let Err(committer) = Self::default_committer_from_env(repo) {
            drop(Self::default_author_from_env(repo)?);
            if let Some(class) = committer.class() {
                // Restored verbatim; a message that cannot be stored leaves libgit2's
                // out-of-memory error instead, which is as true.
                let _ = set_str(class, committer.message());
            }
            return Err(committer);
        }
        let (mut author, mut committer) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: `author` and `committer` are writable slots, set only on success;
        // `repo` a live repository on this thread, whose configuration libgit2 loads and
        // snapshots through this shared handle, keeping nothing of it.
        let rc = unsafe {
            ffi::git_signature_default_from_env(
                &raw mut author,
                &raw mut committer,
                repo.as_ptr().cast_mut(),
            )
        };
        // SAFETY: on success each slot holds a fresh signature from `git_signature_new`
        // the caller owns alone, released by `git_signature_free` as `SignatureBox`
        // does; on failure both stay NULL.
        let value2 = unsafe { SignatureBox::from_c(author) };
        // SAFETY: as above.
        let (author, committer) = unsafe { (value2, SignatureBox::from_c(committer)) };
        Ok((adopted(rc, author)?, adopted(rc, committer)?))
    }

    /// The author git would record in `repo`, from the environment first:
    /// [`default_from_env`](Self::default_from_env)'s author, built alone.
    ///
    /// Wraps: `git_signature_default_from_env`
    ///
    /// # Errors
    ///
    /// Fails with [`ErrorCode::NotFound`](crate::libgit2::util::errors::ErrorCode::NotFound)
    /// (class `GIT_ERROR_CONFIG`) when neither `GIT_AUTHOR_NAME` nor `user.name` is
    /// set, or none of `GIT_AUTHOR_EMAIL`, `user.email` and `EMAIL`; with libgit2's
    /// error when the configuration cannot be read; as [`Signature::new`] fails for a
    /// value it refuses; and with `-1` when `GIT_AUTHOR_DATE` does not parse, for which
    /// libgit2 sets no error of its own: the error is whatever libgit2 left last, such
    /// as a configuration file it did not find.
    pub fn default_author_from_env(
        repo: RepositoryRef<'_>,
    ) -> Result<SignatureBox, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, set only on success, and the committer's
        // slot NULL, which libgit2 skips; `repo` a live repository on this thread, whose
        // configuration libgit2 loads and snapshots through this shared handle.
        let rc = unsafe {
            ffi::git_signature_default_from_env(
                &raw mut out,
                ptr::null_mut(),
                repo.as_ptr().cast_mut(),
            )
        };
        // SAFETY: NULL on failure, or a fresh signature from `git_signature_new` the
        // caller owns alone, released by `git_signature_free` as `SignatureBox` does.
        adopted(rc, unsafe { SignatureBox::from_c(out) })
    }

    /// The committer git would record in `repo`, from the environment first:
    /// [`default_from_env`](Self::default_from_env)'s committer, built alone. Fails as
    /// [`default_author_from_env`](Self::default_author_from_env) does, from the
    /// `GIT_COMMITTER_*` variables.
    ///
    /// Wraps: `git_signature_default_from_env`
    ///
    /// # Errors
    ///
    /// Fails as [`default_author_from_env`](Self::default_author_from_env) does, from the
    /// `GIT_COMMITTER_*` variables.
    pub fn default_committer_from_env(
        repo: RepositoryRef<'_>,
    ) -> Result<SignatureBox, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: as in `default_author_from_env`, with the author's slot NULL.
        let rc = unsafe {
            ffi::git_signature_default_from_env(
                ptr::null_mut(),
                &raw mut out,
                repo.as_ptr().cast_mut(),
            )
        };
        // SAFETY: as in `default_author_from_env`.
        adopted(rc, unsafe { SignatureBox::from_c(out) })
    }
}

/// The signature a constructor returning `rc` handed out, or its failure.
fn adopted(rc: c_int, signature: Option<SignatureBox>) -> Result<SignatureBox, RepositoryError> {
    if rc < 0 {
        return Err(RepositoryError::from_libgit2(rc));
    }
    Ok(signature.expect("a signature constructor succeeded without a signature"))
}

#[cfg(test)]
mod unit_tests {
    use core::ffi::{CStr, c_void};
    use core::ptr::NonNull;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::AtomicIsize;
    use std::sync::atomic::Ordering::SeqCst;

    use super::*;
    use crate::libgit2::core::common::{SearchPathLevel, set_search_path};
    use crate::libgit2::repository::repository::{Repository, RepositoryBox};
    use crate::libgit2::util::errors::{ErrorClass, ErrorCode, last};
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2,
    /// with git's identity variables unset but for `env`; with `tolerate_leaks`,
    /// `LeakSanitizer` is off there.
    fn isolated(test: &str, env: &[(&str, &str)], tolerate_leaks: bool, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name);
        for var in ["NAME", "EMAIL", "DATE"] {
            child.env_remove(format!("GIT_AUTHOR_{var}"));
            child.env_remove(format!("GIT_COMMITTER_{var}"));
        }
        child.env_remove("EMAIL");
        child.envs(env.iter().copied());
        if tolerate_leaks {
            let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
            let separator = if inherited.is_empty() { "" } else { ":" };
            child.env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            );
        }
        let out = child.output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Blocks libgit2's allocator holds: allocated, less freed.
    static LIVE: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, counting the blocks it holds in [`LIVE`].
    struct Counting;

    // SAFETY: forwards to libc's `malloc` family, which `Alloc` allows.
    unsafe impl Alloc for Counting {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            // SAFETY: plain libc allocation.
            let p = NonNull::new(unsafe { malloc(len) }.cast());
            LIVE.fetch_add(isize::from(p.is_some()), SeqCst);
            p
        }

        unsafe fn realloc(
            p: Option<NonNull<u8>>,
            len: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            let fresh = p.is_none();
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            let q = NonNull::new(unsafe { realloc(p, len) }.cast());
            LIVE.fetch_add(isize::from(fresh && q.is_some()), SeqCst);
            q
        }

        unsafe fn free(p: NonNull<u8>) {
            LIVE.fetch_sub(1, SeqCst);
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Installs [`Counting`] as libgit2's allocator, before anything is allocated.
    fn install_counting() {
        let allocator = Allocator::new::<Counting>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// A fresh bare repository whose configuration holds `config`, and no other level
    /// of configuration: the system, global and XDG levels search an empty directory.
    struct TempRepo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl TempRepo {
        fn new(tag: &str, config: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "crustify-signature-unit-{}-{tag}",
                std::process::id()
            ));
            drop(std::fs::remove_dir_all(&path));
            std::fs::create_dir_all(path.join("levels")).unwrap();
            let levels = CString::new(path.join("levels").to_str().unwrap()).unwrap();
            for level in [
                SearchPathLevel::System,
                SearchPathLevel::Global,
                SearchPathLevel::Xdg,
            ] {
                // SAFETY: the test runs alone in its child, on one thread.
                unsafe { set_search_path(level, Some(&levels)) }.unwrap();
            }
            let repo_path = path.join("repo.git");
            let c_path = CString::new(repo_path.to_str().unwrap()).unwrap();
            drop(Repository::init(&c_path, true).unwrap());
            let mut file = std::fs::read_to_string(repo_path.join("config")).unwrap();
            file.push_str(config);
            std::fs::write(repo_path.join("config"), file).unwrap();
            Self {
                path,
                repo: Some(Repository::open(&c_path).unwrap()),
            }
        }

        fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.path));
        }
    }

    const USER: &str = "[user]\n\tname = Config User\n\temail = config@example.com\n";

    /// The corrected divergence: when the author is built and the committer then fails,
    /// `git_signature_default_from_env` frees the author with `git__free`, keeping its
    /// two strings; `default_from_env` reports the committer's failure, with the error
    /// libgit2 leaves, and leaves no block behind. The child runs with `LeakSanitizer`
    /// off, as the C call leaks; the counting allocator stands in for it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_failing_committer_leaks_no_author() {
        isolated(
            concat!(module_path!(), "::a_failing_committer_leaks_no_author"),
            &[("GIT_COMMITTER_DATE", "not a date")],
            true,
            || {
                install_counting();
                let temp = TempRepo::new("leak", USER);
                let repo = temp.repo();
                // Warm libgit2's lazily built state (the configuration, the thread's
                // error) so each count below is the call's alone.
                drop(Signature::default_from_env(repo));

                let before = LIVE.load(SeqCst);
                let (mut author, mut committer) = (ptr::null_mut(), ptr::null_mut());
                // SAFETY: two writable slots and a live repository on this thread.
                let rc = unsafe {
                    ffi::git_signature_default_from_env(
                        &raw mut author,
                        &raw mut committer,
                        repo.as_ptr().cast_mut(),
                    )
                };
                assert!(rc < 0 && author.is_null() && committer.is_null());
                let c_error = last();
                // libgit2's leak: the author's name and email.
                assert_eq!(LIVE.load(SeqCst) - before, 2);

                let before = LIVE.load(SeqCst);
                let e = Signature::default_from_env(repo).unwrap_err();
                assert_eq!(LIVE.load(SeqCst), before);
                assert_eq!(e.raw_code(), rc);
                assert_eq!(e.error(), &c_error);
                assert_eq!(last(), c_error);

                // Each alone: the author builds, the committer fails as above.
                assert!(Signature::default_author_from_env(repo).is_ok());
                let alone = Signature::default_committer_from_env(repo).unwrap_err();
                assert_eq!(alone, e);
            },
        );
    }

    /// When both fail, the author's failure is the one reported, as libgit2 reports it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn the_author_fails_first() {
        isolated(
            concat!(module_path!(), "::the_author_fails_first"),
            &[
                ("GIT_AUTHOR_DATE", "also not a date"),
                ("GIT_COMMITTER_EMAIL", "<bracketed>"),
            ],
            false,
            || {
                let temp = TempRepo::new("author-first", USER);
                let repo = temp.repo();
                let e = Signature::default_from_env(repo).unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::Generic));
                let author = Signature::default_author_from_env(repo).unwrap_err();
                assert_eq!(e, author);
                // libgit2 sets no error for a date it cannot parse.
                assert_ne!(e.class(), Some(ErrorClass::Invalid));
                // The committer's email holds angle brackets, which libgit2 refuses.
                let committer = Signature::default_committer_from_env(repo).unwrap_err();
                assert_eq!(committer.class(), Some(ErrorClass::Invalid));
                assert_ne!(committer, author);
            },
        );
    }

    /// Without dates, the author and committer of one call share its time; the
    /// variables win over the configuration, and `EMAIL` stands in for a missing email.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn pairs_share_their_time() {
        isolated(
            concat!(module_path!(), "::pairs_share_their_time"),
            &[
                ("GIT_AUTHOR_NAME", "Env Author"),
                ("EMAIL", "fallback@example.com"),
            ],
            false,
            || {
                let temp = TempRepo::new("pairs", "[user]\n\tname = Config User\n");
                let repo = temp.repo();
                let (author, committer) = Signature::default_from_env(repo).unwrap();
                let (a, c) = (author.as_ref(), committer.as_ref());
                assert_eq!(a.name(), Some(c"Env Author"));
                assert_eq!(c.name(), Some(c"Config User"));
                assert_eq!(a.email(), Some(c"fallback@example.com"));
                assert_eq!(c.email(), Some(c"fallback@example.com"));
                assert_eq!(a.when(), c.when());
                let e = Signature::default(repo).unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::NotFound));
                assert_eq!(e.message(), c"config value 'user.email' was not found");
            },
        );
    }
}
