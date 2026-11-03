//! Equivalence of `git2/indexer.h`'s progress callback with a C one.
//!
//! A [`ProgressCallback`] over a closure and a C callback (`git_indexer_progress_cb`)
//! written as libgit2's callers write one each drive the indexing of the same pack, on
//! independent odbs, through `git_odb_write_pack`'s writepack: the pack is appended in
//! pieces and committed. After each step, both sides must have returned the same code,
//! left the same progress, raised the same error, and seen the same progresses, call for
//! call; the packs they leave in their odbs must be the same. The closure and the C
//! callback cancel at the same call, from the first one through the last (a delta resolved
//! at commit), the closure with [`ControlFlow::Break`] and the C callback with `GIT_EUSER`,
//! or never.
//!
//! The Rust side reaches libgit2 through a C-ABI forwarder that hands the lent progress to
//! [`ProgressCallback::call`], since no wrapper of a routine taking the callback exists yet;
//! `call` runs the trampoline libgit2 would call.

use std::ffi::{CStr, CString, c_int, c_void};
use std::io::Write as _;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{fs, process, ptr};

use ffibox::{CMutSeam, CRefSeam, CZeroable};
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::indexer::{
    IndexerProgress, IndexerProgressRef, ProgressCallback,
};

/// Takes a `git_libgit2_init` reference that is never released.
fn init() {
    let _ = core::mem::ManuallyDrop::new(libgit2::libgit2::src::libgit2::libgit2::init().unwrap());
}

/// The thread's last libgit2 error: its class and message.
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
    let message = unsafe { CStr::from_ptr(message) };
    (klass, message.to_string_lossy().into_owned())
}

/// The counters of a progress, in field order.
type Counters = [usize; 7];

fn counters(p: IndexerProgressRef<'_>) -> Counters {
    [
        p.total_objects() as usize,
        p.indexed_objects() as usize,
        p.received_objects() as usize,
        p.local_objects() as usize,
        p.total_deltas() as usize,
        p.indexed_deltas() as usize,
        p.received_bytes(),
    ]
}

/// A scratch directory of this process for `tag`, created empty, with an `objects` dir
/// for each side.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("equiv-indexer-{tag}-{}", process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        for side in ["c", "rust"] {
            fs::create_dir_all(dir.join(side).join("pack")).unwrap();
        }
        Self(dir)
    }

    fn objects(&self, side: &str) -> CString {
        CString::new(self.0.join(side).to_str().unwrap()).unwrap()
    }

    /// The file names in `side`'s `pack` dir, sorted.
    fn packs(&self, side: &str) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.0.join(side).join("pack"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _unused = fs::remove_dir_all(&self.0);
    }
}

/// A pack holding three blobs, two of them alike enough for one to be stored as a delta,
/// from `git pack-objects` over SHA-1 ids.
fn git_pack(dir: &Path) -> Vec<u8> {
    let base: Vec<u8> = (0..4096u32)
        .flat_map(|i| format!("line {i}\n").into_bytes())
        .collect();
    let mut edited = base.clone();
    edited.extend_from_slice(b"one more line\n");
    let blobs = [base, edited, b"hello".to_vec()];
    let repo = dir.join("git");
    let git = |args: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", dir);
        cmd
    };
    let repo = repo.to_str().unwrap();
    assert!(
        git(&["init", "-q", "--bare", repo])
            .status()
            .unwrap()
            .success()
    );
    let mut ids = Vec::new();
    for blob in &blobs {
        let mut child = git(&["-C", repo, "hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(blob).unwrap();
        let out = child.wait_with_output().unwrap();
        ids.push(String::from_utf8(out.stdout).unwrap().trim().to_owned());
    }
    let mut child = git(&["-C", repo, "pack-objects", "-q", "--stdout"])
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
    child.wait_with_output().unwrap().stdout
}

/// One step of an indexing: the member's code, the progress it left, the error it raised
/// (when it failed) and how many progresses the callback had seen by then.
#[derive(Debug, PartialEq, Eq)]
struct Step {
    rc: c_int,
    stats: Counters,
    error: Option<(c_int, String)>,
    calls: usize,
}

/// Indexes `pack` through a writepack of the odb at `objects`, with `callback` and
/// `payload`: appends it in pieces of `piece` bytes, then commits, stopping at the first
/// failure. `calls` reads how many progresses the callback has seen.
fn index(
    objects: &CStr,
    pack: &[u8],
    piece: usize,
    callback: ffi::git_indexer_progress_cb,
    payload: *mut c_void,
    calls: &dyn Fn() -> usize,
) -> Vec<Step> {
    let mut odb = ptr::null_mut();
    // SAFETY: `odb` is a writable slot; `objects` is NUL-terminated, read for the call.
    let rc = unsafe { ffi::git_odb_open(&raw mut odb, objects.as_ptr()) };
    assert_eq!(rc, 0);
    let mut wp = ptr::null_mut();
    // SAFETY: `wp` is a writable slot; `odb` is live; the callback and payload stay valid
    // until the writepack is freed below.
    let rc = unsafe { ffi::git_odb_write_pack(&raw mut wp, odb, callback, payload) };
    assert_eq!(rc, 0);
    // SAFETY: `wp` is the live writepack libgit2 opened, with its members set.
    let members = unsafe { wp.read() };
    let (append, commit, free) = (
        members.append.unwrap(),
        members.commit.unwrap(),
        members.free.unwrap(),
    );
    let mut stats = IndexerProgress::zeroed();
    let mut steps = Vec::new();
    let mut record = |rc: c_int, stats: &IndexerProgress| {
        let error = (rc != 0).then(c_last);
        steps.push(Step {
            rc,
            stats: counters(stats.as_ref()),
            error,
            calls: calls(),
        });
        rc == 0
    };
    let mut ok = true;
    for chunk in pack.chunks(piece) {
        // SAFETY: a member of the live `wp`; `chunk` is read for the call, `stats` is a
        // live progress written for the call only.
        let rc = unsafe {
            append(
                wp,
                chunk.as_ptr().cast(),
                chunk.len(),
                stats.as_mut().as_mut_ptr(),
            )
        };
        ok = record(rc, &stats);
        if !ok {
            break;
        }
    }
    if ok {
        // SAFETY: as for `append`.
        let rc = unsafe { commit(wp, stats.as_mut().as_mut_ptr()) };
        record(rc, &stats);
    }
    // SAFETY: `wp` is live and no longer used.
    unsafe { free(wp) };
    // SAFETY: `odb` is live and no longer used.
    unsafe { ffi::git_odb_free(odb) };
    steps
}

/// The C side's payload: the progresses seen, and the call to cancel at.
struct CLog {
    seen: Vec<Counters>,
    stop_at: Option<usize>,
}

/// A C callback as libgit2's callers write one: records the progress, and cancels with
/// `GIT_EUSER` at the call its payload names.
unsafe extern "C" fn c_record(
    stats: *const ffi::git_indexer_progress,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: `payload` is the `CLog` the indexing was given, used by nothing else meanwhile.
    let log = unsafe { &mut *payload.cast::<CLog>() };
    // SAFETY: libgit2 lends a live progress for the call.
    let progress = unsafe { IndexerProgressRef::from_ptr(stats.cast_mut()) }.unwrap();
    log.seen.push(counters(progress));
    if log.stop_at == Some(log.seen.len()) {
        ffi::GIT_EUSER
    } else {
        0
    }
}

/// The Rust side's forwarder: hands the lent progress to the [`ProgressCallback`] its
/// payload addresses, and returns what the callback returns libgit2.
unsafe extern "C" fn forward(
    stats: *const ffi::git_indexer_progress,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: `payload` is the callback the indexing was given, used by nothing else
    // meanwhile.
    let callback = unsafe { &mut *payload.cast::<ProgressCallback<'_>>() };
    // SAFETY: libgit2 lends a live progress for the call.
    let progress = unsafe { IndexerProgressRef::from_ptr(stats.cast_mut()) }.unwrap();
    match callback.call(progress) {
        ControlFlow::Continue(()) => 0,
        ControlFlow::Break(()) => ffi::GIT_EUSER,
    }
}

/// Runs both sides over `pack`, cancelling at `stop_at`, and compares them; returns how
/// many progresses the C callback saw.
fn compare(scratch: &Scratch, pack: &[u8], piece: usize, stop_at: Option<usize>) -> usize {
    let mut log = CLog {
        seen: Vec::new(),
        stop_at,
    };
    let log_ptr: *mut CLog = &raw mut log;
    // SAFETY: the callback is done with `log` whenever `calls` reads it.
    let c_calls = || unsafe { (*log_ptr).seen.len() };
    let c_steps = index(
        &scratch.objects("c"),
        pack,
        piece,
        Some(c_record),
        log_ptr.cast(),
        &c_calls,
    );

    let mut seen = Vec::new();
    let seen_ptr: *mut Vec<Counters> = &raw mut seen;
    let mut closure = |progress: IndexerProgressRef<'_>| {
        // SAFETY: nothing else touches `seen` while libgit2 calls the closure.
        let seen = unsafe { &mut *seen_ptr };
        seen.push(counters(progress));
        if stop_at == Some(seen.len()) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let mut callback = ProgressCallback::new(&mut closure);
    // SAFETY: the closure is done with `seen` whenever `calls` reads it.
    let rust_calls = || unsafe { (*seen_ptr).len() };
    let rust_steps = index(
        &scratch.objects("rust"),
        pack,
        piece,
        Some(forward),
        (&raw mut callback).cast(),
        &rust_calls,
    );
    drop(callback);

    assert_eq!(rust_steps, c_steps, "steps, stopping at {stop_at:?}");
    assert_eq!(seen, log.seen, "progresses seen, stopping at {stop_at:?}");
    assert_eq!(scratch.packs("rust"), scratch.packs("c"));
    log.seen.len()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn a_progress_callback_sees_and_cancels_as_a_c_callback() {
    init();
    let scratch = Scratch::new("progress");
    let pack = git_pack(&scratch.0);
    for piece in [pack.len(), 4096, 61] {
        let scratch = Scratch::new(&format!("progress-{piece}"));
        let total = compare(&scratch, &pack, piece, None);
        assert!(total >= 4, "a call per object and one per resolved delta");
        assert_eq!(scratch.packs("c").len(), 2, "the pack and its index");
        for stop_at in [1, 2, total] {
            let scratch = Scratch::new(&format!("progress-{piece}-{stop_at}"));
            compare(&scratch, &pack, piece, Some(stop_at));
            assert!(
                scratch.packs("c").is_empty(),
                "a cancelled pack is not kept"
            );
        }
    }
}
