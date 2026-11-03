//! UB tests for `git2/odb.h`'s `git_odb`: the constructors on `Odb`, the `OdbArc`
//! owner and the `OdbRef` handles it lends.
//!
//! Every constructor builds and releases odbs over every object id type, on success
//! and on each failure (`ASan` flags a use after free or double free, `LSan` a leaked odb,
//! backend or path). Odbs are shared with repositories that drop before and after the
//! owner, owners cross threads and drop there, and several threads use one odb through
//! shared borrows at once, as its `Sync` claim allows (`TSan` flags a race). The first
//! constructors of a process race to initialise libgit2. Handles are taken again and
//! again from one owner and used between other calls (`BSan`).
//!
//! The readers and writers on `OdbRef` run over both id types, on hits, misses and
//! ambiguous prefixes; ids of the other type, and of no type, are refused before
//! libgit2 formats them (`ASan` flags the loose backend's path overflow otherwise). Read
//! objects outlive their odb, are cloned and dropped in any order, cross threads and are
//! read from several at once, while other threads write, refresh and add alternates
//! (`TSan`). The foreach callback stops on a break and carries a panic back.

mod stream;
mod sys;

use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::odb::{
    ForeachCallback, LookupFlags, Odb, OdbArc, OdbError, OdbExpandId, OdbObject, OdbObjectArc,
    OdbObjectRef, OdbOptions, OdbRef,
};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::util::errors::{ErrorClass, ErrorCode};

use crate::fixture::odb::Repo;
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

/// A blob written through the odb's own writer.
fn write_blob(odb: &OdbArc<'_>, data: &[u8]) -> Result<Oid, OdbError> {
    odb.as_ref().write(data, ObjectType::Blob)
}

/// The header of `id`, through the odb's own reader.
fn read_header(odb: &OdbArc<'_>, id: &Oid) -> Result<(usize, Option<ObjectType>), OdbError> {
    odb.as_ref().read_header(id.as_ref())
}

fn backends(odb: &OdbArc<'_>) -> usize {
    odb.as_ref().num_backends()
}

/// Writes a few blobs and reads each back.
fn exercise(odb: &OdbArc<'_>, tag: &str) {
    for n in [0, 1, 4096, 70_000] {
        let data = format!("{tag}:{}", "x".repeat(n));
        let id = write_blob(odb, data.as_bytes()).unwrap();
        assert_eq!(
            read_header(odb, &id),
            Ok((data.len(), Some(ObjectType::Blob)))
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
                        Ok((data.len(), Some(ObjectType::Blob)))
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

/// Options built before libgit2 is initialised, on several threads at once (`TSan`: the
/// init routine touches no shared state; `MSan`: every byte it hands back is written),
/// moved across threads and used there to build odbs of the type they carry.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn odb_options_build_anywhere_and_cross_threads() {
    isolated(
        concat!(
            module_path!(),
            "::odb_options_build_anywhere_and_cross_threads"
        ),
        || {
            let built: Vec<OdbOptions> = (0..8)
                .map(|_| std::thread::spawn(OdbOptions::new))
                .map(|t| t.join().unwrap())
                .collect();
            for opts in &built {
                assert_eq!(*opts, OdbOptions::default());
                assert_eq!(opts.as_ref().oid_type(), None);
            }
            let kinds = [None, Some(OidType::Sha1), Some(OidType::Sha256)];
            let threads: Vec<_> = kinds
                .into_iter()
                .map(|kind| {
                    let mut opts = built[0];
                    opts.as_mut().set_oid_type(kind);
                    std::thread::spawn(move || {
                        let shared = opts.as_ref();
                        assert_eq!(shared.to_options(), opts);
                        let kind = shared.oid_type().unwrap_or_default();
                        let odb = Odb::with_oid_type(kind).unwrap();
                        assert_eq!(backends(&odb), 0);
                        format!("{opts:?}")
                    })
                })
                .collect();
            for t in threads {
                assert!(t.join().unwrap().starts_with("OdbOptions { version: 1"));
            }
        },
    );
}

/// Queries held in Rust storage: handles taken again and again and used between writes,
/// the id edited in place through a reborrow, copies outliving the borrow they came from,
/// a run shared by several reader threads at once (`TSan`) and queries moved to threads
/// that rewrite them. A query built from a zeroed one needs no libgit2 at all.
#[test]
fn odb_expand_ids_are_plain_values() {
    let mut run = vec![OdbExpandId::default(); 16];
    for (n, q) in run.iter_mut().enumerate() {
        let mut m = q.as_mut();
        m.set_length(u16::try_from(n).unwrap() * 4);
        m.set_kind(
            [None, Some(ObjectType::Blob), Some(ObjectType::Tree)]
                .get(n % 4)
                .copied()
                .flatten(),
        );
        {
            let mut id = m.id_mut();
            id.set_raw_kind(1);
            assert!(id.id_mut().set_elem(0, u8::try_from(n).unwrap()));
        }
        let shared = m.as_ref();
        assert_eq!(shared.id().id()[0], u8::try_from(n).unwrap());
        let copy = shared.to_expand_id();
        m.set_length(0);
        assert_eq!(copy.as_ref().length(), u16::try_from(n).unwrap() * 4);
    }
    let first = run[0];
    run[1].as_mut().set_id(first.as_ref().id());
    let (x, y) = (run[1].as_ref().id(), run[0].as_ref().id());
    assert_eq!((x.raw_kind(), x.id()), (y.raw_kind(), y.id()));

    let shared: Arc<[OdbExpandId]> = run.clone().into();
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                shared
                    .iter()
                    .map(|q| (q.as_ref().length(), q.as_ref().raw_kind()))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let views: Vec<_> = readers.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(views.windows(2).all(|w| w[0] == w[1]));

    let writers: Vec<_> = run
        .into_iter()
        .map(|mut q| {
            std::thread::spawn(move || {
                q.as_mut().set_kind(Some(ObjectType::Commit));
                q.as_mut().set_length(u16::MAX);
                q
            })
        })
        .collect();
    for t in writers {
        let q = t.join().unwrap();
        assert_eq!(
            (q.as_ref().length(), q.as_ref().kind()),
            (u16::MAX, Some(ObjectType::Commit))
        );
    }
}

/// Contents for the readers: empty, small, binary, and past a loose object's zlib window.
fn contents() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"hello, odb\n".to_vec(),
        (0..=255u8).collect(),
        (0..70_000u32).map(|i| (i * 7 % 251) as u8).collect(),
    ]
}

/// Checks an object against what was written.
fn check(object: OdbObjectRef<'_>, id: &Oid, kind: ObjectType, data: &[u8]) {
    assert_eq!(object.id(), id.as_ref());
    assert_eq!(object.kind(), Some(kind));
    assert_eq!(object.size(), data.len());
    assert_eq!(object.data(), data);
    assert!(format!("{object:?}").starts_with("OdbObject"));
}

/// Every reader, over both id types, on hits, misses and prefixes; the objects outlive
/// the odb and are dropped in any order.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn readers_hit_miss_and_outlive_the_odb() {
    init();
    let dir = TempDir::new("readers");
    for kind in TYPES {
        let odb = Odb::open_with_oid_type(&dir.dir(&format!("{}", kind.raw())), kind).unwrap();
        let handle = odb.as_ref();
        let mut kept: Vec<(OdbObjectArc, Oid, Vec<u8>)> = Vec::new();
        for (n, data) in contents().into_iter().enumerate() {
            let object_kind = if n == 0 {
                ObjectType::Tree
            } else {
                ObjectType::Blob
            };
            let id = handle.write(&data, object_kind).unwrap();
            assert_eq!(id.as_ref().kind(), Some(kind));
            assert!(handle.exists(id.as_ref()));
            assert!(handle.exists_ext(id.as_ref(), LookupFlags::NO_REFRESH));
            assert!(handle.exists_ext(id.as_ref(), LookupFlags::from_bits_retain(0xF0)));
            assert_eq!(
                handle.read_header(id.as_ref()),
                Ok((data.len(), Some(object_kind)))
            );
            let object = handle.read(id.as_ref()).unwrap();
            check(object.as_ref(), &id, object_kind, &data);
            for len in [
                4,
                7,
                kind.hex_size() - 1,
                kind.hex_size(),
                kind.hex_size() + 9,
                200,
            ] {
                assert_eq!(handle.exists_prefix(id.as_ref(), len), Ok(id), "{len}");
                let by_prefix = handle.read_prefix(id.as_ref(), len).unwrap();
                check(by_prefix.as_ref(), &id, object_kind, &data);
            }
            let clone = object.clone();
            assert!(OdbObjectArc::ptr_eq(&clone, &object));
            kept.push((clone, id, data));
            drop(object);
        }
        // Misses: the zero id, an id nothing has, and prefixes too short to look up.
        let zero = Oid::zero(kind);
        assert!(!handle.exists(zero.as_ref()));
        assert_eq!(
            handle.read(zero.as_ref()).unwrap_err().code(),
            Some(ErrorCode::NotFound)
        );
        assert_eq!(
            handle.read_header(zero.as_ref()).unwrap_err().code(),
            Some(ErrorCode::NotFound)
        );
        let mut absent = Oid::zero(kind);
        absent.as_mut().set_id([0xAB; 32]);
        assert!(!handle.exists(absent.as_ref()));
        assert!(!handle.exists_ext(absent.as_ref(), LookupFlags::NO_REFRESH));
        for len in [0, 1, 3] {
            let error = handle.read_prefix(absent.as_ref(), len).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Ambiguous), "{len}");
            let error = handle.exists_prefix(absent.as_ref(), len).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Ambiguous), "{len}");
        }
        assert_eq!(
            handle.read(absent.as_ref()).unwrap_err().code(),
            Some(ErrorCode::NotFound)
        );
        assert_eq!(
            handle
                .exists_prefix(absent.as_ref(), 10)
                .unwrap_err()
                .code(),
            Some(ErrorCode::NotFound)
        );
        handle.refresh().unwrap();
        // The objects outlive the odb, and drop in reverse order on another thread.
        drop(odb);
        std::thread::spawn(move || {
            while let Some((object, id, data)) = kept.pop() {
                assert_eq!(object.as_ref().id(), id.as_ref());
                assert_eq!(object.as_ref().data(), &data[..]);
            }
        })
        .join()
        .unwrap();
    }
}

/// Ids of the other type, of no type, and of a type no header names are refused before
/// libgit2 formats them into a path sized for the odb's type, at every length.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn ids_of_another_type_are_refused() {
    init();
    let dir = TempDir::new("mismatch");
    for kind in TYPES {
        let odb = Odb::open_with_oid_type(&dir.dir(&format!("{}", kind.raw())), kind).unwrap();
        let handle = odb.as_ref();
        let other = if kind == OidType::Sha1 {
            OidType::Sha256
        } else {
            OidType::Sha1
        };
        let mut ids = Vec::new();
        for raw_kind in [0, u8::try_from(other.raw()).unwrap(), 3, 0xFF] {
            let mut id = Oid::zero(kind);
            id.as_mut().set_id([0x5A; 32]);
            id.as_mut().set_raw_kind(raw_kind);
            ids.push(id);
        }
        let written = handle.write(b"present", ObjectType::Blob).unwrap();
        let mut retyped = written;
        retyped
            .as_mut()
            .set_raw_kind(u8::try_from(other.raw()).unwrap());
        ids.push(retyped);
        for id in &ids {
            assert!(!handle.exists(id.as_ref()), "{id:?}");
            assert!(!handle.exists_ext(id.as_ref(), LookupFlags::NONE), "{id:?}");
            let refused = |error: OdbError| {
                assert_eq!(error.code(), Some(ErrorCode::Generic));
                assert_eq!(error.class(), Some(ErrorClass::Invalid));
                assert_eq!(
                    error.message(),
                    c"specified object ID type does not match object database object ID type"
                );
            };
            refused(handle.read(id.as_ref()).unwrap_err());
            refused(handle.read_header(id.as_ref()).unwrap_err());
            for len in [4, 40, 41, 64, 65, usize::MAX] {
                refused(handle.read_prefix(id.as_ref(), len).unwrap_err());
                refused(handle.exists_prefix(id.as_ref(), len).unwrap_err());
            }
        }
        // The odb still serves ids of its own type.
        assert!(handle.exists(written.as_ref()));
    }
}

/// An odb with no backends answers every reader without one.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn an_odb_without_backends_finds_nothing() {
    init();
    for kind in TYPES {
        let odb = Odb::with_oid_type(kind).unwrap();
        let handle = odb.as_ref();
        let mut id = Oid::zero(kind);
        id.as_mut().set_id([1; 32]);
        assert!(!handle.exists(id.as_ref()));
        assert!(handle.read(id.as_ref()).is_err());
        assert!(handle.read_header(id.as_ref()).is_err());
        assert!(handle.read_prefix(id.as_ref(), 8).is_err());
        assert!(handle.exists_prefix(id.as_ref(), 8).is_err());
        assert!(handle.write(b"x", ObjectType::Blob).is_err());
        handle.refresh().unwrap();
    }
}

/// Alternates are added while other threads read, write, refresh and look up through
/// the same odb; objects read on one thread are read on others.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn readers_writers_and_alternates_share_one_odb() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<OdbObjectArc>();
    send_sync::<OdbObjectRef<'static>>();
    send_sync::<OdbObject>();
    init();
    let dir = TempDir::new("concurrent");
    // Alternates seeded with a blob each.
    let alternates: Vec<(CString, Oid)> = (0..4)
        .map(|i| {
            let path = dir.dir(&format!("alt{i}"));
            let seed = Odb::open(&path).unwrap();
            let id = write_blob(&seed, format!("alternate {i}").as_bytes()).unwrap();
            (path, id)
        })
        .collect();
    let odb = Odb::open(&dir.dir("main")).unwrap();
    let shared = write_blob(&odb, b"shared").unwrap();
    let object = odb.as_ref().read(shared.as_ref()).unwrap();
    std::thread::scope(|s| {
        for (t, (path, id)) in alternates.iter().enumerate() {
            let odb = &odb;
            let object = &object;
            s.spawn(move || {
                let handle = odb.as_ref();
                handle.add_disk_alternate(path).unwrap();
                assert!(handle.exists(id.as_ref()));
                for i in 0..8 {
                    let data = format!("thread {t} blob {i}");
                    let written = handle.write(data.as_bytes(), ObjectType::Blob).unwrap();
                    let read = handle.read(written.as_ref()).unwrap();
                    assert_eq!(read.as_ref().data(), data.as_bytes());
                    assert_eq!(object.as_ref().data(), b"shared");
                    let again = object.clone();
                    drop(again);
                    handle.refresh().unwrap();
                    let _ = handle.num_backends();
                }
            });
        }
    });
    // Two backends each for the directory and every alternate.
    assert_eq!(backends(&odb), 2 + 2 * alternates.len());
    for (_, id) in &alternates {
        assert!(odb.as_ref().read(id.as_ref()).is_ok());
    }
    // An alternate is never written to.
    let (path, _) = &alternates[0];
    let alone = Odb::new().unwrap();
    alone.as_ref().add_disk_alternate(path).unwrap();
    assert_eq!(backends(&alone), 2);
    assert!(write_blob(&alone, b"read-only").is_err());
    // A missing directory is skipped; adding a directory twice adds it once.
    alone
        .as_ref()
        .add_disk_alternate(c"/nonexistent/objects")
        .unwrap();
    alone.as_ref().add_disk_alternate(path).unwrap();
    assert_eq!(backends(&alone), 2);
}

/// The foreach callback continues, breaks with its code, and carries a panic back to
/// the caller instead of across C.
#[test]
#[cfg_attr(
    miri,
    ignore = "calls libgit2: `OidRef::to_oid` copies with `git_oid_cpy`"
)]
fn the_foreach_callback_breaks_and_carries_panics() {
    use std::num::NonZero;
    use std::ops::ControlFlow;

    let ids: Vec<Oid> = (0..4u8)
        .map(|i| {
            let mut id = Oid::zero(OidType::Sha1);
            id.as_mut().set_id([i; 32]);
            id
        })
        .collect();
    let mut seen = Vec::new();
    let mut count = |id: libgit2::libgit2::src::libgit2::oid::OidRef<'_>| {
        seen.push(id.to_oid());
        if seen.len() == 3 {
            ControlFlow::Break(NonZero::new(-42).unwrap())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut callback = ForeachCallback::new(&mut count);
    let flow: Vec<_> = ids.iter().map(|id| callback.call(id.as_ref())).collect();
    assert!(format!("{callback:?}").contains("panicked: false"));
    drop(callback);
    assert_eq!(
        flow,
        [
            ControlFlow::Continue(()),
            ControlFlow::Continue(()),
            ControlFlow::Break(NonZero::new(-42).unwrap()),
            ControlFlow::Continue(()),
        ]
    );
    assert_eq!(seen, ids);
    let mut panics = |_: libgit2::libgit2::src::libgit2::oid::OidRef<'_>| -> ControlFlow<_> {
        panic!("from the closure")
    };
    let mut callback = ForeachCallback::new(&mut panics);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.call(ids[0].as_ref())
    }));
    assert_eq!(
        caught.unwrap_err().downcast_ref::<&str>(),
        Some(&"from the closure")
    );
    // The callback is usable again once the panic has resumed.
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        callback.call(ids[1].as_ref())
    }));
    assert!(caught.is_err());
}
