//! The tests of `src/libgit2/odb.c`: its two `unsafe` methods, each called within its
//! contract (no other thread uses the odb for the call), over odbs with and without
//! backends, packs and commit-graphs, of both id types, owned by a repository or not,
//! and used afterwards (`ASan` flags a commit-graph or pack list used after it was
//! freed, `LSan` one leaked).

mod sys;

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libgit2::libgit2::src::libgit2::object::{Object, ObjectType};
use libgit2::libgit2::src::libgit2::odb::{Odb, OdbArc};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::libgit2::repository::Repository;
use libgit2::libgit2::src::util::errors::{ErrorClass, ErrorCode};

/// A directory of its own under the temporary directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-odb-ub-unsafe-{}-{tag}",
            std::process::id()
        ));
        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn c_path(&self, name: &str) -> CString {
        CString::new(self.0.join(name).as_os_str().as_bytes()).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
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

/// A writable copy of the libgit2 test repository `name`, at `scratch/name`.
fn resource(scratch: &Scratch, name: &str) -> CString {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("c/tests/resources")
        .join(name);
    copy_dir(&source, &scratch.0.join(name));
    scratch.c_path(name)
}

/// `testrepo.git`'s `master` commit, packed.
const MASTER: &std::ffi::CStr = c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750";

/// Reads `id` and checks that it is a commit.
fn reads_commit(odb: &OdbArc<'_>, id: &Oid) {
    let object = odb.as_ref().read(id.as_ref()).unwrap();
    assert_eq!(object.as_ref().kind(), Some(ObjectType::Commit));
    assert_eq!(object.as_ref().id(), id.as_ref());
}

/// The multi-pack-index is written over every pack, replacing the one there, and the
/// odb reads through it afterwards; again and again; on each id type.
#[test]
#[cfg_attr(miri, ignore = "copies a fixture: file system isolation")]
fn write_multi_pack_index_over_packs_of_each_type() {
    let scratch = Scratch::new("midx");
    let sha1 = resource(&scratch, "testrepo.git");
    let objects = scratch.0.join("testrepo.git/objects");
    fs::remove_file(objects.join("pack/multi-pack-index")).unwrap();
    let odb = Odb::open(&scratch.c_path("testrepo.git/objects")).unwrap();
    let master = Oid::from_string(MASTER, OidType::Sha1).unwrap();
    reads_commit(&odb, &master);
    for _ in 0..3 {
        // SAFETY: this thread alone uses the odb, which no repository owns.
        unsafe { odb.as_ref().write_multi_pack_index() }.unwrap();
        assert!(objects.join("pack/multi-pack-index").is_file());
        reads_commit(&odb, &master);
    }
    drop(odb);
    // Reopened, the odb reads through the index it wrote.
    let odb = Odb::open(&scratch.c_path("testrepo.git/objects")).unwrap();
    reads_commit(&odb, &master);
    drop(sha1);

    drop(resource(&scratch, "testrepo_256.git"));
    let odb = Odb::open_with_oid_type(&scratch.c_path("testrepo_256.git/objects"), OidType::Sha256)
        .unwrap();
    // SAFETY: as above.
    unsafe { odb.as_ref().write_multi_pack_index() }.unwrap();
    assert!(
        scratch
            .0
            .join("testrepo_256.git/objects/pack/multi-pack-index")
            .is_file()
    );
    assert!(odb.as_ref().num_backends() >= 2);
}

/// Without a main backend that packs, the write is refused, and nothing changes.
#[test]
#[cfg_attr(miri, ignore = "copies a fixture: file system isolation")]
fn write_multi_pack_index_needs_a_main_packed_backend() {
    let scratch = Scratch::new("midx-refused");
    let unsupported = |odb: &OdbArc<'_>| {
        // SAFETY: this thread alone uses the odb, which no repository owns.
        let error = unsafe { odb.as_ref().write_multi_pack_index() }.unwrap_err();
        assert_eq!(error.code(), Some(ErrorCode::Generic));
        assert_eq!(error.class(), Some(ErrorClass::Odb));
        assert_eq!(
            error.message(),
            c"cannot write multi-pack-index - unsupported in the loaded odb backends"
        );
    };
    for kind in [OidType::Sha1, OidType::Sha256] {
        unsupported(&Odb::with_oid_type(kind).unwrap());
    }
    // An alternate is never written to.
    drop(resource(&scratch, "testrepo.git"));
    let alone = Odb::new().unwrap();
    alone
        .as_ref()
        .add_disk_alternate(&scratch.c_path("testrepo.git/objects"))
        .unwrap();
    unsupported(&alone);
    reads_commit(&alone, &Oid::from_string(MASTER, OidType::Sha1).unwrap());
}

/// A repository's own odb, used by this thread alone: the repository reads through the
/// rewritten index, and through the odb once its commit-graph is gone.
#[test]
#[cfg_attr(miri, ignore = "copies a fixture: file system isolation")]
fn a_repositorys_odb_within_the_contract() {
    let scratch = Scratch::new("repository");
    let path = resource(&scratch, "testrepo.git");
    let repo = Repository::open(&path).unwrap();
    let master = Oid::from_string(MASTER, OidType::Sha1).unwrap();
    let odb = repo.as_ref().odb().unwrap();
    // SAFETY: this thread alone uses the odb and the repository that owns it.
    unsafe { odb.as_ref().write_multi_pack_index() }.unwrap();
    // SAFETY: as above; no revision walk holds the graph.
    unsafe { odb.as_ref().clear_commit_graph() }.unwrap();
    drop(odb);
    let commit = Object::lookup(repo.as_ref(), master.as_ref(), None).unwrap();
    assert_eq!(commit.as_ref().kind(), ObjectType::Commit);
    drop(commit);
    let odb = repo.as_ref().odb().unwrap();
    reads_commit(&odb, &master);
    drop(odb);
    drop(repo);
}

/// The commit-graph is freed and not reloaded by reads or refreshes, cleared again
/// harmlessly, and reloaded by the next disk backend added; an odb without one clears
/// nothing.
#[test]
#[cfg_attr(miri, ignore = "copies a fixture: file system isolation")]
fn clear_commit_graph_frees_it_once_and_again() {
    let scratch = Scratch::new("cgraph");
    drop(resource(&scratch, "testrepo.git"));
    let objects = scratch.c_path("testrepo.git/objects");
    let master = Oid::from_string(MASTER, OidType::Sha1).unwrap();
    for kind in [OidType::Sha1, OidType::Sha256] {
        let empty = Odb::with_oid_type(kind).unwrap();
        for _ in 0..2 {
            // SAFETY: this thread alone uses the odb, which no repository owns.
            unsafe { empty.as_ref().clear_commit_graph() }.unwrap();
        }
    }
    let odb = Odb::open(&objects).unwrap();
    reads_commit(&odb, &master);
    for _ in 0..2 {
        // SAFETY: as above.
        unsafe { odb.as_ref().clear_commit_graph() }.unwrap();
        odb.as_ref().refresh().unwrap();
        reads_commit(&odb, &master);
    }
    // A disk backend added later loads a graph again, which the next clear frees.
    odb.as_ref().add_disk_alternate(&objects).unwrap();
    let elsewhere = Scratch::new("cgraph-alternate");
    drop(resource(&elsewhere, "testrepo.git"));
    odb.as_ref()
        .add_disk_alternate(&elsewhere.c_path("testrepo.git/objects"))
        .unwrap();
    // SAFETY: as above.
    unsafe { odb.as_ref().clear_commit_graph() }.unwrap();
    reads_commit(&odb, &master);
    // Moved to another thread, which then uses it alone.
    std::thread::spawn(move || {
        // SAFETY: as above: the odb moved here, and no other thread holds it.
        unsafe { odb.as_ref().clear_commit_graph() }.unwrap();
        reads_commit(&odb, &master);
    })
    .join()
    .unwrap();
}
