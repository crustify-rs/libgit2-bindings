//! `RepositoryRef::config` (`git_repository_config`) hands out a `ConfigArc` that does
//! not borrow the repository: the module docs say the config outlives it, because
//! `git_repository_free` disowns it (`GIT_REFCOUNT_OWN(config, NULL)`) first. But the
//! file backends the repository loaded the config with keep the repository too
//! (`config_file_open`, `b->repo = repo`), for `includeIf` conditions: every reparse
//! (`config_file_refresh` on a changed file, `config_file_refresh_from_buffer` after a
//! write) evaluates `gitdir:` through `git_repository__item_path(repo)` and
//! `onbranch:` through `git_repository_refdb(repo)` on that pointer, which
//! `git_repository_free` does not clear.
//!
//! Safe API only: the first repository's config is installed in a second one
//! (`RepositoryMut::set_config`), the first repository is dropped, and the second one
//! is made bare (`RepositoryMut::set_bare`), which writes `core.bare` through the
//! config and reparses the file: `heap-use-after-free` in `git_repository_path`
//! under AddressSanitizer.
#![forbid(unsafe_code)]

use std::ffi::CString;
use std::fs;

use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let dir = std::env::temp_dir().join(format!("config-includeif-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let first = dir.join("first");
    let second = dir.join("second");
    let c_first = CString::new(first.to_str().unwrap()).unwrap();
    let c_second = CString::new(second.to_str().unwrap()).unwrap();

    let a = Repository::init(&c_first, false).unwrap();
    // A conditional include: libgit2 evaluates it against the repository the config
    // was loaded for, on every reparse.
    fs::write(
        first.join(".git/config"),
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n[includeIf \"gitdir:/nowhere/\"]\n\tpath = extra\n",
    )
    .unwrap();
    let config = a.as_ref().config().unwrap();

    let mut b = Repository::init(&c_second, false).unwrap();
    b.as_mut().set_config(&config);
    drop(config);
    drop(a);

    // Writes `core.bare = true` through the installed config, whose file backend
    // reparses the file and evaluates the condition against the freed repository.
    b.as_mut().set_bare().unwrap();
    println!("no use-after-free observed");
    let _ = fs::remove_dir_all(&dir);
}
