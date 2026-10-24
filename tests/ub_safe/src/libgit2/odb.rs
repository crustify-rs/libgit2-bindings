//! UB tests for `git2/odb.h`'s `git_odb`: the constructors on `Odb`, the `OdbArc`
//! owner and the `OdbRef` handles it lends.
//!
//! Every constructor builds and releases odbs over every object id type, on success
//! and on each failure (`ASan` flags a use after free or double free, `LSan` a leaked odb,
//! backend or path). Odbs are shared with repositories that drop before and after the
//! owner, owners cross threads and drop there, and several threads use one odb through
//! shared borrows at once, as its `Sync` claim allows (`TSan` flags a race). The first
//! constructors of a process race to initialise libgit2. Handles are taken again and
//! again from one owner and used between other calls (`BSan`). The odb readers and
//! writers are stand-ins from `fixture::odb`.

mod sys;

use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::{Odb, OdbArc, OdbRef};
use libgit2::libgit2::src::libgit2::oid::OidType;

use crate::fixture::odb::{Repo, backends, read_header, write_blob};
use crate::fixture::{init, isolated};

/// A directory of its own under the temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("crustify-odb-ub-{}-{tag}", std::process::id()));
        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// `name` under the directory, created as a directory, as a C string.
    fn dir(&self, name: &str) -> CString {
        let path = self.0.join(name);
        fs::create_dir_all(&path).unwrap();
        CString::new(path.to_str().unwrap()).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

const TYPES: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

/// Writes a few blobs and reads each back.
fn exercise(odb: &OdbArc<'_>, tag: &str) {
    for n in [0, 1, 4096, 70_000] {
        let data = format!("{tag}:{}", "x".repeat(n));
        let id = write_blob(odb, data.as_bytes()).unwrap();
        assert_eq!(
            read_header(odb, &id),
            Ok((data.len(), ffi::GIT_OBJECT_BLOB))
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn every_constructor_builds_and_releases() {
    init();
    let dir = TempDir::new("constructors");
    for round in 0..8 {
        let empty = Odb::new().unwrap();
        assert_eq!(backends(&empty), 0);
        assert!(write_blob(&empty, b"nowhere to go").is_err());
        for kind in TYPES {
            let empty = Odb::with_oid_type(kind).unwrap();
            assert_eq!(backends(&empty), 0);
            let objects = dir.dir(&format!("{}/{round}", kind.raw()));
            let on_disk = Odb::open_with_oid_type(&objects, kind).unwrap();
            assert_eq!(backends(&on_disk), 2);
            exercise(&on_disk, "typed");
        }
        let on_disk = Odb::open(&dir.dir(&format!("plain/{round}"))).unwrap();
        exercise(&on_disk, "plain");
    }
}

/// Failed opens build nothing and leak nothing; alternates that cycle or nest deeper
/// than libgit2 follows are opened, each directory once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn failed_and_nested_opens_release_everything() {
    init();
    let dir = TempDir::new("opens");
    let missing = CString::new(dir.0.join("missing").to_str().unwrap()).unwrap();
    let unreadable = dir.dir("unreadable");
    fs::create_dir_all(dir.0.join("unreadable/info/alternates")).unwrap();
    for path in [&missing, &CString::default(), &unreadable] {
        assert!(Odb::open(path).is_err(), "{path:?}");
        for kind in TYPES {
            assert!(Odb::open_with_oid_type(path, kind).is_err(), "{path:?}");
        }
    }
    // A chain of eight directories, each naming the next and the first.
    let chain: Vec<CString> = (0..8).map(|i| dir.dir(&format!("chain/{i}"))).collect();
    for (i, objects) in chain.iter().enumerate() {
        let info = PathBuf::from(objects.to_str().unwrap()).join("info");
        fs::create_dir_all(&info).unwrap();
        let next = chain.get(i + 1).unwrap_or(&chain[0]).to_str().unwrap();
        let first = chain[0].to_str().unwrap();
        fs::write(info.join("alternates"), format!("{next}\n{first}\n.\n")).unwrap();
    }
    for _ in 0..4 {
        let odb = Odb::open(&chain[0]).unwrap();
        // The directory and six levels of alternates (libgit2 reads the alternates
        // files of the first six), two backends each.
        assert_eq!(backends(&odb), 14);
        exercise(&odb, "chained");
    }
}

/// An owner lends handles again and again, between uses of the odb through the owner.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn handles_are_lent_again_and_again() {
    let odb = Odb::new().unwrap();
    let handles: Vec<OdbRef<'_, 'static>> = (0..16).map(|_| odb.as_ref()).collect();
    for (i, handle) in handles.iter().enumerate() {
        let copy = *handle;
        assert_eq!(format!("{copy:?}"), format!("{:?}", odb.as_ref()));
        assert_eq!(backends(&odb), 0, "{i}");
    }
}

/// A repository given the odb keeps it alive once the caller's reference is gone, and
/// hands out owners of its own, each borrowing the repository: those owners drop in
/// any order before it, and the odb stays usable through whichever is left.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_repository_shares_the_odb_in_either_drop_order() {
    init();
    let dir = TempDir::new("shared");
    // The caller's reference goes into the repository, which keeps the odb alive.
    let repo = Repo::wrap(Odb::open(&dir.dir("a")).unwrap());
    let again = repo.odb();
    exercise(&again, "owner first");
    drop(again);
    drop(repo);
    // Several owners from the repository, dropped in either order among themselves.
    let repo = Repo::wrap(Odb::open(&dir.dir("b")).unwrap());
    let first = repo.odb();
    let others: Vec<OdbArc<'_>> = (0..4).map(|_| repo.odb()).collect();
    exercise(&first, "first owner");
    for other in &others {
        exercise(other, "other owner");
    }
    drop(first);
    let last = repo.odb();
    drop(others);
    exercise(&last, "last owner");
    drop(last);
    drop(repo);
    // Two repositories in turn over one directory, each the owner of its odb.
    let first = Repo::wrap(Odb::open(&dir.dir("c")).unwrap());
    exercise(&first.odb(), "first repository");
    drop(first);
    let second = Repo::wrap(Odb::open(&dir.dir("c")).unwrap());
    exercise(&second.odb(), "second repository");
    drop(second);
}

/// Owners cross threads and drop there; one odb is used from several threads at once
/// through shared borrows, and the last of several `Arc`s drops on whichever thread.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn odbs_are_shared_and_sent_across_threads() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OdbArc<'static>>();
    send_sync::<OdbRef<'static, 'static>>();
    init();
    let dir = TempDir::new("threads");
    let odb = Odb::open(&dir.dir("objects")).unwrap();
    std::thread::scope(|s| {
        for t in 0..4 {
            let odb = &odb;
            s.spawn(move || {
                let handle = odb.as_ref();
                for i in 0..16 {
                    let data = format!("thread {t} blob {i}");
                    let id = write_blob(odb, data.as_bytes()).unwrap();
                    assert_eq!(
                        read_header(odb, &id),
                        Ok((data.len(), ffi::GIT_OBJECT_BLOB))
                    );
                    // The same object from every thread: libgit2 serialises the writes.
                    write_blob(odb, b"common").unwrap();
                }
                format!("{handle:?}")
            });
        }
    });
    let sent = Odb::open(&dir.dir("sent")).unwrap();
    std::thread::spawn(move || {
        exercise(&sent, "sent");
        drop(sent);
    })
    .join()
    .unwrap();
    let shared = Arc::new(odb);
    let threads: Vec<_> = (0..4)
        .map(|t| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || exercise(&shared, &format!("arc {t}")))
        })
        .collect();
    drop(shared);
    for thread in threads {
        thread.join().unwrap();
    }
}

/// The first libgit2 calls of a process are odb constructors, on several threads at
/// once: each pins the runtime before allocating (`TSan` flags a race with
/// `git_libgit2_init`'s allocator setup).
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn first_constructors_race_to_pin_the_runtime() {
    isolated(
        concat!(
            module_path!(),
            "::first_constructors_race_to_pin_the_runtime"
        ),
        || {
            let threads: Vec<_> = (0..8)
                .map(|t| {
                    std::thread::spawn(move || {
                        let odb = if t % 2 == 0 {
                            Odb::new()
                        } else {
                            Odb::with_oid_type(OidType::Sha256)
                        };
                        assert_eq!(backends(&odb.unwrap()), 0);
                    })
                })
                .collect();
            for thread in threads {
                thread.join().unwrap();
            }
        },
    );
}
