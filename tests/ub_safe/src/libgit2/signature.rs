//! UB tests for `git2/signature.h`'s repository defaults: `Signature::default`,
//! `Signature::default_from_env`, `default_author_from_env` and
//! `default_committer_from_env` (`git_signature_default`,
//! `git_signature_default_from_env`).
//!
//! Each builds owned signatures from a snapshot of the repository's configuration and
//! from the environment: they outlive the repository and its configuration, which are
//! dropped before they are read (`ASan`), and every failure releases what libgit2 built.
//! In particular, the pair whose committer fails after its author is built, on which
//! libgit2 itself leaks the author's strings, must leak nothing through the wrapper
//! (`LSan`). Identities of up to a megabyte are read and copied (`ASan`). Each case runs in
//! a child process whose environment it sets, its other configuration levels pointed at
//! an empty directory, on one thread: every call reads the local time zone through
//! glibc's `mktime`, whose `tzset` `TSan` reports against any concurrent caller, so no
//! test runs them on threads. Every test calls libgit2, so Miri skips them all.

mod types;
