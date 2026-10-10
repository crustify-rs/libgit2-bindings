//! UB tests for the unsafe API of `git2/repository.h` and `git2/sys/repository.h`:
//! `RepositoryRef::set_refdb`, whose caller vouches that the installed refdb's own
//! repository outlives this one's reference operations and shares its object id type.
//!
//! Every call honours that contract: the refdb is opened for the repository itself, or
//! for a repository of the same type declared before (and so dropped after) the one
//! installing it, and no libgit2 routine is running on the repository. Each refdb is
//! released exactly once, by whichever of the arc and the repository lets go last, a
//! replaced one included (ASan: use after free, double free; LSan: leak), and the
//! reference operations of the installing repository reach the refdb's own repository
//! only while it lives (ASan). Every test calls libgit2, so nothing here runs under
//! Miri.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libgit2::libgit2::refs::refdb::{Refdb, RefdbArc};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

/// A scratch directory of its own under the temp dir, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-repository-ub-unsafe-{}-{tag}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A writable copy of libgit2's bare `testrepo.git`, at `scratch/name`.
fn testrepo(scratch: &Scratch, name: &str) -> RepositoryBox {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/resources/testrepo.git");
    let path = scratch.0.join(name);
    copy_dir(&source, &path);
    Repository::open(&CString::new(path.as_os_str().as_bytes()).unwrap()).unwrap()
}

fn head(scratch: &Scratch, name: &str) -> String {
    fs::read_to_string(scratch.0.join(name).join("HEAD")).unwrap()
}

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process.
fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "child `{name}` failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The repository's own refdbs, installed in turn: each replaces and releases the one
/// before, whether the caller's arc goes before or after the swap.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_refdb_installs_the_repository_s_own_refdbs() {
    isolated(
        concat!(
            module_path!(),
            "::set_refdb_installs_the_repository_s_own_refdbs"
        ),
        set_refdb_installs_the_repository_s_own_refdbs_alone,
    );
}

/// The body, alone in a child process: libgit2 writes the reflog through
/// `git_signature_now`, whose glibc `tzset` TSan reports against any concurrent caller.
fn set_refdb_installs_the_repository_s_own_refdbs_alone() {
    let scratch = Scratch::new("own");
    let repo = testrepo(&scratch, "repo");
    let lent = repo.as_ref().refdb().unwrap();
    for keep_arc in [false, true, false] {
        let db: RefdbArc<'_> = Refdb::open(repo.as_ref()).unwrap().into();
        // SAFETY: a refdb opened for this very repository, which outlives every
        // operation through it; no libgit2 routine is running on the repository.
        unsafe { repo.as_ref().set_refdb(&db) };
        let installed = repo.as_ref().refdb().unwrap();
        assert_eq!(installed.as_c_ptr(), db.as_c_ptr());
        drop(installed);
        if keep_arc {
            repo.as_ref().set_head(c"refs/heads/br2").unwrap();
            assert_eq!(head(&scratch, "repo"), "ref: refs/heads/br2\n");
            drop(db);
        } else {
            drop(db);
            repo.as_ref().set_head(c"refs/heads/master").unwrap();
            assert_eq!(head(&scratch, "repo"), "ref: refs/heads/master\n");
        }
    }
    // The refdb lent before the first swap is still a counted reference of its own.
    drop(lent);
    drop(repo);
}

/// A refdb of another repository of the same type, which outlives the one installing
/// it: the installing repository's reference operations reach the refdb's repository,
/// as libgit2 keeps it, and its storage.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn set_refdb_installs_the_refdb_of_an_outliving_repository() {
    isolated(
        concat!(
            module_path!(),
            "::set_refdb_installs_the_refdb_of_an_outliving_repository"
        ),
        set_refdb_installs_the_refdb_of_an_outliving_repository_alone,
    );
}

/// The body, alone in a child process: libgit2 writes the reflog through
/// `git_signature_now`, whose glibc `tzset` TSan reports against any concurrent caller.
fn set_refdb_installs_the_refdb_of_an_outliving_repository_alone() {
    let scratch = Scratch::new("outliving");
    // Declared first, so dropped last.
    let source = testrepo(&scratch, "source");
    let db: RefdbArc<'_> = Refdb::open(source.as_ref()).unwrap().into();
    for target in [testrepo(&scratch, "target"), Repository::new().unwrap()] {
        // SAFETY: `source` is of the same type (SHA1) and outlives `target`, which
        // this iteration drops; no libgit2 routine is running on `target`.
        unsafe { target.as_ref().set_refdb(&db) };
        target.as_ref().set_head(c"refs/heads/haacked").unwrap();
        assert_eq!(head(&scratch, "source"), "ref: refs/heads/haacked\n");
        source.as_ref().set_head(c"refs/heads/master").unwrap();
        drop(target);
    }
    // The copy's own storage was never written.
    assert_eq!(head(&scratch, "target"), "ref: refs/heads/master\n");
    drop(db);
    drop(source);
}
