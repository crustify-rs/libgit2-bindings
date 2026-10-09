//! UB tests for the unsafe API of `git2/trace.h`: `TraceCallback::from_fn`, called
//! within its contract, edge cases included.
//!
//! Its contract is that the adopted C function may be called with any named level
//! and any C string live for the call, from any thread, concurrently and reentrantly,
//! for the rest of the process. The functions adopted here meet it, so calls through
//! the callback must race nothing (TSan), read the message only while it lives
//! (ASan), and reach the function exactly as a direct call does; libgit2's own calls,
//! once it is installed, too, even after every `TraceCallback` is gone.

use std::ffi::{CStr, CString, c_char, c_int};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::thread;

use libgit2::ffi;
use libgit2::libgit2::core::trace::{TraceCallback, TraceLevel, Tracer};

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process, where no other thread touches libgit2's trace
/// state.
fn isolated(test: &str, body: fn()) {
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

static LOG: Mutex<Vec<(ffi::git_trace_level_t, Vec<u8>)>> = Mutex::new(Vec::new());
static CALLS: AtomicUsize = AtomicUsize::new(0);

/// Copies the message into a locked log: callable from any thread, at once, and
/// reads `msg` only during the call.
unsafe extern "C" fn c_logging(level: ffi::git_trace_level_t, msg: *const c_char) {
    // SAFETY: `TraceFn`'s contract: `msg` is a C string live for the call.
    let bytes = unsafe { CStr::from_ptr(msg) }.to_bytes().to_vec();
    LOG.lock().unwrap().push((level, bytes));
}

/// Counts calls with an atomic and reads every byte of the message.
unsafe extern "C" fn c_counting(_: ffi::git_trace_level_t, msg: *const c_char) {
    // SAFETY: as above.
    let len = unsafe { CStr::from_ptr(msg) }.to_bytes().len();
    CALLS.fetch_add(1, SeqCst);
    std::hint::black_box(len);
}

const LEVELS: [TraceLevel; 7] = [
    TraceLevel::None,
    TraceLevel::Fatal,
    TraceLevel::Error,
    TraceLevel::Warn,
    TraceLevel::Info,
    TraceLevel::Debug,
    TraceLevel::Trace,
];

fn take_mine(tag: &[u8]) -> Vec<(ffi::git_trace_level_t, Vec<u8>)> {
    let mut log = LOG.lock().unwrap();
    let (mine, rest) = log.drain(..).partition(|(_, m)| m.starts_with(tag));
    *log = rest;
    mine
}

#[test]
fn from_fn_calls_reach_the_function_as_direct_calls_do() {
    // SAFETY: `c_logging` meets the contract (see its doc).
    let cb = unsafe { TraceCallback::from_fn(c_logging) };
    for level in LEVELS {
        for n in [0usize, 1, 100, 4096] {
            let message = CString::new([b"direct:".as_slice(), &vec![b'z'; n]].concat()).unwrap();
            cb.call(level, &message);
            let through = take_mine(b"direct:");
            // SAFETY: a named level and a C string live for the call.
            unsafe { c_logging(level.raw(), message.as_ptr()) };
            drop(message);
            assert_eq!(take_mine(b"direct:"), through);
            assert_eq!(through.len(), 1);
            assert_eq!(through[0].0, level.raw());
        }
    }
}

#[test]
fn from_fn_calls_from_many_threads_race_nothing() {
    // SAFETY: `c_counting` meets the contract (see its doc).
    let cb = unsafe { TraceCallback::from_fn(c_counting) };
    let (threads, per) = if cfg!(any(miri, bsan)) {
        (3, 4)
    } else {
        (8, 200)
    };
    let before = CALLS.load(SeqCst);
    thread::scope(|s| {
        for t in 0..threads {
            s.spawn(move || {
                for i in 0..per {
                    let message = CString::new(format!("{t}:{i}")).unwrap();
                    cb.call(LEVELS[i % LEVELS.len()], &message);
                }
            });
        }
    });
    assert!(CALLS.load(SeqCst) - before >= threads * per);
}

thread_local! {
    static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Calls back into the callback adopted over itself, two levels deep.
unsafe extern "C" fn c_reentrant(level: ffi::git_trace_level_t, msg: *const c_char) {
    let depth = DEPTH.get();
    if depth < 2 {
        DEPTH.set(depth + 1);
        // SAFETY: this function meets the contract: it reads `msg` only during the
        // call, keeps per-thread state, and nests to a fixed depth.
        let cb = unsafe { TraceCallback::from_fn(c_reentrant) };
        let inner = CString::new(format!("reentrant:inner {depth}")).unwrap();
        cb.call(TraceLevel::from_raw(level).unwrap(), &inner);
        DEPTH.set(depth);
    }
    // SAFETY: the outer message is still live: the nested call borrowed its own.
    unsafe { c_logging(level, msg) };
}

#[test]
fn from_fn_reentrant_calls_nest() {
    // SAFETY: `c_reentrant` meets the contract (see its doc).
    let cb = unsafe { TraceCallback::from_fn(c_reentrant) };
    cb.call(TraceLevel::Error, c"reentrant:outer");
    let messages: Vec<Vec<u8>> = take_mine(b"reentrant:")
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    assert_eq!(
        messages,
        [
            b"reentrant:inner 1".to_vec(),
            b"reentrant:inner 0".to_vec(),
            b"reentrant:outer".to_vec()
        ]
    );
}

/// A tracer adopted back through `from_fn`, over `new`'s trampoline.
struct Counting;

impl Tracer for Counting {
    fn trace(_: TraceLevel, message: &CStr) {
        CALLS.fetch_add(1, SeqCst);
        std::hint::black_box(message.to_bytes().len());
    }
}

#[test]
fn from_fn_adopts_a_trampoline() {
    // SAFETY: `new`'s trampolines meet the contract, adding no obligation.
    let cb = unsafe { TraceCallback::from_fn(TraceCallback::new::<Counting>().as_fn()) };
    let before = CALLS.load(SeqCst);
    thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| cb.call(TraceLevel::Debug, c"trampoline"));
        }
    });
    assert!(CALLS.load(SeqCst) - before >= 2);
}

/// An HTTP server on loopback answering every request `404`; its URL.
fn serve_404() -> CString {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&chunk[..n]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    CString::new(format!("http://127.0.0.1:{port}/repo.git")).unwrap()
}

/// Connects a detached remote to `url` for a fetch and frees it; the status.
fn connect(url: &CStr) -> c_int {
    let mut remote = std::ptr::null_mut();
    // SAFETY: a live out-parameter and a C string, which libgit2 copies.
    let rc = unsafe { ffi::git_remote_create_detached(&raw mut remote, url.as_ptr()) };
    assert_eq!(rc, 0);
    // SAFETY: a live remote; NULL callbacks, proxy and headers take the defaults.
    let rc = unsafe {
        ffi::git_remote_connect(
            remote,
            ffi::GIT_DIRECTION_FETCH,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    // SAFETY: the remote created above, freed once.
    unsafe { ffi::git_remote_free(remote) };
    rc
}

/// Installed in libgit2, an adopted function receives its messages on every thread
/// that connects, at once.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_fn_installed_in_libgit2() {
    isolated(
        concat!(module_path!(), "::from_fn_installed_in_libgit2"),
        || {
            // SAFETY: no arguments; the reference dies with the child.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);
            let url = serve_404();
            // Untraced, on this thread first: libgit2's HTTP client fills its parser
            // settings lazily in a static without a lock, so concurrent first
            // connections would race there.
            assert!(connect(&url) < 0);
            // SAFETY: `c_logging` meets the contract (see its doc).
            let cb = unsafe { TraceCallback::from_fn(c_logging) };
            // SAFETY: a named level with a callback that libgit2 may call from any thread
            // for the life of the process; no other thread touches the trace state now.
            let rc = unsafe { ffi::git_trace_set(TraceLevel::Debug.raw(), cb.raw()) };
            assert_eq!(rc, 0);
            let threads = if cfg!(bsan) { 2 } else { 4 };
            thread::scope(|s| {
                for _ in 0..threads {
                    s.spawn(|| assert!(connect(&url) < 0, "the server answers 404"));
                }
            });
            // SAFETY: `None` without a callback; the connecting threads have ended.
            let rc = unsafe { ffi::git_trace_set(TraceLevel::None.raw(), None) };
            assert_eq!(rc, 0);
            let connecting = take_mine(b"Connecting to remote 127.0.0.1");
            assert_eq!(connecting.len(), threads);
            assert!(
                connecting
                    .iter()
                    .all(|(level, _)| *level == ffi::GIT_TRACE_DEBUG)
            );
        },
    );
}

/// Once installed, libgit2 calls the function with no `TraceCallback` left anywhere:
/// `from_fn`'s obligation binds the function for the process, not for the value that
/// adopted it, so a function that honours it keeps receiving messages after that
/// value is gone.
#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn from_fn_installed_function_outlives_every_callback_value() {
    isolated(
        concat!(
            module_path!(),
            "::from_fn_installed_function_outlives_every_callback_value"
        ),
        || {
            // SAFETY: no arguments; the reference dies with the child.
            assert!(unsafe { ffi::git_libgit2_init() } > 0);
            let url = serve_404();
            assert!(connect(&url) < 0);
            let raw = {
                // SAFETY: `c_logging` meets the contract (see its doc): its state is
                // static, so it stays callable for the process.
                let cb = unsafe { TraceCallback::from_fn(c_logging) };
                cb.raw()
            };
            // SAFETY: a named level with a function libgit2 may call from any thread
            // for the life of the process; this thread is alone with the trace state.
            let rc = unsafe { ffi::git_trace_set(TraceLevel::Debug.raw(), raw) };
            assert_eq!(rc, 0);
            // Every `TraceCallback` is gone; libgit2 still calls the function.
            for _ in 0..3 {
                assert!(connect(&url) < 0, "the server answers 404");
            }
            // SAFETY: `None` without a callback; nothing else touches the trace state.
            let rc = unsafe { ffi::git_trace_set(TraceLevel::None.raw(), None) };
            assert_eq!(rc, 0);
            let connecting = take_mine(b"Connecting to remote 127.0.0.1");
            assert_eq!(connecting.len(), 3);
            assert!(
                connecting
                    .iter()
                    .all(|(level, _)| *level == ffi::GIT_TRACE_DEBUG)
            );
        },
    );
}
