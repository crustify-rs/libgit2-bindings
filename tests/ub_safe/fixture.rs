//! The suite's fixtures: repositories, objects and options built through the safe API.

use std::ffi::{CStr, c_int};
use std::process::Command;

const CHILD: &str = "CRUSTIFY_UB_SAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2.
pub(crate) fn isolated(test: &str, body: fn()) {
    let name = test.split_once("::").map_or(test, |(_, rest)| rest);
    if std::env::var(CHILD).as_deref() == Ok(name) {
        body();
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
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

/// A plain C struct to place in an `OdbDataBox`; the crate wraps no C type yet.
///
/// Replace with a wrapped libgit2 type once one is resource-free and `CZeroable`.
#[repr(C)]
#[expect(
    unreachable_pub,
    reason = "`define_ctype!` makes its newtype `pub`, which names this layout"
)]
pub struct pair_st {
    pub a: u64,
    pub b: u32,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);

impl PairRef<'_> {}

// --- `git2/types.h`: `git_writestream` ---------------------------------------------
use std::ffi::CString;
use std::path::PathBuf;

use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};

/// Takes a `git_libgit2_init` reference that is never released.
pub(crate) fn init() {
    let _ = core::mem::ManuallyDrop::new(libgit2::libgit2::src::libgit2::libgit2::init().unwrap());
}

/// A fresh bare repository in a temporary directory, freed and removed on drop.
pub(crate) struct TempRepo {
    pub path: PathBuf,
    repo: Option<RepositoryBox>,
}

impl TempRepo {
    pub(crate) fn new(tag: &str) -> Self {
        init();
        let path = std::env::temp_dir().join(format!(
            "crustify-writestream-ub-{}-{tag}",
            std::process::id()
        ));
        drop(std::fs::remove_dir_all(&path));
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let repo = Repository::init(&c_path, true).unwrap();
        Self {
            path,
            repo: Some(repo),
        }
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        // Every stream the tests opened on the repository is gone by now (they borrow
        // the fixture); free it before its directory goes.
        drop(self.repo.take());
        drop(std::fs::remove_dir_all(&self.path));
    }
}

thread_local! {
    /// `Upper` streams made and dropped on this thread: the callback makes them,
    /// libgit2 frees them on the thread that runs the chain.
    static UPPER: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

// --- `git2/repository.h`: `git_repository_item_t` -----------------------------------

use libgit2::libgit2::src::libgit2::repository::Item;

/// [`RepositoryRef::item_path`](libgit2::libgit2::src::libgit2::repository::RepositoryRef::item_path)
/// as a string: the path, or the return.
pub(crate) fn item_path(repo: &TempRepo, item: Item) -> Result<String, c_int> {
    match repo.repo.as_ref().unwrap().as_ref().item_path(item) {
        Ok(buf) => Ok(buf.as_ref().to_str().unwrap().to_owned()),
        Err(error) => Err(error.raw_code()),
    }
}

// --- `git2/commit.h`: `git_commitarray`; `git2/config.h`: `git_config` --------------

/// Fixtures over the repositories that hand out commit arrays and configs, through the
/// safe wrappers.
pub(crate) mod commit_and_config {
    use libgit2::libgit2::src::libgit2::commit::CommitArrayVal;
    use libgit2::libgit2::src::libgit2::config::ConfigArc;
    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};

    /// A fixture repository opened read-only, freed on drop.
    pub(crate) struct OpenRepo(RepositoryBox);

    /// `testrepo.git` (`HEAD` at `a65fedf`), `mergedrepo` (merging `e280915` into
    /// `3a34580`) and `empty_bare.git` (an unborn `HEAD`).
    pub(crate) const TESTREPO: &str = "testrepo.git";
    pub(crate) const MERGEDREPO: &str = "mergedrepo/.gitted";
    pub(crate) const EMPTY: &str = "empty_bare.git";

    impl OpenRepo {
        pub(crate) fn open(name: &str) -> Self {
            super::init();
            let path = std::ffi::CString::new(format!(
                "{}/c/tests/resources/{name}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            Self(Repository::open(&path).unwrap())
        }
    }

    /// The commit parents, through the safe wrapper.
    pub(crate) fn commit_parents(repo: &OpenRepo) -> CommitArrayVal<'_> {
        repo.0.as_ref().commit_parents().unwrap()
    }

    /// The repository's config, through the safe wrapper: a shared reference, which
    /// borrows the repository (its file backends read it on every reparse).
    pub(crate) fn config(repo: &OpenRepo) -> ConfigArc<'_> {
        repo.0.as_ref().config().unwrap()
    }
}

// --- `git2/commit.h`: `git_commit_create_ext_options` ---------------------------------

/// Commit creation through the `git_commit_create_ext` wrapper, reading the commit back
/// through the object database's read wrapper.
pub(crate) mod create_ext {
    use std::ffi::c_int;

    use libgit2::libgit2::src::libgit2::commit::{CreateExtOptions, create_ext};
    use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    use libgit2::libgit2::src::libgit2::signature::{SignatureView, Time};
    use libgit2::libgit2::src::libgit2::tree::Tree;

    use super::TempRepo;

    /// [`create_ext`] of a root commit of the empty tree in `repo`, with a fixed
    /// signature and message, through `options`: the new commit's raw text, or the code
    /// the wrapper failed with. A panic of the options' signer resumes from here.
    pub(crate) fn create(
        repo: &TempRepo,
        options: &mut CreateExtOptions<'_>,
    ) -> Result<String, c_int> {
        let r = repo.repo.as_ref().unwrap().as_ref();
        let empty = c"4b825dc642cb6eb9a060e54bf8d69288fbee4904";
        let tree = Tree::lookup(r, Oid::from_string(empty, OidType::Sha1).unwrap().as_ref());
        let tree = tree.unwrap();
        let sig = SignatureView::new(c"A", c"a@b", Time::new(1, 0));
        let id = create_ext(
            r,
            sig.as_ref(),
            sig.as_ref(),
            c"msg\n",
            tree.as_ref(),
            &[],
            Some(options.as_mut()),
        )
        .map_err(|e| e.raw_code())?;
        let object = r.odb().unwrap().as_ref().read(id.as_ref()).unwrap();
        Ok(String::from_utf8(object.as_ref().data().to_vec()).unwrap())
    }
}

// --- `git2/refdb.h`: `git_refdb` ---------------------------------------------------

use libgit2::libgit2::src::libgit2::refdb::{Refdb, RefdbBox};

impl TempRepo {
    /// A refdb of this repository, through [`Refdb::open`]. The borrow of the fixture is
    /// the refdb's `'r`, so the repository outlives it.
    pub(crate) fn refdb(&self) -> RefdbBox<'_> {
        Refdb::open(self.repo.as_ref().unwrap().as_ref()).unwrap()
    }
}

// --- `git2/odb.h`: `git_odb` -------------------------------------------------------

pub(crate) mod odb {
    //! A repository over an odb, for the tests that share one with it.

    use libgit2::libgit2::src::libgit2::odb::OdbArc;
    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};

    /// A repository over an odb ([`Repository::wrap_odb`]), which holds a reference of
    /// its own and becomes the odb's owner; freed on drop. [`odb`](Self::odb) stands in
    /// for the `git_repository_odb` wrapper, which lends the odb back, borrowing the
    /// repository.
    ///
    /// [`Repository::wrap_odb`]: libgit2::libgit2::src::libgit2::repository::Repository::wrap_odb
    pub(crate) struct Repo(RepositoryBox);

    impl Repo {
        pub(crate) fn wrap(odb: OdbArc<'static>) -> Self {
            Self(Repository::wrap_odb(odb).unwrap())
        }

        /// A new reference to the repository's odb (`RepositoryRef::odb`), borrowing
        /// the repository that owns it.
        pub(crate) fn odb(&self) -> OdbArc<'_> {
            self.0.as_ref().odb().unwrap()
        }
    }
}

// --- `git2/index.h`: `git_index` ---------------------------------------------------

/// Fixtures over `git_index`, standing in for the index and repository symbol
/// wrappers, which replace them: they observe and change an index through raw calls on
/// an owner's pointer, which `as_c_ptr` hands out.
pub(crate) mod index {
    use std::ffi::{CStr, CString, c_int};

    use ffibox::CZeroable;
    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::index::{
        Capabilities, EntryExtendedFlags, EntryFlags, IndexArc, IndexBox, IndexEntry, IndexRef,
        ReucSide,
    };
    use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    use libgit2::libgit2::src::libgit2::repository::RepositoryError;
    use libgit2::libgit2::src::libgit2::tree::FileMode;

    use super::{CHILD, TempRepo};

    /// As [`isolated`](super::isolated), with `LeakSanitizer` off in the child: for bodies
    /// that drive libgit2's index reader down a path on which it leaks (a version 4
    /// entry cut short leaks its path, `read_entry` in `src/libgit2/index.c`). Every
    /// other sanitizer check stays on.
    pub(crate) fn isolated_tolerating_leaks(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
        let separator = if inherited.is_empty() { "" } else { ":" };
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            )
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Either index owner, by its C pointer or its shared handle.
    pub(crate) trait Owner {
        fn handle(&self) -> IndexRef<'_>;
    }

    impl Owner for IndexBox {
        fn handle(&self) -> IndexRef<'_> {
            self.as_ref()
        }
    }

    impl Owner for IndexArc {
        fn handle(&self) -> IndexRef<'_> {
            self.as_ref()
        }
    }

    /// The repository's index, through the safe wrapper: a counted reference.
    pub(crate) fn repository_index(repo: &TempRepo) -> IndexArc {
        repo.repo.as_ref().unwrap().as_ref().index().unwrap()
    }

    /// `RepositoryMut::set_index` on the fixture's repository, which keeps a
    /// reference of its own: it takes an `IndexArc`.
    pub(crate) fn set_repository_index(repo: &mut TempRepo, index: &IndexArc) {
        repo.repo.as_mut().unwrap().as_mut().set_index(Some(index));
    }

    /// `IndexRef::path`, copied out.
    pub(crate) fn path(index: &impl Owner) -> Option<CString> {
        index.handle().path().map(CStr::to_owned)
    }

    /// `IndexMut::add` of a regular file at `path`; the status.
    pub(crate) fn add(index: &mut IndexBox, path: &CStr) -> c_int {
        add_flagged(index, path, 0)
    }

    /// `IndexMut::add` of a regular file at `path` in `stage` (a conflict side for 1 to
    /// 3), of the SHA1 id starting with the bytes of `id`; the status. A repository's
    /// index needs the id of an object its repository has.
    pub(crate) fn add_staged_id(index: &mut IndexBox, path: &CStr, stage: u16, id: &[u8]) -> c_int {
        let mut bytes = [0; 32];
        bytes[..id.len()].copy_from_slice(id);
        let mut entry = IndexEntry::new(path, Oid::zeroed().as_ref(), FileMode::Blob);
        let mut handle = entry.as_mut();
        // `GIT_INDEX_ENTRY_STAGESHIFT`, a header macro.
        handle
            .set_flags(EntryFlags::from_bits_retain(u32::from(stage) << 12))
            .unwrap();
        handle.id_mut().set_kind(OidType::Sha1);
        handle.id_mut().set_id(bytes);
        status(index.as_mut().add(entry.as_ref()))
    }

    /// `IndexMut::add` of a regular file at `path` marked intent-to-add, which needs a
    /// version 3 entry; the status.
    pub(crate) fn add_intent_to_add(index: &mut IndexBox, path: &CStr) -> c_int {
        add_flagged(
            index,
            path,
            u16::try_from(ffi::GIT_INDEX_ENTRY_INTENT_TO_ADD).unwrap(),
        )
    }

    /// `IndexMut::set_version`; the status.
    pub(crate) fn set_version(index: &mut IndexBox, version: u32) -> c_int {
        status(index.as_mut().set_version(version))
    }

    fn status(result: Result<(), RepositoryError>) -> c_int {
        result.map_or_else(|e| e.raw_code(), |()| 0)
    }

    fn add_flagged(index: &mut IndexBox, path: &CStr, flags_extended: u16) -> c_int {
        let mut entry = IndexEntry::new(path, Oid::zeroed().as_ref(), FileMode::Blob);
        let mut handle = entry.as_mut();
        handle
            .set_flags_extended(EntryExtendedFlags::from_bits_retain(flags_extended.into()))
            .unwrap();
        handle.id_mut().set_kind(OidType::Sha1);
        let mut id = [0; 32];
        id[0] = 1;
        handle.id_mut().set_id(id);
        status(index.as_mut().add(entry.as_ref()))
    }

    /// `IndexRef::write`; the status.
    pub(crate) fn write(index: &mut IndexBox) -> c_int {
        status(index.as_ref().write())
    }

    /// `IndexMut::add_reuc`, replacing any record for `path`; the status (`-1`, the
    /// only failure, for an error). A side whose mode is 0 is absent.
    pub(crate) fn add_reuc(
        index: &mut IndexBox,
        path: &CStr,
        modes: [c_int; 3],
        ids: [&Oid; 3],
    ) -> c_int {
        let side = |i: usize| {
            (modes[i] != 0).then(|| ReucSide {
                mode: (modes[i]).cast_unsigned(),
                id: ids[i].as_ref(),
            })
        };
        let added = index.as_mut().add_reuc(path, side(0), side(1), side(2));
        added.map_or(-1, |()| 0)
    }

    /// `IndexMut::remove_reuc`; the status: its only failure, a position out of range,
    /// is `GIT_ENOTFOUND`.
    pub(crate) fn remove_reuc(index: &mut IndexBox, position: usize) -> c_int {
        let removed = index.as_mut().remove_reuc(position);
        removed.map_or(ffi::GIT_ENOTFOUND, |()| 0)
    }

    /// `IndexMut::clear_reuc`; the status.
    pub(crate) fn clear_reuc(index: &mut IndexBox) -> c_int {
        index.as_mut().clear_reuc().map_or(-1, |()| 0)
    }

    /// `IndexMut::set_caps` with ignore-case on or off; the status.
    pub(crate) fn set_ignore_case(index: &mut IndexBox, on: bool) -> c_int {
        let caps = if on {
            Capabilities::IGNORE_CASE
        } else {
            Capabilities::empty()
        };
        status(index.as_mut().set_caps(caps.into()))
    }
}

// --- `git2/worktree.h` ---------------------------------------------------------------

pub(crate) mod worktree {
    //! Repositories with linked worktrees laid out on disk, for the worktree wrappers
    //! to look up.

    use std::ffi::{CStr, CString};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox, RepositoryRef};
    use libgit2::libgit2::src::libgit2::worktree::{Worktree, WorktreeBox};

    /// A non-bare repository in a temporary directory, to which [`add`](Self::add) lays
    /// out linked worktrees on disk as `git_worktree_add` would; freed and removed on
    /// drop.
    pub(crate) struct WorktreeRepo {
        pub base: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl WorktreeRepo {
        pub(crate) fn new(tag: &str) -> Self {
            super::init();
            let base = std::env::temp_dir()
                .join(format!("crustify-worktree-ub-{}-{tag}", std::process::id()));
            drop(fs::remove_dir_all(&base));
            let c_path = CString::new(base.join("repo").as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            Self {
                base,
                repo: Some(repo),
            }
        }

        /// The worktree `name`'s administrative directory.
        pub(crate) fn admin(&self, name: &str) -> PathBuf {
            self.base.join("repo/.git/worktrees").join(name)
        }

        /// The worktree `name`'s working directory.
        pub(crate) fn workdir(&self, name: &str) -> PathBuf {
            self.base.join("worktrees").join(name)
        }

        /// Lays out the worktree `name`: its administrative directory under
        /// `.git/worktrees/` and its working directory beside the repository.
        pub(crate) fn add(&self, name: &str) {
            let admin = self.admin(name);
            let workdir = self.workdir(name);
            fs::create_dir_all(&admin).unwrap();
            fs::create_dir_all(&workdir).unwrap();
            fs::write(admin.join("commondir"), "../..\n").unwrap();
            let gitlink = workdir.join(".git");
            fs::write(admin.join("gitdir"), format!("{}\n", gitlink.display())).unwrap();
            fs::write(admin.join("HEAD"), format!("ref: refs/heads/{name}\n")).unwrap();
            fs::write(gitlink, format!("gitdir: {}\n", admin.display())).unwrap();
        }

        /// The repository.
        pub(crate) fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }

        /// The worktree `name`.
        pub(crate) fn lookup(&self, name: &CStr) -> WorktreeBox {
            Worktree::lookup(self.repo(), name).unwrap()
        }

        /// Frees the repository and removes its directory, leaving the worktrees looked
        /// up in it.
        #[expect(
            clippy::unused_self,
            reason = "a method of the fixture, as its callers use it"
        )]
        pub(crate) fn close(self) {}
    }

    impl Drop for WorktreeRepo {
        fn drop(&mut self) {
            // The worktrees do not borrow the repository; free it before its directory.
            drop(self.repo.take());
            drop(fs::remove_dir_all(&self.base));
        }
    }
}

// --- `git2/worktree.h`: `git_worktree_add` -----------------------------------------

/// A repository to add linked worktrees to, through `Worktree::add`.
pub(crate) mod worktree_add {
    use std::ffi::{CString, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use libgit2::libgit2::src::libgit2::object::ObjectType;
    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
    use libgit2::libgit2::src::libgit2::worktree::{AddOptions, Worktree, WorktreeBox};

    /// A fresh non-bare repository at `<base>/repo` whose `HEAD` is `main`, a commit of
    /// the empty tree (so a checkout loads no filter), with the branches `feature` and
    /// `alpha` and the lightweight tag `v1` on it; new worktrees go to
    /// `<base>/wt/<name>`. Freed and removed on drop.
    pub(crate) struct AddRepo {
        pub base: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl AddRepo {
        #[expect(
            clippy::items_after_statements,
            reason = "each helper item sits beside the steps that use it"
        )]
        pub(crate) fn new(tag: &str) -> Self {
            super::init();
            let base = std::env::temp_dir().join(format!(
                "crustify-worktree-add-ub-{}-{tag}",
                std::process::id()
            ));
            drop(std::fs::remove_dir_all(&base));
            let path = base.join("repo");
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            let odb = repo.as_ref().odb().unwrap();
            let write = |data: &[u8], kind: ObjectType| {
                let id = odb.as_ref().write(data, kind).unwrap();
                id.as_ref().to_hex_string().unwrap()
            };
            const WHO: &str = "A U Thor <author@example.com> 1700000000 +0000";
            let tree = write(b"", ObjectType::Tree);
            let commit = format!("tree {tree}\nauthor {WHO}\ncommitter {WHO}\n\nfirst\n");
            let commit = write(commit.as_bytes(), ObjectType::Commit);
            drop(odb);
            // Loose files: `git_reference_create` would take a reflog signature at the
            // current time, whose timezone lookup TSan reports against other tests'.
            for name in [
                "refs/heads/main",
                "refs/heads/feature",
                "refs/heads/alpha",
                "refs/tags/v1",
            ] {
                std::fs::write(path.join(".git").join(name), format!("{commit}\n")).unwrap();
            }
            std::fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            // `git_worktree_add` creates the worktree's directory, not its parents.
            std::fs::create_dir(base.join("wt")).unwrap();
            Self {
                base,
                repo: Some(repo),
            }
        }

        /// `Worktree::add(repo, name, <base>/wt/<name>, opts)`: the new worktree, or the
        /// error code and message.
        pub(crate) fn add(
            &self,
            name: &str,
            opts: Option<&AddOptions<'_>>,
        ) -> Result<WorktreeBox, (c_int, String)> {
            let c_name = CString::new(name).unwrap();
            let dir = self.base.join("wt").join(name);
            let c_dir = CString::new(dir.as_os_str().as_bytes()).unwrap();
            let repo = self.repo.as_ref().unwrap().as_ref();
            Worktree::add(repo, &c_name, &c_dir, opts.map(AddOptions::as_ref))
                .map_err(|e| (e.raw_code(), e.message().to_string_lossy().into_owned()))
        }

        /// The repository.
        pub(crate) fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }

        /// The new worktree `name`'s administrative directory.
        pub(crate) fn admin(&self, name: &str) -> PathBuf {
            self.base.join("repo/.git/worktrees").join(name)
        }
    }

    impl Drop for AddRepo {
        fn drop(&mut self) {
            // Its references borrowed `self` and are gone; free it before its directory.
            drop(self.repo.take());
            drop(std::fs::remove_dir_all(&self.base));
        }
    }
}

// --- `git2/sys/repository.h`: the submodule cache ------------------------------------

pub(crate) mod submodule {
    //! A repository with one configured submodule, and submodules looked up through
    //! `git_submodule_lookup`, standing in for the submodule wrappers.

    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};

    /// A non-bare repository whose `.gitmodules` configures the submodule `sub`;
    /// removed on drop.
    pub(crate) struct ModulesRepo {
        pub base: PathBuf,
        pub repo: RepositoryBox,
    }

    impl ModulesRepo {
        pub(crate) fn new(tag: &str) -> Self {
            super::init();
            let base = std::env::temp_dir().join(format!(
                "crustify-submodule-ub-{}-{tag}",
                std::process::id()
            ));
            drop(fs::remove_dir_all(&base));
            let c_path = CString::new(base.as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            let this = Self { base, repo };
            this.set_ignore("none");
            this
        }

        /// Rewrites `.gitmodules` with `sub`'s `ignore` setting.
        pub(crate) fn set_ignore(&self, ignore: &str) {
            fs::write(
                self.base.join(".gitmodules"),
                format!(
                    "[submodule \"sub\"]\n\tpath = sub\n\turl = https://example.com/sub\n\
                     \tignore = {ignore}\n"
                ),
            )
            .unwrap();
        }
    }

    impl Drop for ModulesRepo {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.base));
        }
    }
}

// --- `git2/repository.h`: routines (head and lookups) --------------------------------

/// Copies of libgit2's test repositories.
pub(crate) mod head_and_lookups {
    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};

    const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/c/tests/resources");

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), &target).unwrap();
            } else if kind.is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// Copies of `tests/resources` repositories under a scratch directory, removed on
    /// drop: `bare` (`testrepo.git`), `testrepo` with its linked worktree
    /// `testrepo-worktree`, and `merged` (`mergedrepo`, mid-merge).
    pub(crate) struct Sandbox(pub PathBuf);

    impl Sandbox {
        pub(crate) fn new(tag: &str) -> Self {
            super::init();
            let root = std::env::temp_dir()
                .join(format!("crustify-routines-ub-{}-{tag}", std::process::id()));
            drop(fs::remove_dir_all(&root));
            for (name, to) in [
                ("testrepo.git", "bare"),
                ("testrepo", "testrepo"),
                ("testrepo-worktree", "testrepo-worktree"),
                ("mergedrepo", "merged"),
            ] {
                let to = root.join(to);
                copy_tree(&Path::new(RESOURCES).join(name), &to);
                if to.join(".gitted").exists() {
                    fs::rename(to.join(".gitted"), to.join(".git")).unwrap();
                }
            }
            Self(root)
        }

        pub(crate) fn c_path(&self, rel: &str) -> CString {
            CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
        }

        pub(crate) fn open(&self, rel: &str) -> RepositoryBox {
            Repository::open(&self.c_path(rel)).unwrap()
        }

        pub(crate) fn write(&self, rel: &str, contents: &str) {
            fs::write(self.0.join(rel), contents).unwrap();
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            drop(fs::remove_dir_all(&self.0));
        }
    }
}

// --- `git2/tree.h`: `git_treebuilder` ----------------------------------------------

/// Fixtures over the trees and tree builders the tests drive, through their safe
/// getters.
pub(crate) mod tree {
    use std::ffi::{CStr, CString};

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
    use libgit2::libgit2::src::libgit2::tree::{TreeArc, TreeBuilderBox};

    /// Each entry's name and raw file mode, of `tree`, through the safe getters.
    pub(crate) fn entries(tree: &TreeArc<'_>) -> Vec<(CString, ffi::git_filemode_t)> {
        let tree = tree.as_ref();
        let entries: Vec<_> = tree
            .iter()
            .map(|entry| (entry.name().to_owned(), entry.filemode_raw()))
            .collect();
        assert_eq!(entries.len(), tree.len());
        entries
    }

    /// A SHA1 object id of `byte` repeated, for entries libgit2 does not check.
    pub(crate) fn id(byte: u8) -> Oid {
        Oid::from_raw(&[byte; 20], OidType::Sha1).unwrap()
    }

    /// The builder's entry count, through the safe `len`.
    pub(crate) fn count(builder: &TreeBuilderBox<'_>) -> usize {
        builder.as_ref().len()
    }

    /// The raw file mode of the builder's entry `name`, if it holds one.
    pub(crate) fn mode_of(
        builder: &TreeBuilderBox<'_>,
        name: &CStr,
    ) -> Option<ffi::git_filemode_t> {
        Some(builder.as_ref().get(name)?.filemode_raw())
    }
}

// --- `git2/object.h`: `git_object`; `git2/tree.h`: `git_tree_entry` -----------------

/// Fixtures over `git_object` and `git_tree_entry`: C's view of an owner, through the
/// pointer `as_c_ptr` hands out, standing in for the object and entry getters.
pub(crate) mod object_and_tree_entry {
    use std::ffi::CString;

    use libgit2::ffi;
    use libgit2::libgit2::src::libgit2::blob::create_from_buffer;
    use libgit2::libgit2::src::libgit2::object::{Object, ObjectArc, ObjectType};
    use libgit2::libgit2::src::libgit2::oid::Oid;
    use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox, RepositoryRef};
    use libgit2::libgit2::src::libgit2::tree::{FileMode, TreeBuilder, TreeEntryBox};

    use super::TempRepo;

    /// `testrepo.git`, opened read-only: lookups only fill its object cache.
    pub(crate) fn open_testrepo() -> RepositoryBox {
        super::init();
        let path = CString::new(format!(
            "{}/c/tests/resources/testrepo.git",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Repository::open(&path).unwrap()
    }

    /// A temporary repository's handle.
    pub(crate) fn repository(repo: &TempRepo) -> RepositoryRef<'_> {
        repo.repo.as_ref().unwrap().as_ref()
    }

    /// The hex id and C's type of the object an owner holds, through the safe getters.
    pub(crate) fn object_view(object: &ObjectArc<'_>) -> (String, ffi::git_object_t) {
        let object = object.as_ref();
        (object.id().to_hex_string().unwrap(), object.kind().raw())
    }

    /// A new blob of `content` in `repo`, looked up as an object; its hex id is read
    /// through [`object_view`].
    pub(crate) fn write_blob<'r>(repo: &'r RepositoryBox, content: &[u8]) -> ObjectArc<'r> {
        let id = create_from_buffer(repo.as_ref(), content).unwrap();
        Object::lookup(repo.as_ref(), id.as_ref(), Some(ObjectType::Blob)).unwrap()
    }

    /// The name, hex id and raw file mode of the entry an owner holds, through the
    /// safe getters.
    pub(crate) fn entry_view(entry: &TreeEntryBox) -> (CString, String, u32) {
        let entry = entry.as_ref();
        (
            entry.name().to_owned(),
            entry.id().to_hex_string().unwrap(),
            entry.filemode_raw(),
        )
    }

    /// Writes a blob holding `data` and a tree naming it once under each of `names`,
    /// through a tree builder; the tree's id.
    pub(crate) fn write_tree(repo: &TempRepo, data: &[u8], names: &[CString]) -> Oid {
        let blob = create_from_buffer(repository(repo), data).unwrap();
        let mut builder = TreeBuilder::new(repository(repo)).unwrap();
        for name in names {
            let inserted = builder.as_mut().insert(name, blob.as_ref(), FileMode::Blob);
            assert!(inserted.is_ok(), "{name:?}: {inserted:?}");
        }
        builder.as_mut().write().unwrap()
    }

    /// `OdbRef::write` of `data` as an object of type `kind`, which the object
    /// database stores without parsing it; the id.
    pub(crate) fn write_raw(repo: &TempRepo, data: &[u8], kind: ffi::git_object_t) -> Oid {
        let kind = ObjectType::from_raw(kind).expect("an object type");
        let odb = repository(repo).odb().unwrap();
        odb.as_ref().write(data, kind).unwrap()
    }
}

// --- `git2/filter.h`: `git_filter_list` ---------------------------------------------

/// A probe filter for the blob-taking filter list constructors: registered once as
/// `crustify-probe` and selected where the `crustify-probe` attribute is set, its
/// `check` records the id the filter source carries (the blob's) and keeps a payload of
/// its own in the list, which its `cleanup` frees when the list is. Stands in for a
/// filter written through the `git2/sys/filter.h` wrappers (a later run).
pub(crate) mod filter_probe {
    use std::cell::RefCell;

    thread_local! {
        /// What the probe's `check` saw on this thread: the source's id, hex, if any.
        static SEEN: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
        /// Payloads the probe's `check` made and its `cleanup` freed on this thread.
        static PAYLOADS: RefCell<(usize, usize)> = const { RefCell::new((0, 0)) };
    }
}

// --- `git2/signature.h`: repository defaults ----------------------------------------

/// An index holding a regular-file entry at each of `paths`, added through
/// `IndexMut::add`.
pub(crate) fn match_list_index(paths: &[&CStr]) -> libgit2::libgit2::src::libgit2::index::IndexBox {
    let mut index = libgit2::libgit2::src::libgit2::index::Index::new().unwrap();
    for path in paths {
        assert_eq!(index::add(&mut index, path), 0);
    }
    index
}
