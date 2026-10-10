//! UB tests for the unsafe API of `git2/tree.h`: `TreeBuilder::from_tree`, called within
//! its contract (a live `git_tree` for the call) on every shape of tree: the empty
//! tree, flat trees, a tree holding a subtree, a tree of the builder's own repository
//! or of another, and a SHA256 repository's tree seeding a SHA1 repository's builder.
//! Each builder is then driven through the safe API and the raw routines its symbols
//! will wrap: read, cleared, written and released, the source tree freed first (ASan:
//! use after free, overflow, double free; LSan: leak). The builder keeps nothing of the
//! tree, so freeing the tree right after the call is within the contract.

use std::ffi::{CStr, CString, c_int};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr::{self, NonNull};

use libgit2::ffi;
use libgit2::libgit2::object::tree::{TreeBuilder, TreeBuilderBox};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/resources");
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
/// `testrepo.git`'s `HEAD` tree: three blobs.
const FLAT: &str = "944c0f6e4dfa41595e6eb3ceecdb14f50fe18162";
/// A `testrepo.git` tree holding three blobs and the subtree `ab`.
const NESTED: &str = "ae90f12eea699729ed24555e40b9fd669da12a12";
/// `testrepo_256.git`'s `HEAD` tree: three blobs, with SHA256 ids.
const FLAT_256: &str = "f2a108f86a3b4fd9ad75ed55e9cb3cb46e348fca3b9dba3db64f7c9f64b8a736";

/// Opens the fixture repository `name` under `tests/resources`, read-only.
fn open(name: &str) -> RepositoryBox {
    let path = CString::new(format!("{RESOURCES}/{name}")).unwrap();
    Repository::open(&path).unwrap()
}

/// A fresh bare repository in a temporary directory, removed on drop after the
/// repository it holds.
struct TempRepo {
    path: PathBuf,
    repo: Option<RepositoryBox>,
}

impl TempRepo {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-tree-ub-unsafe-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let repo = Repository::init(&c_path, true).unwrap();
        Self {
            path,
            repo: Some(repo),
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

/// The tree `hex` names in `repo`: one counted reference the caller frees with
/// `git_tree_free`. Stands in for the `git_tree` wrapper.
fn lookup(repo: &RepositoryBox, hex: &str, kind: OidType) -> NonNull<ffi::git_tree> {
    let oid = Oid::from_string(&CString::new(hex).unwrap(), kind).unwrap();
    let id = ffi::git_oid {
        type_: oid.as_ref().raw_kind(),
        id: oid.as_ref().id(),
    };
    let mut tree = ptr::null_mut();
    // SAFETY: a writable slot, the box's live repository and a live id of its type.
    let rc = unsafe { ffi::git_tree_lookup(&mut tree, repo.as_c_ptr(), &id) };
    assert_eq!(rc, 0);
    NonNull::new(tree).unwrap()
}

fn free(tree: NonNull<ffi::git_tree>) {
    // SAFETY: a tree `lookup` handed out, released once.
    unsafe { ffi::git_tree_free(tree.as_ptr()) }
}

/// `git_tree_entrycount`, and each entry's name and raw file mode.
fn entries(tree: NonNull<ffi::git_tree>) -> Vec<(CString, ffi::git_filemode_t)> {
    // SAFETY: a live tree, only read; each entry lives with it and is copied out.
    unsafe {
        (0..ffi::git_tree_entrycount(tree.as_ptr()))
            .map(|i| {
                let entry = ffi::git_tree_entry_byindex(tree.as_ptr(), i);
                (name_of(entry), ffi::git_tree_entry_filemode_raw(entry))
            })
            .collect()
    }
}

/// The name of a live tree entry, copied.
///
/// # Safety
///
/// `entry` addresses a live tree entry.
unsafe fn name_of(entry: *const ffi::git_tree_entry) -> CString {
    // SAFETY: a live entry, per this function's contract, whose name is a
    // NUL-terminated string it owns.
    unsafe { CStr::from_ptr(ffi::git_tree_entry_name(entry)) }.to_owned()
}

fn count(builder: &TreeBuilderBox<'_>) -> usize {
    // SAFETY: the box's live builder, only read.
    unsafe { ffi::git_treebuilder_entrycount(builder.as_c_ptr()) }
}

/// `git_treebuilder_get`'s raw file mode for `name`, if the builder holds it.
fn mode_of(builder: &TreeBuilderBox<'_>, name: &CStr) -> Option<ffi::git_filemode_t> {
    // SAFETY: the box's live builder, only read; the entry it returns lives until the
    // builder next changes, after the read.
    unsafe {
        let entry = ffi::git_treebuilder_get(builder.as_c_ptr(), name.as_ptr());
        (!entry.is_null()).then(|| ffi::git_tree_entry_filemode_raw(entry))
    }
}

/// `git_treebuilder_write`'s id, as hex, or its return code.
fn write(builder: &mut TreeBuilderBox<'_>) -> Result<String, c_int> {
    let mut out = ffi::git_oid::default();
    // SAFETY: a writable id and the box's live builder, unaliased; its repository
    // outlives it.
    let rc = unsafe { ffi::git_treebuilder_write(&mut out, builder.as_c_ptr()) };
    if rc < 0 {
        return Err(rc);
    }
    let mut hex = [0u8; 65];
    // SAFETY: 65 writable bytes bound the write; `out` is live.
    unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &out) };
    Ok(CStr::from_bytes_until_nul(&hex)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned())
}

/// Seeds a builder of `target` with `source`, frees `source` at once, then checks
/// that the builder holds a copy of each entry.
fn seeded<'r>(target: &'r RepositoryBox, source: NonNull<ffi::git_tree>) -> TreeBuilderBox<'r> {
    let expected = entries(source);
    // SAFETY: `source` is a live tree, which `lookup` counted for the caller; it is
    // freed only after the call returns.
    let builder = unsafe { TreeBuilder::from_tree(target.as_ref(), source) }.unwrap();
    free(source);
    assert_eq!(count(&builder), expected.len());
    for (name, mode) in &expected {
        assert_eq!(mode_of(&builder, name), Some(*mode), "{name:?}");
    }
    builder
}

/// Flat and nested trees, seeding a builder of another repository, are copied whole:
/// the builder writes the very tree back, after its source is gone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn trees_seed_builders_of_another_repository() {
    let source = open("testrepo.git");
    let target = TempRepo::new("another");
    for hex in [FLAT, NESTED] {
        let mut builder = seeded(target.repo(), lookup(&source, hex, OidType::Sha1));
        assert_eq!(write(&mut builder).as_deref(), Ok(hex));
        builder.as_mut().clear();
        assert_eq!(count(&builder), 0);
        assert_eq!(write(&mut builder).as_deref(), Ok(EMPTY_TREE));
    }
    // The source repository may go before the builders seeded from it.
    let tree = lookup(&source, NESTED, OidType::Sha1);
    let builder = seeded(target.repo(), tree);
    drop(source);
    assert_eq!(count(&builder), 4);
    assert_eq!(mode_of(&builder, c"ab"), Some(ffi::GIT_FILEMODE_TREE));
}

/// A tree of the builder's own repository: the empty tree it wrote, then a tree with
/// entries, each builder released with its entries still held.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn trees_seed_builders_of_their_own_repository() {
    let temp = TempRepo::new("own");
    let repo = temp.repo();
    let mut empty = TreeBuilder::new(repo.as_ref()).unwrap();
    assert_eq!(write(&mut empty).as_deref(), Ok(EMPTY_TREE));
    drop(empty);

    let builder = seeded(repo, lookup(repo, EMPTY_TREE, OidType::Sha1));
    assert_eq!(count(&builder), 0);
    drop(builder);

    let source = open("testrepo.git");
    let mut builder = seeded(repo, lookup(&source, NESTED, OidType::Sha1));
    // Written into `repo`, the tree is now its own.
    let written = write(&mut builder).unwrap();
    drop(builder);
    let again = seeded(repo, lookup(repo, &written, OidType::Sha1));
    let mut copies: Vec<_> = (0..4)
        .map(|_| seeded(repo, lookup(repo, &written, OidType::Sha1)))
        .collect();
    copies[2].as_mut().clear();
    drop(again);
    drop(copies);
}

/// A SHA256 repository's tree seeds a SHA1 repository's builder: the ids are copied as
/// they are, and writing truncates each to SHA1's 20 bytes, within the id's storage;
/// the tree written is malformed but nothing is read or written out of bounds.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn trees_of_another_object_id_type_stay_in_bounds() {
    let source = open("testrepo_256.git");
    let target = TempRepo::new("sha256");
    let mut builder = seeded(target.repo(), lookup(&source, FLAT_256, OidType::Sha256));
    let written = write(&mut builder).unwrap();
    assert_ne!(written, FLAT_256);
    assert_eq!(written.len(), 40);
    builder.as_mut().clear();
    assert_eq!(write(&mut builder).as_deref(), Ok(EMPTY_TREE));
}
