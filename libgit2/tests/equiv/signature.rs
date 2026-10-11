//! Equivalence of `git2/signature.h`'s repository defaults, [`Signature::default`] and
//! [`Signature::default_from_env`] (with its author-only and committer-only forms),
//! with `git_signature_default` and `git_signature_default_from_env`.
//!
//! Each case runs in a child process of its own, whose environment holds the case's
//! `GIT_AUTHOR_*`, `GIT_COMMITTER_*` and `EMAIL` variables and nothing else of git's,
//! and whose system, global and XDG configuration levels search an empty directory, so
//! that only the repository's configuration (the case's `[user]` section) is read.
//! Both sides read one repository, each through its own handle, and call in turn:
//! `git_signature_default`, then `git_signature_default_from_env` for the author alone,
//! the committer alone, and both. Each call's outcome is compared: the signatures'
//! names, emails and time zones, and their times (exactly when the environment gives
//! them, within a few seconds of each other when they are the current time), or the
//! code, error class and message of the failure. A pair without dates must share one
//! time on both sides.
//!
//! The cases cover identities from the configuration alone, from the variables alone,
//! mixed, with `EMAIL` standing in for a missing email, dates in git's raw and ISO
//! formats, missing names and emails, values libgit2 refuses, and dates that do not
//! parse (for which libgit2 sets no error of its own, so both sides report the one it
//! left).
//!
//! Documented divergence, compared in its own child with LeakSanitizer off: when the
//! author is built and the committer then fails, C leaks the author's strings, where
//! [`Signature::default_from_env`] asks for the committer first and leaks nothing; the
//! failure and the error it leaves are the same. `signature.rs`'s unit tests count the
//! leaked blocks.
//!
//! [`Signature::default`]: libgit2::libgit2::core::types::Signature::default
//! [`Signature::default_from_env`]: libgit2::libgit2::core::types::Signature::default_from_env

use std::ffi::{CStr, CString, c_int};
use std::path::PathBuf;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::core::common::{SearchPathLevel, set_search_path};
use libgit2::libgit2::core::types::{Signature, SignatureBox};
use libgit2::libgit2::repository::repository::{Repository, RepositoryError};

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process whose git variables are `env` alone; with
/// `tolerate_leaks`, LeakSanitizer is off there.
fn isolated(test: &str, env: &[(&str, &str)], tolerate_leaks: bool, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
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
    if tolerate_leaks {
        let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
        let separator = if inherited.is_empty() { "" } else { ":" };
        command.env(
            "ASAN_OPTIONS",
            format!("{inherited}{separator}detect_leaks=0"),
        );
    }
    let out = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A signature as compared: name, email, seconds, offset, sign.
type Seen = (Vec<u8>, Vec<u8>, i64, i32, u8);

/// A call's outcome: the signatures it built, or its failure.
type Outcome = Result<Vec<Seen>, (c_int, c_int, String)>;

fn c_seen(sig: *mut ffi::git_signature) -> Seen {
    // SAFETY: a live signature the caller owns, with both strings (libgit2 built it
    // through `git_signature_new`), read, then released once.
    unsafe {
        let s = &*sig;
        let seen = (
            CStr::from_ptr(s.name).to_bytes().to_vec(),
            CStr::from_ptr(s.email).to_bytes().to_vec(),
            s.when.time,
            s.when.offset,
            s.when.sign as u8,
        );
        ffi::git_signature_free(sig);
        seen
    }
}

fn c_failed(rc: c_int) -> Outcome {
    // SAFETY: libgit2 is initialised; the last error and its message are live until
    // the next call on this thread, after the copy.
    unsafe {
        let e = ffi::git_error_last();
        Err((
            rc,
            (*e).klass,
            CStr::from_ptr((*e).message).to_string_lossy().into_owned(),
        ))
    }
}

fn rust_seen(sig: &SignatureBox) -> Seen {
    let s = sig.as_ref();
    let when = s.when();
    (
        s.name().unwrap().to_bytes().to_vec(),
        s.email().unwrap().to_bytes().to_vec(),
        when.seconds(),
        when.offset_minutes(),
        when.sign(),
    )
}

fn rust_failed(e: RepositoryError) -> Outcome {
    Err((
        e.raw_code(),
        e.raw_class(),
        e.message().to_string_lossy().into_owned(),
    ))
}

/// The four calls, C's: the default, then the author, the committer, and both.
fn c_calls(repo: *mut ffi::git_repository) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let mut out = ptr::null_mut();
    // SAFETY: a writable slot and the live repository.
    let rc = unsafe { ffi::git_signature_default(&mut out, repo) };
    outcomes.push(if rc < 0 {
        c_failed(rc)
    } else {
        Ok(vec![c_seen(out)])
    });
    for (author, committer) in [(true, false), (false, true), (true, true)] {
        let (mut a, mut c) = (ptr::null_mut(), ptr::null_mut());
        let slot = |wanted: bool, s: &mut *mut ffi::git_signature| {
            if wanted {
                ptr::from_mut(s)
            } else {
                ptr::null_mut()
            }
        };
        // SAFETY: writable slots (or NULL, not both) and the live repository.
        let rc = unsafe {
            ffi::git_signature_default_from_env(slot(author, &mut a), slot(committer, &mut c), repo)
        };
        outcomes.push(if rc < 0 {
            c_failed(rc)
        } else {
            Ok([a, c]
                .into_iter()
                .filter(|p| !p.is_null())
                .map(c_seen)
                .collect())
        });
    }
    outcomes
}

/// The four calls, Rust's.
fn rust_calls(repo: &libgit2::libgit2::repository::repository::RepositoryBox) -> Vec<Outcome> {
    let r = repo.as_ref();
    let one = |result: Result<SignatureBox, RepositoryError>| match result {
        Ok(sig) => Ok(vec![rust_seen(&sig)]),
        Err(e) => rust_failed(e),
    };
    vec![
        one(Signature::default(r)),
        one(Signature::default_author_from_env(r)),
        one(Signature::default_committer_from_env(r)),
        match Signature::default_from_env(r) {
            Ok((a, c)) => Ok(vec![rust_seen(&a), rust_seen(&c)]),
            Err(e) => rust_failed(e),
        },
    ]
}

/// Whether `rust` matches `c`, the current time allowed to have moved by a few seconds
/// when `exact` is not set.
fn same(rust: &Outcome, c: &Outcome, exact: bool) -> bool {
    match (rust, c) {
        (Ok(r), Ok(c)) => {
            r.len() == c.len()
                && r.iter().zip(c).all(|(r, c)| {
                    (&r.0, &r.1, r.3, r.4) == (&c.0, &c.1, c.3, c.4)
                        && if exact {
                            r.2 == c.2
                        } else {
                            (r.2 - c.2).abs() <= 5
                        }
                })
        }
        (r, c) => r == c,
    }
}

/// Opens a fresh repository whose configuration holds `config`, the other levels
/// searching an empty directory, compares the four calls on both sides, and checks
/// what `expect` asks of C's outcomes.
fn compare(tag: &str, config: &str, exact: bool, expect: fn(&[Outcome])) {
    let root = std::env::temp_dir().join(format!(
        "crustify-signature-equiv-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("levels")).unwrap();
    let levels = CString::new(root.join("levels").to_str().unwrap()).unwrap();
    for level in [
        SearchPathLevel::System,
        SearchPathLevel::Global,
        SearchPathLevel::Xdg,
    ] {
        // SAFETY: the case runs alone in its child, on one thread.
        unsafe { set_search_path(level, Some(&levels)) }.unwrap();
    }
    let path: PathBuf = root.join("repo.git");
    let c_path = CString::new(path.to_str().unwrap()).unwrap();
    drop(Repository::init(&c_path, true).unwrap());
    let mut file = std::fs::read_to_string(path.join("config")).unwrap();
    file.push_str(config);
    std::fs::write(path.join("config"), file).unwrap();

    let mut c_repo = ptr::null_mut();
    // SAFETY: a writable slot and a NUL-terminated path.
    let rc = unsafe { ffi::git_repository_open(&mut c_repo, c_path.as_ptr()) };
    assert_eq!(rc, 0);
    let rust_repo = Repository::open(&c_path).unwrap();
    // Twice: the second round reads the configuration both sides have loaded.
    for round in 0..2 {
        let c = c_calls(c_repo);
        let rust = rust_calls(&rust_repo);
        for (i, (r, c)) in rust.iter().zip(&c).enumerate() {
            assert!(same(r, c, exact), "round {round}, call {i}:\n{r:?}\n{c:?}");
        }
        for pair in [&c[3], &rust[3]] {
            if let (Ok(pair), false) = (pair, exact) {
                assert_eq!(pair[0].2, pair[1].2, "a pair shares its time");
            }
        }
        expect(&c);
    }
    drop(rust_repo);
    // SAFETY: the raw repository is released once; nothing of it is held.
    unsafe { ffi::git_repository_free(c_repo) };
    let _ = std::fs::remove_dir_all(&root);
}

const USER: &str = "[user]\n\tname = Config User\n\temail = config@example.com\n";

fn all_ok(c: &[Outcome]) {
    assert!(c.iter().all(Result::is_ok), "{c:?}");
}

fn all_failed(c: &[Outcome]) {
    assert!(c.iter().all(Result::is_err), "{c:?}");
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn identity_from_the_configuration() {
    isolated(
        concat!(module_path!(), "::identity_from_the_configuration"),
        &[],
        false,
        || compare("config", USER, false, all_ok),
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn identity_and_dates_from_the_environment() {
    isolated(
        concat!(module_path!(), "::identity_and_dates_from_the_environment"),
        &[
            ("GIT_AUTHOR_NAME", "  Env Author  "),
            ("GIT_AUTHOR_EMAIL", "author@example.com"),
            ("GIT_AUTHOR_DATE", "1700000000 +0130"),
            ("GIT_COMMITTER_NAME", "Env Committer"),
            ("GIT_COMMITTER_EMAIL", "committer@example.com"),
            ("GIT_COMMITTER_DATE", "2005-04-07T22:13:13-0700"),
        ],
        false,
        || {
            compare("env", "", true, |c| {
                // The configuration has no identity: the default fails.
                assert!(c[0].is_err() && c[1..].iter().all(Result::is_ok), "{c:?}");
            })
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn mixed_sources_and_the_email_fallback() {
    isolated(
        concat!(module_path!(), "::mixed_sources_and_the_email_fallback"),
        &[
            ("GIT_COMMITTER_NAME", "Env Committer"),
            ("EMAIL", "fallback@example.com"),
        ],
        false,
        || {
            compare("mixed", "[user]\n\tname = Config User\n", false, |c| {
                assert!(c[0].is_err() && c[1..].iter().all(Result::is_ok), "{c:?}");
            })
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn missing_identities() {
    isolated(
        concat!(module_path!(), "::missing_identities"),
        &[],
        false,
        || {
            compare("missing", "", false, all_failed);
            compare(
                "no-email",
                "[user]\n\tname = Only Name\n",
                false,
                all_failed,
            );
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn refused_values() {
    isolated(
        concat!(module_path!(), "::refused_values"),
        &[("GIT_AUTHOR_NAME", "Ang<le")],
        false,
        || {
            compare(
                "refused",
                "[user]\n\tname = \"  \"\n\temail = e@example.com\n",
                false,
                all_failed,
            );
        },
    );
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn dates_that_do_not_parse() {
    isolated(
        concat!(module_path!(), "::dates_that_do_not_parse"),
        &[
            ("GIT_AUTHOR_DATE", "not a date"),
            ("GIT_COMMITTER_DATE", "neither"),
        ],
        false,
        || {
            compare("bad-dates", USER, false, |c| {
                assert!(c[0].is_ok() && c[1..].iter().all(Result::is_err), "{c:?}");
            })
        },
    );
}

/// The divergence: C leaks the author when the committer fails after it; the outcome
/// is the same.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_failing_committer_after_the_author() {
    isolated(
        concat!(module_path!(), "::a_failing_committer_after_the_author"),
        &[("GIT_COMMITTER_EMAIL", "<bracketed>")],
        true,
        || {
            compare("committer-fails", USER, false, |c| {
                assert!(
                    c[0].is_ok() && c[1].is_ok() && c[2].is_err() && c[3].is_err(),
                    "{c:?}"
                );
                assert_eq!(c[2], c[3]);
            })
        },
    );
}

/// A configuration file corrupted after both sides opened the repository: each side's
/// first snapshot reads it afresh and fails alike, the environment's form first, then
/// the configuration's.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_configuration_corrupted_after_opening() {
    isolated(
        concat!(module_path!(), "::a_configuration_corrupted_after_opening"),
        &[],
        false,
        || {
            let root = std::env::temp_dir().join(format!(
                "crustify-signature-equiv-{}-corrupt",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("levels")).unwrap();
            let levels = CString::new(root.join("levels").to_str().unwrap()).unwrap();
            for level in [
                SearchPathLevel::System,
                SearchPathLevel::Global,
                SearchPathLevel::Xdg,
            ] {
                // SAFETY: the case runs alone in its child, on one thread.
                unsafe { set_search_path(level, Some(&levels)) }.unwrap();
            }
            let path = root.join("repo.git");
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            drop(Repository::init(&c_path, true).unwrap());
            let config = path.join("config");
            let mut file = std::fs::read_to_string(&config).unwrap();
            file.push_str(USER);
            std::fs::write(&config, &file).unwrap();
            let mut c_repo = ptr::null_mut();
            // SAFETY: a writable slot and a NUL-terminated path.
            let rc = unsafe { ffi::git_repository_open(&mut c_repo, c_path.as_ptr()) };
            assert_eq!(rc, 0);
            let rust_repo = Repository::open(&c_path).unwrap();
            file.push_str("[user\n");
            std::fs::write(&config, &file).unwrap();

            let mut c_author = ptr::null_mut();
            // SAFETY: a writable slot, no committer slot, and the live repository.
            let rc = unsafe {
                ffi::git_signature_default_from_env(&mut c_author, ptr::null_mut(), c_repo)
            };
            let c_seen = if rc < 0 {
                c_failed(rc)
            } else {
                Ok(vec![c_seen(c_author)])
            };
            let rust = match Signature::default_author_from_env(rust_repo.as_ref()) {
                Ok(sig) => Ok(vec![rust_seen(&sig)]),
                Err(e) => rust_failed(e),
            };
            assert!(c_seen.is_err(), "{c_seen:?}");
            assert!(same(&rust, &c_seen, false), "{rust:?}\n{c_seen:?}");
            // Then every call, in turn, on what each side's configuration now holds.
            let c = c_calls(c_repo);
            let rust = rust_calls(&rust_repo);
            for (i, (r, c)) in rust.iter().zip(&c).enumerate() {
                assert!(same(r, c, false), "call {i}:\n{r:?}\n{c:?}");
            }
            // Corrupted again: the configuration's form reads it first this time.
            file.push_str("[user\n");
            std::fs::write(&config, &file).unwrap();
            let c = c_calls(c_repo);
            let rust = rust_calls(&rust_repo);
            assert!(c[0].is_err(), "{:?}", c[0]);
            for (i, (r, c)) in rust.iter().zip(&c).enumerate() {
                assert!(same(r, c, false), "again, call {i}:\n{r:?}\n{c:?}");
            }
            drop(rust_repo);
            // SAFETY: the raw repository is released once; nothing of it is held.
            unsafe { ffi::git_repository_free(c_repo) };
            let _ = std::fs::remove_dir_all(&root);
        },
    );
}
