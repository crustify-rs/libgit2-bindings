//! UB tests for `git2/odb_backend.h`'s `git_odb_stream` and `git_odb_writepack`: the
//! stream constructors on `OdbRef`, the `OdbStreamBox` and `OdbWritepackBox` owners,
//! their handles' getters, and the writepack's `append` and `commit`.
//!
//! Streams are opened over every object id type and many sizes, and released unwritten,
//! partly written, overwritten and stored (`ASan` flags a use after free or double free,
//! `LSan` a leaked hashing context, temporary file or buffer); several are open at once
//! on one odb and finished out of order, handles taken again and again between calls
//! (`BSan`). Threads share one odb and each opens its own streams (`TSan`). Writepacks
//! take a real pack in pieces and commit it, and are dropped unfinished and after
//! failures. The stream operations and the writepack constructor are stand-ins from
//! `fixture::odb_stream`.

use std::ffi::CString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::indexer::IndexerProgress;
use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::odb::{Odb, OdbArc, OdbError, OdbStreamBox, OdbWritepack};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::util::errors::{ErrorClass, ErrorCode};

use crate::fixture::init;
use crate::fixture::odb::Repo;
use crate::fixture::odb_stream::{finalize, read, write, write_pack};

/// A blob written through the odb's own writer, as the raw id the stream steps compare.
fn write_blob(odb: &OdbArc<'_>, data: &[u8]) -> Result<ffi::git_oid, OdbError> {
    odb.as_ref().write(data, ObjectType::Blob).map(|id| {
        let id = id.as_ref();
        ffi::git_oid {
            type_: id.raw_kind(),
            id: id.id(),
        }
    })
}

/// The header of the raw id `id`, through the odb's own reader, as raw values.
fn read_header(
    odb: &OdbArc<'_>,
    id: &ffi::git_oid,
) -> Result<(usize, ffi::git_object_t), OdbError> {
    odb.as_ref()
        .read_header(oid(id).as_ref())
        .map(|(len, kind)| (len, kind.map_or(ffi::GIT_OBJECT_INVALID, ObjectType::raw)))
}

/// A directory of its own under the temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-odb-stream-ub-{}-{tag}",
            std::process::id()
        ));
        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// An `objects` directory under the directory, with its `pack` subdirectory, as a
    /// C string.
    fn objects(&self, name: &str) -> CString {
        let path = self.0.join(name);
        fs::create_dir_all(path.join("pack")).unwrap();
        CString::new(path.to_str().unwrap()).unwrap()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

const TYPES: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

/// The id libgit2 returned, as a safe [`Oid`].
fn oid(id: &ffi::git_oid) -> Oid {
    let kind = OidType::try_from(ffi::git_oid_t::from(id.type_)).unwrap();
    let len = if kind == OidType::Sha1 { 20 } else { 32 };
    Oid::from_raw(&id.id[..len], kind).unwrap()
}

/// The raw id spelled in hex by `hex`, of `kind`.
fn raw_oid(hex: &str, kind: OidType) -> ffi::git_oid {
    let mut id = ffi::git_oid {
        type_: u8::try_from(kind.raw()).unwrap(),
        ..ffi::git_oid::default()
    };
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        id.id[i] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
    }
    id
}

/// Writes `data` through a fresh write stream, in pieces of `piece` bytes, and stores it.
fn stream_blob(odb: &OdbArc<'_>, data: &[u8], piece: usize) -> ffi::git_oid {
    let mut stream = odb
        .as_ref()
        .open_write_stream(data.len() as u64, ObjectType::Blob)
        .unwrap();
    for chunk in data.chunks(piece.max(1)) {
        assert_eq!(write(&mut stream, chunk), 0);
        assert!(stream.as_ref().received_bytes() <= stream.as_ref().declared_size());
    }
    assert_eq!(stream.as_ref().received_bytes(), data.len() as u64);
    finalize(&mut stream).unwrap()
}

/// Checks what a fresh write stream over an odb of `kind` reports.
fn fresh_write_stream(stream: &OdbStreamBox<'_>, size: u64, kind: OidType) {
    let s = stream.as_ref();
    assert_eq!(s.declared_size(), size);
    assert_eq!(s.received_bytes(), 0);
    assert_eq!(s.oid_type(), Ok(kind));
    assert_eq!(s.mode(), ffi::GIT_STREAM_WRONLY);
    assert!(s.has_hash_ctx());
    assert!(s.can_write() && s.can_finalize_write() && !s.can_read());
    assert!(!s.backend().is_null());
    assert!(format!("{s:?}").starts_with("OdbStreamRef"));
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn write_streams_open_and_release_in_every_state() {
    init();
    let dir = TempDir::new("write");
    for kind in TYPES {
        let odb = Odb::open_with_oid_type(&dir.objects(&format!("w{}", kind.raw())), kind).unwrap();
        for (round, size) in [0_usize, 1, 7, 4096, 70_000].into_iter().enumerate() {
            let data = vec![b'a' + u8::try_from(round).unwrap(); size];

            // Released unwritten.
            let stream = odb
                .as_ref()
                .open_write_stream(size as u64, ObjectType::Blob)
                .unwrap();
            fresh_write_stream(&stream, size as u64, kind);
            drop(stream);

            // Released partly written.
            let mut stream = odb
                .as_ref()
                .open_write_stream(size as u64, ObjectType::Blob)
                .unwrap();
            assert_eq!(write(&mut stream, &data[..size / 2]), 0);
            assert_eq!(stream.as_ref().received_bytes(), (size / 2) as u64);
            // Short of the declared size, unless that is 0.
            assert_eq!(finalize(&mut stream).is_err(), size > 0);
            drop(stream);

            // Overwritten: the write is counted, then refused.
            let mut stream = odb
                .as_ref()
                .open_write_stream(size as u64, ObjectType::Blob)
                .unwrap();
            assert_eq!(write(&mut stream, &data), 0);
            assert!(write(&mut stream, b"x") < 0);
            assert_eq!(stream.as_ref().received_bytes(), size as u64 + 1);
            drop(stream);

            // Stored, in pieces and whole: the same id, readable from the odb.
            let id = stream_blob(&odb, &data, 3);
            assert_eq!(oid(&stream_blob(&odb, &data, size)), oid(&id));
            assert_eq!(oid(&id).as_ref().kind(), Some(kind));
            assert_eq!(read_header(&odb, &id), Ok((size, ffi::GIT_OBJECT_BLOB)));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn write_streams_store_every_object_type() {
    init();
    let dir = TempDir::new("kinds");
    let odb = Odb::open(&dir.objects("objects")).unwrap();
    // A blob of any content; the others are stored as given, unparsed.
    for kind in [
        ObjectType::Blob,
        ObjectType::Commit,
        ObjectType::Tree,
        ObjectType::Tag,
    ] {
        let data = b"tree 0000\n";
        let mut stream = odb
            .as_ref()
            .open_write_stream(data.len() as u64, kind)
            .unwrap();
        assert_eq!(write(&mut stream, data), 0);
        let id = finalize(&mut stream).unwrap();
        assert_eq!(read_header(&odb, &id), Ok((data.len(), kind.raw())));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn streams_on_an_odb_without_backends_fail() {
    init();
    for kind in TYPES {
        let odb = Odb::with_oid_type(kind).unwrap();
        for _ in 0..4 {
            let err = odb
                .as_ref()
                .open_write_stream(4, ObjectType::Blob)
                .unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Odb));
            assert!(err.raw_code() < 0);
            let err = odb
                .as_ref()
                .open_read_stream(Oid::zero(kind).as_ref())
                .unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Odb));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn read_streams_read_what_was_written() {
    init();
    let dir = TempDir::new("read");
    for kind in TYPES {
        let odb = Odb::open_with_oid_type(&dir.objects(&format!("r{}", kind.raw())), kind).unwrap();
        for size in [0_usize, 1, 100, 70_000] {
            let data: Vec<u8> = (0..size).map(|i| u8::try_from(i % 251).unwrap()).collect();
            let id = oid(&write_blob(&odb, &data).unwrap());
            let mut opened = odb.as_ref().open_read_stream(id.as_ref()).unwrap();
            assert_eq!((opened.len, opened.kind), (size, Some(ObjectType::Blob)));
            let s = opened.stream.as_ref();
            assert!(s.can_read() && !s.can_write() && !s.can_finalize_write());
            assert!(s.has_hash_ctx());
            assert_eq!((s.mode(), s.declared_size(), s.received_bytes()), (0, 0, 0));
            assert!(s.oid_type().is_err());
            assert!(!s.backend().is_null());

            let mut back = Vec::new();
            let mut buf = [0_u8; 333];
            loop {
                let n = read(&mut opened.stream, &mut buf).unwrap();
                if n == 0 {
                    break;
                }
                back.extend_from_slice(&buf[..n]);
                // The handle again, between reads.
                assert!(opened.stream.as_ref().can_read());
            }
            assert_eq!(back, data);
        }
        // An object nobody wrote.
        let missing = Oid::from_raw(
            &[0x5A; 32][..if kind == OidType::Sha1 { 20 } else { 32 }],
            kind,
        )
        .unwrap();
        let err = odb.as_ref().open_read_stream(missing.as_ref()).unwrap_err();
        assert_eq!(err.code(), Some(ErrorCode::NotFound));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn many_streams_at_once_finish_out_of_order() {
    init();
    let dir = TempDir::new("many");
    let odb = Odb::open(&dir.objects("objects")).unwrap();
    let datas: Vec<Vec<u8>> = (0..8_u8).map(|i| vec![i; 100 + usize::from(i)]).collect();
    let mut streams: Vec<_> = datas
        .iter()
        .map(|d| {
            odb.as_ref()
                .open_write_stream(d.len() as u64, ObjectType::Blob)
                .unwrap()
        })
        .collect();
    for piece in 0..4 {
        for (stream, data) in streams.iter_mut().zip(&datas) {
            let q = data.len() / 4;
            let end = if piece == 3 {
                data.len()
            } else {
                (piece + 1) * q
            };
            assert_eq!(write(stream, &data[piece * q..end]), 0);
        }
    }
    let mut ids = Vec::new();
    for (stream, data) in streams.iter_mut().zip(&datas).rev() {
        let id = finalize(stream).unwrap();
        assert_eq!(
            read_header(&odb, &id),
            Ok((data.len(), ffi::GIT_OBJECT_BLOB))
        );
        ids.push(oid(&id));
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), datas.len());
    // Dropped in opening order, after storing.
    drop(streams);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn streams_of_a_repository_odb() {
    init();
    let dir = TempDir::new("repo");
    let odb = Odb::open(&dir.objects("objects")).unwrap();
    let repo = Repo::wrap(odb);
    let odb = repo.odb();
    let id = stream_blob(&odb, b"through the repository", 5);
    let opened = odb.as_ref().open_read_stream(oid(&id).as_ref()).unwrap();
    assert_eq!(opened.len, 22);
    let unfinished = odb.as_ref().open_write_stream(9, ObjectType::Blob).unwrap();
    drop(opened);
    drop(unfinished);
    drop(odb);
    drop(repo);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn threads_open_their_own_streams_on_one_odb() {
    init();
    let dir = TempDir::new("threads");
    let odb = Arc::new(Odb::open(&dir.objects("objects")).unwrap());
    std::thread::scope(|scope| {
        for t in 0..8_u8 {
            let odb: &OdbArc<'_> = &odb;
            scope.spawn(move || {
                for n in 0..16_u8 {
                    let data = vec![t.wrapping_mul(16).wrapping_add(n); 64 + usize::from(n)];
                    let id = stream_blob(odb, &data, 10);
                    let opened = odb.as_ref().open_read_stream(oid(&id).as_ref()).unwrap();
                    assert_eq!(opened.len, data.len());
                }
            });
        }
    });
}

/// A pack holding `blobs`, as `git pack-objects` writes it, from a scratch repository
/// under `dir`, over `kind` ids; the blobs' ids in hex.
fn git_pack(dir: &Path, blobs: &[&[u8]], kind: OidType) -> (Vec<u8>, Vec<String>) {
    let repo = dir.join(format!("git-{}", kind.raw()));
    let git = |args: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&repo).args(args);
        cmd.env("GIT_CONFIG_NOSYSTEM", "1").env("HOME", dir);
        cmd
    };
    let format = if kind == OidType::Sha1 {
        "sha1"
    } else {
        "sha256"
    };
    let status = Command::new("git")
        .args(["init", "-q", "--bare", &format!("--object-format={format}")])
        .arg(&repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .status()
        .unwrap();
    assert!(status.success());
    let mut ids = Vec::new();
    for blob in blobs {
        let mut child = git(&["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(blob).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        ids.push(String::from_utf8(out.stdout).unwrap().trim().to_owned());
    }
    let mut child = git(&["pack-objects", "-q", "--stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(ids.join("\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    (out.stdout, ids)
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2 and git")]
fn writepacks_take_a_pack_in_pieces_and_commit_it() {
    init();
    let dir = TempDir::new("pack");
    for kind in TYPES {
        let blobs: [&[u8]; 3] = [b"one", b"two two", &[7; 5000]];
        let (pack, ids) = git_pack(&dir.0, &blobs, kind);
        let odb = Odb::open_with_oid_type(&dir.objects(&format!("p{}", kind.raw())), kind).unwrap();
        for piece in [1, 7, 4096, pack.len()] {
            let mut wp = write_pack(&odb).unwrap();
            assert!(!wp.as_ref().backend().is_null());
            assert!(format!("{:?}", wp.as_mut()).starts_with("OdbWritepackRef"));
            let mut stats = IndexerProgress::default();
            for chunk in pack.chunks(piece) {
                wp.as_mut().append(chunk, stats.as_mut()).unwrap();
            }
            wp.as_mut().append(&[], stats.as_mut()).unwrap();
            assert_eq!(stats.as_ref().total_objects(), 3);
            OdbWritepack::commit(wp, stats.as_mut()).unwrap();
            assert_eq!(stats.as_ref().indexed_objects(), 3);
        }
        for (id, blob) in ids.iter().zip(blobs) {
            assert_eq!(
                read_header(&odb, &raw_oid(id, kind)),
                Ok((blob.len(), ffi::GIT_OBJECT_BLOB))
            );
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2 and git")]
fn writepacks_fail_and_discard() {
    init();
    let dir = TempDir::new("badpack");
    let (pack, _) = git_pack(&dir.0, &[b"only"], OidType::Sha1);
    let odb = Odb::open(&dir.objects("objects")).unwrap();
    let mut stats = IndexerProgress::default();

    // Dropped unfinished, empty and partly appended.
    drop(write_pack(&odb).unwrap());
    let mut wp = write_pack(&odb).unwrap();
    wp.as_mut()
        .append(&pack[..pack.len() / 2], stats.as_mut())
        .unwrap();
    drop(wp);

    // Committed without data, and with part of the pack.
    let wp = write_pack(&odb).unwrap();
    let err = OdbWritepack::commit(wp, stats.as_mut()).unwrap_err();
    assert_eq!(err.class(), Some(ErrorClass::Indexer));
    let mut wp = write_pack(&odb).unwrap();
    wp.as_mut()
        .append(&pack[..pack.len() - 3], stats.as_mut())
        .unwrap();
    assert!(OdbWritepack::commit(wp, stats.as_mut()).is_err());

    // Garbage, then more, then commit.
    let mut wp = write_pack(&odb).unwrap();
    assert!(
        wp.as_mut()
            .append(b"this is not a pack at all", stats.as_mut())
            .is_err()
    );
    drop(wp.as_mut().append(&pack, stats.as_mut()));
    assert!(OdbWritepack::commit(wp, stats.as_mut()).is_err());

    // An odb without a pack backend opens none.
    let empty = Odb::new().unwrap();
    assert!(write_pack(&empty).is_err());
}

/// A read stream is never opened for an id of another type than the odb's: libgit2 hands
/// the id to the loose backend unchecked, which formats it into a path sized for the odb's
/// type (`ASan` flags the overflow of a SHA-256 id in a SHA-1 odb; a SHA-1 id in a SHA-256
/// odb leaves part of the path unwritten). The refusal reaches no backend, and an id of the
/// odb's type still opens its stream afterwards.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn read_streams_refuse_ids_of_another_type() {
    init();
    let dir = TempDir::new("foreign");
    for kind in TYPES {
        let odb = Odb::open_with_oid_type(&dir.objects(&format!("f{}", kind.raw())), kind).unwrap();
        let id = write_blob(&odb, b"foreign").unwrap();
        for other in TYPES.into_iter().filter(|&other| other != kind) {
            let len = if other == OidType::Sha1 { 20 } else { 32 };
            let foreign = Oid::from_raw(&[0xAB; 32][..len], other).unwrap();
            for _ in 0..3 {
                let err = odb.as_ref().open_read_stream(foreign.as_ref()).unwrap_err();
                assert_eq!(err.code(), Some(ErrorCode::Generic));
                assert_eq!(err.class(), Some(ErrorClass::Invalid));
                assert!(
                    err.message()
                        .to_bytes()
                        .starts_with(b"specified object ID type")
                );
            }
        }
        let opened = odb.as_ref().open_read_stream(oid(&id).as_ref()).unwrap();
        assert_eq!(opened.len, 7);
        assert_eq!(opened.kind, Some(ObjectType::Blob));
        let mut stream = opened.stream;
        let mut buf = [0_u8; 16];
        assert_eq!(read(&mut stream, &mut buf), Ok(7));
        assert_eq!(&buf[..7], b"foreign");
    }
}
