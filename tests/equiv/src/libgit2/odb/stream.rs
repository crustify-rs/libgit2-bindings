//! Equivalence of `git2/odb_backend.h`'s streams and writepacks with libgit2's.
//!
//! Each stream constructor ([`OdbRef::open_write_stream`], [`OdbRef::open_read_stream`])
//! runs beside the C routine it wraps (`git_odb_open_wstream`, `git_odb_open_rstream`)
//! on independent inputs: two `objects` directories laid out alike. A failure must
//! report libgit2's code and error; a success must give a stream whose getters read
//! what C reads from its fields, after every step of a sequence: writes in pieces, a
//! refused overwrite, the stored id, the bytes read back. A writepack's
//! [`append`](OdbWritepackMut::append) and [`OdbWritepack::commit`] run beside C calls
//! of its members on two writepacks over two odbs, comparing the result and the
//! progress after each step, and what the odb holds at the end.
//!
//! [`OdbRef::open_write_stream`]: libgit2::libgit2::src::libgit2::odb::OdbRef::open_write_stream
//! [`OdbRef::open_read_stream`]: libgit2::libgit2::src::libgit2::odb::OdbRef::open_read_stream
//! [`OdbWritepackMut::append`]: libgit2::libgit2::src::libgit2::odb::OdbWritepackMut::append

use std::ffi::{CStr, CString, c_int};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{fs, ptr};

use ffibox::CRefSeam;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::filter::StreamError;
use libgit2::libgit2::src::libgit2::indexer::IndexerProgress;
use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::odb::{
    Odb, OdbArc, OdbStreamBox, OdbStreamRef, OdbWritepack, OdbWritepackBox,
};
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType, UnnamedOidType};

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    let _ = core::mem::ManuallyDrop::new(libgit2::libgit2::src::libgit2::libgit2::init().unwrap());
}

/// The thread's last libgit2 error, read through C: its class and message.
fn c_last() -> (c_int, String) {
    // SAFETY: libgit2 is initialised; the result is a static error or this thread's
    // error state, live until the next libgit2 call on this thread.
    let e = unsafe { ffi::git_error_last() };
    assert!(!e.is_null());
    // SAFETY: `e` is live (above); its fields are copied out before any other call.
    let klass = unsafe { (*e).klass };
    // SAFETY: as above.
    let message = unsafe { (*e).message };
    // SAFETY: as above, a NUL-terminated message.
    (
        klass,
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// A C failure: the code, and the thread's last error.
fn c_fail(rc: c_int) -> (c_int, c_int, String) {
    let (class, message) = c_last();
    (rc, class, message)
}

/// A Rust failure as the triple [`c_fail`] reads.
fn rust_fail(err: &StreamError) -> (c_int, c_int, String) {
    (
        err.raw_code(),
        err.raw_class(),
        err.message().to_string_lossy().into_owned(),
    )
}

/// A directory of its own under the temporary directory, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crustify-odb-stream-equiv-{}-{tag}",
            std::process::id()
        ));
        drop(fs::remove_dir_all(&path));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// An `objects` directory under the directory, with its `pack` subdirectory.
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

/// An odb C owns, freed on drop.
struct COdb(*mut ffi::git_odb);

impl COdb {
    fn open(dir: &CStr, kind: OidType) -> Self {
        let opts = ffi::git_odb_options {
            version: ffi::GIT_ODB_OPTIONS_VERSION,
            oid_type: kind.raw(),
        };
        let mut out = ptr::null_mut();
        // SAFETY: `out` is writable; `dir` and `opts` are live for the call.
        let rc = unsafe { ffi::git_odb_open_ext(&raw mut out, dir.as_ptr(), &raw const opts) };
        assert_eq!(rc, 0);
        Self(out)
    }

    fn new(kind: OidType) -> Self {
        let opts = ffi::git_odb_options {
            version: ffi::GIT_ODB_OPTIONS_VERSION,
            oid_type: kind.raw(),
        };
        let mut out = ptr::null_mut();
        // SAFETY: `out` is writable; `opts` is live for the call.
        let rc = unsafe { ffi::git_odb_new_ext(&raw mut out, &raw const opts) };
        assert_eq!(rc, 0);
        Self(out)
    }
}

impl Drop for COdb {
    fn drop(&mut self) {
        // SAFETY: the odb this owner holds, freed once.
        unsafe { ffi::git_odb_free(self.0) }
    }
}

/// A stream C owns, freed with `git_odb_stream_free` on drop.
struct CStream(*mut ffi::git_odb_stream);

impl Drop for CStream {
    fn drop(&mut self) {
        // SAFETY: the stream this owner holds, freed once.
        unsafe { ffi::git_odb_stream_free(self.0) }
    }
}

/// What a stream's fields hold, as the getters publish them.
#[derive(Debug, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per field the getters test"
)]
struct Fields {
    backend_set: bool,
    mode: u32,
    hash_ctx: bool,
    oid_type: u32,
    declared_size: u64,
    received_bytes: u64,
    read: bool,
    write: bool,
    finalize_write: bool,
}

/// The fields of C's stream, read directly.
fn c_fields(s: *mut ffi::git_odb_stream) -> Fields {
    // SAFETY: `s` is a live stream libgit2 opened; it is copied out, with no reference
    // held.
    let raw = unsafe { s.read() };
    Fields {
        backend_set: !raw.backend.is_null(),
        mode: raw.mode,
        hash_ctx: !raw.hash_ctx.is_null(),
        oid_type: raw.oid_type,
        declared_size: raw.declared_size,
        received_bytes: raw.received_bytes,
        read: raw.read.is_some(),
        write: raw.write.is_some(),
        finalize_write: raw.finalize_write.is_some(),
    }
}

/// The fields of the Rust stream, through its getters.
fn rust_fields(s: OdbStreamRef<'_, '_>) -> Fields {
    Fields {
        backend_set: s.backend().is_some(),
        mode: s.mode(),
        hash_ctx: s.has_hash_ctx(),
        oid_type: s.oid_type().map_or_else(UnnamedOidType::raw, OidType::raw),
        declared_size: s.declared_size(),
        received_bytes: s.received_bytes(),
        read: s.can_read(),
        write: s.can_write(),
        finalize_write: s.can_finalize_write(),
    }
}

/// `git_odb_stream_write`, on either stream: the code.
fn write(s: *mut ffi::git_odb_stream, data: &[u8]) -> c_int {
    // SAFETY: `s` is a live write stream; `data` is read for the call only.
    unsafe { ffi::git_odb_stream_write(s, data.as_ptr().cast(), data.len()) }
}

/// `git_odb_stream_finalize_write`, on either stream: the id, or the code.
fn finalize(s: *mut ffi::git_odb_stream) -> Result<(u8, [u8; 32]), c_int> {
    let mut id = ffi::git_oid::default();
    // SAFETY: `s` is a live write stream; `id` is writable.
    let rc = unsafe { ffi::git_odb_stream_finalize_write(&raw mut id, s) };
    if rc == 0 {
        Ok((id.type_, id.id))
    } else {
        Err(rc)
    }
}

/// Reads a read stream to its end in pieces of `piece` bytes: the content, or the code.
fn read_all(s: *mut ffi::git_odb_stream, piece: usize) -> Result<Vec<u8>, c_int> {
    let mut out = Vec::new();
    let mut buf = vec![0_u8; piece];
    loop {
        // SAFETY: `s` is a live read stream; `buf` is writable for its length.
        let rc = unsafe { ffi::git_odb_stream_read(s, buf.as_mut_ptr().cast(), buf.len()) };
        match usize::try_from(rc) {
            Ok(0) => return Ok(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(_) => return Err(rc),
        }
    }
}

/// `git_odb_open_wstream` on C's odb.
fn c_open_wstream(
    odb: &COdb,
    size: u64,
    kind: ObjectType,
) -> Result<CStream, (c_int, c_int, String)> {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is writable; the odb is live.
    let rc = unsafe { ffi::git_odb_open_wstream(&raw mut out, odb.0, size, kind.raw()) };
    if rc < 0 {
        Err(c_fail(rc))
    } else {
        Ok(CStream(out))
    }
}

/// `git_odb_open_rstream` on C's odb: the stream, length and type.
fn c_open_rstream(
    odb: &COdb,
    id: &Oid,
) -> Result<(CStream, usize, ffi::git_object_t), (c_int, c_int, String)> {
    let (mut out, mut len, mut kind) = (ptr::null_mut(), 0, ffi::GIT_OBJECT_INVALID);
    // SAFETY: the out slots are writable; the odb is live; `id` is a live `git_oid`.
    let rc = unsafe {
        ffi::git_odb_open_rstream(
            &raw mut out,
            &raw mut len,
            &raw mut kind,
            odb.0,
            id.as_ref().as_ptr(),
        )
    };
    if rc < 0 {
        Err(c_fail(rc))
    } else {
        Ok((CStream(out), len, kind))
    }
}

/// Runs the same sequence on a C and a Rust write stream, comparing after each step;
/// the stored id.
fn compare_write_sequence(
    c: &CStream,
    rust: &mut OdbStreamBox<'_>,
    data: &[u8],
    piece: usize,
) -> Result<(u8, [u8; 32]), c_int> {
    let r = rust.as_c_ptr();
    assert_eq!(rust_fields(rust.as_ref()), c_fields(c.0));
    for chunk in data.chunks(piece.max(1)) {
        assert_eq!(write(r, chunk), write(c.0, chunk));
        assert_eq!(rust_fields(rust.as_ref()), c_fields(c.0));
    }
    let id = finalize(r);
    assert_eq!(id, finalize(c.0));
    assert_eq!(rust_fields(rust.as_ref()), c_fields(c.0));
    id
}

const TYPES: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_write_stream_opens_what_git_odb_open_wstream_opens() {
    init();
    let dir = TempDir::new("write");
    for kind in TYPES {
        let c = COdb::open(&dir.objects(&format!("c{}", kind.raw())), kind);
        let rust =
            Odb::open_with_oid_type(&dir.objects(&format!("r{}", kind.raw())), kind).unwrap();
        for (size, piece) in [(0_usize, 1), (1, 1), (5, 2), (4096, 1000), (70_000, 70_000)] {
            let data: Vec<u8> = (0..size).map(|i| u8::try_from(i % 241).unwrap()).collect();
            for kind in [ObjectType::Blob, ObjectType::Tree] {
                let cs = c_open_wstream(&c, size as u64, kind).unwrap();
                let mut rs = rust.as_ref().open_write_stream(size as u64, kind).unwrap();
                compare_write_sequence(&cs, &mut rs, &data, piece).unwrap();
            }
            // Short, then overwritten: the same refusals, counted alike.
            let cs = c_open_wstream(&c, size as u64, ObjectType::Blob).unwrap();
            let rs = rust
                .as_ref()
                .open_write_stream(size as u64, ObjectType::Blob)
                .unwrap();
            let r = rs.as_c_ptr();
            assert_eq!(write(r, &data[..size / 2]), write(cs.0, &data[..size / 2]));
            let rf = finalize(r);
            let cf = finalize(cs.0);
            assert_eq!(rf, cf);
            if rf.is_err() {
                assert_eq!(c_last(), c_last());
            }
            let extra = [b'z'; 3];
            let tail = &data[size / 2..];
            assert_eq!(write(r, tail), write(cs.0, tail));
            let rw = write(r, &extra);
            let r_err = c_last();
            assert_eq!(rw, write(cs.0, &extra));
            assert_eq!(r_err, c_last());
            assert_eq!(rust_fields(rs.as_ref()), c_fields(cs.0));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_write_stream_fails_as_git_odb_open_wstream_fails() {
    init();
    for kind in TYPES {
        let c = COdb::new(kind);
        let rust = Odb::with_oid_type(kind).unwrap();
        let c_err = c_open_wstream(&c, 3, ObjectType::Blob)
            .map(|_| ())
            .unwrap_err();
        let r_err = rust
            .as_ref()
            .open_write_stream(3, ObjectType::Blob)
            .unwrap_err();
        assert_eq!(rust_fail(&r_err), c_err);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn open_read_stream_opens_what_git_odb_open_rstream_opens() {
    init();
    let dir = TempDir::new("read");
    for kind in TYPES {
        let c = COdb::open(&dir.objects(&format!("c{}", kind.raw())), kind);
        let rust =
            Odb::open_with_oid_type(&dir.objects(&format!("r{}", kind.raw())), kind).unwrap();
        for size in [0_usize, 3, 1000, 70_000] {
            let data: Vec<u8> = (0..size).map(|i| u8::try_from(i % 239).unwrap()).collect();
            // The object, written alike into both odbs through write streams.
            let cs = c_open_wstream(&c, size as u64, ObjectType::Blob).unwrap();
            let mut rs = rust
                .as_ref()
                .open_write_stream(size as u64, ObjectType::Blob)
                .unwrap();
            let (ty, raw) = compare_write_sequence(&cs, &mut rs, &data, 4096).unwrap();
            drop((cs, rs));
            let len = if kind == OidType::Sha1 { 20 } else { 32 };
            assert_eq!(u32::from(ty), kind.raw());
            let id = Oid::from_raw(&raw[..len], kind).unwrap();

            let (cs, c_len, c_kind) = c_open_rstream(&c, &id).unwrap();
            let opened = rust.as_ref().open_read_stream(id.as_ref()).unwrap();
            assert_eq!(opened.len, c_len);
            assert_eq!(opened.kind.map(ObjectType::raw), Some(c_kind));
            assert_eq!(rust_fields(opened.stream.as_ref()), c_fields(cs.0));
            for piece in [7, 100_000] {
                if piece == 100_000 {
                    // A fresh pair, read whole.
                    let (cs, _, _) = c_open_rstream(&c, &id).unwrap();
                    let opened = rust.as_ref().open_read_stream(id.as_ref()).unwrap();
                    assert_eq!(
                        read_all(opened.stream.as_c_ptr(), piece),
                        read_all(cs.0, piece)
                    );
                    assert_eq!(rust_fields(opened.stream.as_ref()), c_fields(cs.0));
                } else {
                    let r = read_all(opened.stream.as_c_ptr(), piece);
                    assert_eq!(r, read_all(cs.0, piece));
                    assert_eq!(r.unwrap(), data);
                    assert_eq!(rust_fields(opened.stream.as_ref()), c_fields(cs.0));
                }
            }
        }
        // Missing, and on an odb without backends.
        let missing = Oid::from_raw(
            &[0xA5; 32][..if kind == OidType::Sha1 { 20 } else { 32 }],
            kind,
        )
        .unwrap();
        let c_err = c_open_rstream(&c, &missing).map(|_| ()).unwrap_err();
        let r_err = rust
            .as_ref()
            .open_read_stream(missing.as_ref())
            .unwrap_err();
        assert_eq!(rust_fail(&r_err).0, c_err.0);
        assert_eq!(rust_fail(&r_err).1, c_err.1);
        let c_empty = COdb::new(kind);
        let r_empty = Odb::with_oid_type(kind).unwrap();
        let c_err = c_open_rstream(&c_empty, &missing).map(|_| ()).unwrap_err();
        let r_err = r_empty
            .as_ref()
            .open_read_stream(missing.as_ref())
            .unwrap_err();
        assert_eq!(rust_fail(&r_err), c_err);
    }
}

/// A pack holding `blobs`, from `git pack-objects`, over `kind` ids; the blobs' ids in
/// hex.
fn git_pack(dir: &Path, blobs: &[&[u8]], kind: OidType) -> (Vec<u8>, Vec<String>) {
    let repo = dir.join(format!("git-{}", kind.raw()));
    let format = if kind == OidType::Sha1 {
        "sha1"
    } else {
        "sha256"
    };
    let git = |args: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir);
        cmd
    };
    let repo_s = repo.to_str().unwrap();
    assert!(
        git(&[
            "init",
            "-q",
            "--bare",
            &format!("--object-format={format}"),
            repo_s
        ])
        .status()
        .unwrap()
        .success()
    );
    let mut ids = Vec::new();
    for blob in blobs {
        let mut child = git(&["-C", repo_s, "hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(blob).unwrap();
        let out = child.wait_with_output().unwrap();
        ids.push(String::from_utf8(out.stdout).unwrap().trim().to_owned());
    }
    let mut child = git(&["-C", repo_s, "pack-objects", "-q", "--stdout"])
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
    (child.wait_with_output().unwrap().stdout, ids)
}

/// `git_odb_write_pack` with no progress callback, on a raw odb.
fn raw_write_pack(odb: *mut ffi::git_odb) -> *mut ffi::git_odb_writepack {
    let mut out = ptr::null_mut();
    // SAFETY: `out` is writable; the odb is live; no callback, so no payload.
    let rc = unsafe { ffi::git_odb_write_pack(&raw mut out, odb, None, ptr::null_mut()) };
    assert_eq!(rc, 0);
    assert!(!out.is_null());
    out
}

/// C's `append` member call.
fn c_append(
    wp: *mut ffi::git_odb_writepack,
    data: &[u8],
    stats: &mut ffi::git_indexer_progress,
) -> c_int {
    // SAFETY: `wp` is a live writepack libgit2 opened, its members set.
    let append = unsafe { (*wp).append }.unwrap();
    // SAFETY: the member of `wp`; `data` is read for the call, `stats` writable.
    unsafe { append(wp, data.as_ptr().cast(), data.len(), stats) }
}

/// C's `commit` member call, then its `free` member.
fn c_commit(wp: *mut ffi::git_odb_writepack, stats: &mut ffi::git_indexer_progress) -> c_int {
    // SAFETY: as in `c_append`.
    let commit = unsafe { (*wp).commit }.unwrap();
    // SAFETY: as above.
    let free = unsafe { (*wp).free }.unwrap();
    // SAFETY: the member of `wp`, committed once.
    let rc = unsafe { commit(wp, stats) };
    // SAFETY: the member of `wp`, which is freed once, after the commit.
    unsafe { free(wp) };
    rc
}

/// The Rust writepack's progress as C's struct.
fn progress(p: &IndexerProgress) -> ffi::git_indexer_progress {
    ffi::git_indexer_progress::from(*p)
}

/// The progress as comparable counters.
fn counters(p: &ffi::git_indexer_progress) -> [usize; 7] {
    [
        p.total_objects as usize,
        p.indexed_objects as usize,
        p.received_objects as usize,
        p.local_objects as usize,
        p.total_deltas as usize,
        p.indexed_deltas as usize,
        p.received_bytes,
    ]
}

/// Appends `chunks` to both writepacks and commits both (unless `commit` is false, when
/// both are dropped), comparing after each step.
fn compare_pack_sequence(
    c: *mut ffi::git_odb_writepack,
    mut rust: OdbWritepackBox<'_>,
    chunks: &[&[u8]],
    commit: bool,
) {
    let mut c_stats = ffi::git_indexer_progress::default();
    let mut r_stats = IndexerProgress::default();
    for chunk in chunks {
        let r = rust.as_mut().append(chunk, r_stats.as_mut());
        let r = r.map_err(|e| rust_fail(&e));
        let rc = c_append(c, chunk, &mut c_stats);
        let c_res = if rc < 0 { Err(c_fail(rc)) } else { Ok(()) };
        assert_eq!(r, c_res);
        assert_eq!(counters(&progress(&r_stats)), counters(&c_stats));
    }
    if commit {
        let r = OdbWritepack::commit(rust, r_stats.as_mut()).map_err(|e| rust_fail(&e));
        let rc = c_commit(c, &mut c_stats);
        let c_res = if rc < 0 { Err(c_fail(rc)) } else { Ok(()) };
        assert_eq!(r, c_res);
        assert_eq!(counters(&progress(&r_stats)), counters(&c_stats));
    } else {
        drop(rust);
        // SAFETY: the writepack C opened, live, its members set.
        let free = unsafe { (*c).free }.unwrap();
        // SAFETY: its member, freeing it once.
        unsafe { free(c) };
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn writepack_members_behave_as_in_c() {
    init();
    let dir = TempDir::new("pack");
    for kind in TYPES {
        let blobs: [&[u8]; 3] = [b"alpha", b"beta beta", &[3; 9000]];
        let (pack, ids) = git_pack(&dir.0, &blobs, kind);
        let c = COdb::open(&dir.objects(&format!("c{}", kind.raw())), kind);
        let rust =
            Odb::open_with_oid_type(&dir.objects(&format!("r{}", kind.raw())), kind).unwrap();
        let open = |odb: &OdbArc<'_>| -> OdbWritepackBox<'_> {
            // SAFETY: the writepack libgit2 opened for this odb, with its members set,
            // owned by the caller; it borrows the odb.
            unsafe { OdbWritepackBox::from_c(raw_write_pack(odb.as_c_ptr())) }.unwrap()
        };
        let halves: Vec<&[u8]> = pack.chunks(pack.len() / 2 + 1).collect();
        let pieces: Vec<&[u8]> = pack.chunks(13).collect();
        // Whole, in halves, in small pieces with an empty one.
        compare_pack_sequence(raw_write_pack(c.0), open(&rust), &[&pack], true);
        compare_pack_sequence(raw_write_pack(c.0), open(&rust), &halves, true);
        let mut with_empty = pieces.clone();
        with_empty.insert(3, &[]);
        compare_pack_sequence(raw_write_pack(c.0), open(&rust), &with_empty, true);
        // Failures: no data, truncated, garbage first; and dropped unfinished.
        compare_pack_sequence(raw_write_pack(c.0), open(&rust), &[], true);
        compare_pack_sequence(
            raw_write_pack(c.0),
            open(&rust),
            &[&pack[..pack.len() - 5]],
            true,
        );
        compare_pack_sequence(
            raw_write_pack(c.0),
            open(&rust),
            &[b"garbage, not a pack", &pack],
            true,
        );
        compare_pack_sequence(raw_write_pack(c.0), open(&rust), &halves[..1], false);
        // Both odbs hold the pack's objects.
        for hex in ids {
            let id = Oid::from_string(&CString::new(hex).unwrap(), kind).unwrap();
            let id = id.as_ref().as_ptr();
            // SAFETY: the odb is live; `id` is a live `git_oid`, read for the call.
            let c_has = unsafe { ffi::git_odb_exists(c.0, id) };
            // SAFETY: as above.
            let r_has = unsafe { ffi::git_odb_exists(rust.as_c_ptr(), id) };
            assert_eq!((c_has, r_has), (1, 1));
        }
    }
}
