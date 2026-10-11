//! UB tests for `git2/signature.h`'s repository defaults: `Signature::default`,
//! `Signature::default_from_env`, `default_author_from_env` and
//! `default_committer_from_env` (`git_signature_default`,
//! `git_signature_default_from_env`).
//!
//! Each builds owned signatures from a snapshot of the repository's configuration and
//! from the environment: they outlive the repository and its configuration, which are
//! dropped before they are read (ASan), and every failure releases what libgit2 built.
//! In particular, the pair whose committer fails after its author is built, on which
//! libgit2 itself leaks the author's strings, must leak nothing through the wrapper
//! (LSan). Identities of up to a megabyte are read and copied (ASan). Each case runs in
//! a child process whose environment it sets, its other configuration levels pointed at
//! an empty directory, on one thread: every call reads the local time zone through
//! glibc's `mktime`, whose `tzset` TSan reports against any concurrent caller, so no
//! test runs them on threads. Every test calls libgit2, so Miri skips them all.

use std::ffi::{CStr, CString};
use std::path::PathBuf;
use std::process::Command;

use libgit2::libgit2::core::types::{Signature, SignatureBox};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
use libgit2::libgit2::util::errors::ErrorCode;

use crate::fixture::signature::isolate_config_levels;

const CHILD: &str = "CRUSTIFY_UB_SAFE_CHILD";

/// Runs `body` alone in a child process whose git identity variables are `env` alone.
fn isolated(test: &str, env: &[(&str, &str)], body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name);
    for who in ["AUTHOR", "COMMITTER"] {
        for what in ["NAME", "EMAIL", "DATE"] {
            command.env_remove(format!("GIT_{who}_{what}"));
        }
    }
    command.env_remove("EMAIL");
    command.envs(env.iter().copied());
    let out = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A fresh bare repository whose configuration holds `config`, the other levels
/// searching an empty directory; removed on drop.
struct TempRepo {
    path: PathBuf,
    repo: Option<RepositoryBox>,
}

impl TempRepo {
    fn new(tag: &str, config: &str) -> Self {
        crate::fixture::init();
        let path = std::env::temp_dir().join(format!(
            "crustify-signature-ub-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("levels")).unwrap();
        isolate_config_levels(&path.join("levels"));
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

    fn repo(&self) -> &RepositoryBox {
        self.repo.as_ref().unwrap()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        drop(self.repo.take());
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn strings(sig: &SignatureBox) -> (usize, usize) {
    let s = sig.as_ref();
    (
        s.name().map_or(0, CStr::count_bytes),
        s.email().map_or(0, CStr::count_bytes),
    )
}

/// Every form's signatures, built from a large identity, outlive the repository and
/// its configuration, and copy cleanly.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn signatures_outlive_their_repository() {
    isolated(
        concat!(module_path!(), "::signatures_outlive_their_repository"),
        &[("GIT_COMMITTER_DATE", "1700000000 -0130")],
        || {
            let name = "n".repeat(1 << 20);
            let config = format!("[user]\n\tname = {name}\n\temail = e@example.com\n");
            let temp = TempRepo::new("outlive", &config);
            let r = temp.repo().as_ref();
            let default = Signature::default(r).unwrap();
            let author = Signature::default_author_from_env(r).unwrap();
            let committer = Signature::default_committer_from_env(r).unwrap();
            let (pair_author, pair_committer) = Signature::default_from_env(r).unwrap();
            drop(temp);
            for sig in [&default, &author, &committer, &pair_author, &pair_committer] {
                assert_eq!(strings(sig), (1 << 20, 13));
                let copy = sig.clone();
                assert_eq!(copy.as_ref(), sig.as_ref());
            }
            assert_eq!(committer.as_ref().when().seconds(), 1_700_000_000);
            assert_eq!(pair_committer.as_ref().when().offset_minutes(), -90);
        },
    );
}

/// Failures release what libgit2 built, the pair whose committer fails after its
/// author included, on which libgit2 alone would leak the author (LSan).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failures_release_everything() {
    isolated(
        concat!(module_path!(), "::failures_release_everything"),
        &[
            ("GIT_AUTHOR_NAME", "Author"),
            ("GIT_AUTHOR_EMAIL", "author@example.com"),
            ("GIT_COMMITTER_DATE", "no date at all"),
        ],
        || {
            let temp = TempRepo::new("failures", "[user]\n\tname = Config User\n");
            let r = temp.repo().as_ref();
            for _ in 0..3 {
                let e = Signature::default(r).unwrap_err();
                assert_eq!(e.code(), Some(ErrorCode::NotFound));
                assert!(Signature::default_author_from_env(r).is_ok());
                assert!(Signature::default_committer_from_env(r).is_err());
                assert!(Signature::default_from_env(r).is_err());
            }
        },
    );
}
