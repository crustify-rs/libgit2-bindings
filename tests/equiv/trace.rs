//! Equivalence of `git2/trace.h`'s [`TraceLevel`] with the `git_trace_level_t`
//! values libgit2 declares and accepts.
//!
//! [`TraceLevel`] must name exactly the values of `git_trace_level_t`, each with its C
//! value, and order them as libgit2 does when it decides to report a message
//! (`configured >= level`, over the C values). libgit2 observes a level through
//! `git_trace_set`, which accepts any level with a callback and only
//! [`TraceLevel::None`] without one.
//!
//! Documented divergence, outside these comparisons: a debug build of libgit2 aborts
//! on a level above `None` without a callback (`GIT_ASSERT_ARG`), so that probe is
//! skipped under the `coverage` variant, the one debug build.
//!
//! Then the trace callback, [`TraceCallback`], against a C function receiving the same
//! messages (`callback` below), and [`trace::set`] / [`trace::clear`] against
//! `git_trace_set` (`config` below).
//!
//! [`TraceCallback`]: libgit2::libgit2::core::trace::TraceCallback
//! [`trace::set`]: libgit2::libgit2::core::trace::set
//! [`trace::clear`]: libgit2::libgit2::core::trace::clear

use std::ffi::{CStr, c_char};
use std::sync::Mutex;

use libgit2::ffi;
use libgit2::libgit2::core::trace::{TraceLevel, UnnamedTraceLevel};

/// Every `git_trace_level_t` constant, beside the variant that must carry it.
const LEVELS: [(ffi::git_trace_level_t, TraceLevel); 7] = [
    (ffi::GIT_TRACE_NONE, TraceLevel::None),
    (ffi::GIT_TRACE_FATAL, TraceLevel::Fatal),
    (ffi::GIT_TRACE_ERROR, TraceLevel::Error),
    (ffi::GIT_TRACE_WARN, TraceLevel::Warn),
    (ffi::GIT_TRACE_INFO, TraceLevel::Info),
    (ffi::GIT_TRACE_DEBUG, TraceLevel::Debug),
    (ffi::GIT_TRACE_TRACE, TraceLevel::Trace),
];

/// `git_trace_set` writes libgit2's global trace state without a lock: one writer at a
/// time in this binary.
static TRACE_STATE: Mutex<()> = Mutex::new(());

/// Every value around the declared ones and the extremes.
fn probes() -> impl Iterator<Item = ffi::git_trace_level_t> {
    (0..=64).chain([
        0x100,
        ffi::git_trace_level_t::MAX - 1,
        ffi::git_trace_level_t::MAX,
    ])
}

/// Initialises libgit2, whose error state a rejected call writes; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The message of the calling thread's last error.
fn last_message() -> String {
    // SAFETY: `git_error_last` never returns NULL, and its message is a NUL-terminated
    // string, both live until the next libgit2 call on this thread, after the copy.
    let error_last = unsafe { ffi::git_error_last() };
    // SAFETY: as above.
    let message2 = unsafe { (*error_last).message };
    // SAFETY: as above.
    unsafe { CStr::from_ptr(message2) }
        .to_string_lossy()
        .into_owned()
}

unsafe extern "C" fn ignore(_: ffi::git_trace_level_t, _: *const c_char) {}

/// `git_trace_set(raw, cb)`.
fn trace_set(raw: ffi::git_trace_level_t, cb: ffi::git_trace_cb) -> i32 {
    // SAFETY: `cb` is NULL or a function that ignores its arguments, valid for the
    // process; nothing in this binary emits a trace message.
    unsafe { ffi::git_trace_set(raw, cb) }
}

#[test]
fn levels_carry_their_c_values() {
    for (c, level) in LEVELS {
        assert_eq!(level.raw(), c, "{level:?}");
        assert_eq!(ffi::git_trace_level_t::from(level), c, "{level:?}");
    }
}

#[test]
fn from_raw_names_exactly_the_c_levels() {
    for raw in probes() {
        let c = LEVELS.iter().find(|(c, _)| *c == raw).map(|&(_, l)| l);
        assert_eq!(TraceLevel::from_raw(raw), c, "{raw}");
        assert_eq!(
            TraceLevel::try_from(raw).map_err(UnnamedTraceLevel::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
fn order_is_libgit2s_report_rule() {
    for (configured_raw, configured) in LEVELS {
        for (message_raw, message) in LEVELS {
            // `git_trace`: `git_trace__data.level >= level`.
            assert_eq!(
                configured >= message,
                configured_raw >= message_raw,
                "{configured:?} {message:?}"
            );
            assert_eq!(configured.cmp(&message), configured_raw.cmp(&message_raw));
        }
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_trace_set_accepts_every_level_with_a_callback() {
    init();
    let _state = TRACE_STATE.lock().unwrap();
    for raw in probes() {
        assert_eq!(trace_set(raw, Some(ignore)), 0, "{raw}");
    }
    assert_eq!(trace_set(TraceLevel::None.raw(), None), 0);
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_trace_set_needs_a_callback_above_none() {
    if std::env::var("CRUSTIFY_BUILD_VARIANT").as_deref() == Ok("coverage") {
        // `GIT_ASSERT_ARG` aborts in the debug build.
        return;
    }
    init();
    let _state = TRACE_STATE.lock().unwrap();
    for raw in probes() {
        let rc = trace_set(raw, None);
        if TraceLevel::from_raw(raw) == Some(TraceLevel::None) {
            assert_eq!(rc, 0, "{raw}");
        } else {
            assert_eq!(rc, -1, "{raw}");
            // The condition as the preprocessor expanded `NULL`.
            let message = last_message();
            assert!(
                message.starts_with("invalid argument: 'level == 0 || callback != "),
                "{raw}: {message}"
            );
        }
    }
    assert_eq!(trace_set(TraceLevel::None.raw(), None), 0);
}

// --- Callback ------------------------------------------------------------------------
//
// libgit2 traces only from its HTTP client, so each sequence connects to a local HTTP
// server, once with a C recorder installed and once with a `TraceCallback`, at the same
// level, and compares the connection's outcome and the messages received after each
// pair of steps. The trace state is process-global and every other test here installs
// callbacks of its own, so the sequences run alone in a child process.

mod callback {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::sync::Mutex;
    use std::{ptr, thread};

    use libgit2::ffi;
    use libgit2::libgit2::core::trace::{TraceCallback, TraceLevel, Tracer};

    const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

    /// Runs `body` alone in a child process, so no other test shares the trace state.
    pub(super) fn isolated(test: &str, body: fn()) {
        let name = test.strip_prefix("equiv::").unwrap_or(test);
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

    /// One message as a callback received it: the raw level and the message's bytes.
    type Message = (ffi::git_trace_level_t, Vec<u8>);

    static C_LOG: Mutex<Vec<Message>> = Mutex::new(Vec::new());
    static RUST_LOG: Mutex<Vec<Message>> = Mutex::new(Vec::new());

    fn take(log: &Mutex<Vec<Message>>) -> Vec<Message> {
        std::mem::take(&mut *log.lock().unwrap())
    }

    /// The reference: a C callback recording what libgit2 passes it.
    unsafe extern "C" fn c_record(level: ffi::git_trace_level_t, msg: *const c_char) {
        // SAFETY: libgit2 passes a C string live for the call.
        let bytes = unsafe { CStr::from_ptr(msg) }.to_bytes().to_vec();
        C_LOG.lock().unwrap().push((level, bytes));
    }

    /// The tracer under test, recording what it receives.
    struct RustRecord;

    impl Tracer for RustRecord {
        fn trace(level: TraceLevel, message: &CStr) {
            RUST_LOG
                .lock()
                .unwrap()
                .push((level.raw(), message.to_bytes().to_vec()));
        }
    }

    pub(super) fn init() {
        // SAFETY: no arguments; the reference dies with the child.
        assert!(unsafe { ffi::git_libgit2_init() } > 0);
    }

    fn trace_set(level: TraceLevel, cb: ffi::git_trace_cb) {
        // SAFETY: the child runs one thread while it changes the trace state; `cb` is
        // NULL with `None` or a function valid for the process.
        assert_eq!(unsafe { ffi::git_trace_set(level.raw(), cb) }, 0);
    }

    /// An HTTP server on loopback answering every request `404`, for the life of the
    /// child; its URL.
    pub(super) fn serve() -> CString {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
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
                drop(stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ));
            }
        });
        CString::new(format!("http://127.0.0.1:{port}/repo.git")).unwrap()
    }

    /// The outcome of connecting to `url` for a fetch: the status and, on failure, the
    /// error's class and message.
    pub(super) fn connect(url: &CStr) -> (c_int, Option<(c_int, CString)>) {
        let mut remote = ptr::null_mut();
        // SAFETY: a live out-parameter and a C string, copied by libgit2.
        let rc = unsafe { ffi::git_remote_create_detached(&raw mut remote, url.as_ptr()) };
        assert_eq!(rc, 0);
        // SAFETY: a live remote; NULL callbacks, proxy and headers take the defaults.
        let rc = unsafe {
            ffi::git_remote_connect(
                remote,
                ffi::GIT_DIRECTION_FETCH,
                ptr::null(),
                ptr::null(),
                ptr::null(),
            )
        };
        let error = (rc < 0).then(|| {
            {
                // SAFETY: never NULL; copied out before the next libgit2 call.
                let e = unsafe { ffi::git_error_last() };
                // SAFETY: as above.
                let klass = unsafe { (*e).klass };
                // SAFETY: as above.
                let message2 = unsafe { (*e).message };
                // SAFETY: as above.
                unsafe { (klass, CStr::from_ptr(message2).to_owned()) }
            }
        });
        // SAFETY: the remote created above, freed once.
        unsafe { ffi::git_remote_free(remote) };
        (rc, error)
    }

    const LEVELS: [TraceLevel; 7] = [
        TraceLevel::Trace,
        TraceLevel::Debug,
        TraceLevel::Info,
        TraceLevel::Warn,
        TraceLevel::Error,
        TraceLevel::Fatal,
        TraceLevel::None,
    ];

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn libgit2s_messages_reach_a_tracer_as_they_reach_c() {
        isolated(
            "trace::callback::libgit2s_messages_reach_a_tracer_as_they_reach_c",
            || {
                init();
                let url = serve();
                let port = url.to_str().unwrap().rsplit(':').next().unwrap();
                let port = port.split('/').next().unwrap();
                let rust = TraceCallback::new::<RustRecord>();
                // Each level twice, so a step also follows one of its own kind.
                for level in LEVELS.into_iter().chain(LEVELS) {
                    trace_set(level, Some(c_record));
                    let c = connect(&url);
                    let c_log = take(&C_LOG);

                    trace_set(level, rust.raw());
                    let r = connect(&url);
                    let rust_log = take(&RUST_LOG);

                    assert_eq!(r, c, "{level:?}");
                    assert_eq!(rust_log, c_log, "{level:?}");
                    assert_eq!(take(&C_LOG), [], "{level:?}: C received a Rust step");
                    assert!(c.0 < 0, "the server answers 404");
                    // libgit2 reports exactly the messages at or below the level.
                    assert!(c_log.iter().all(|(l, _)| *l <= level.raw()), "{level:?}");
                    let connecting = format!("Connecting to remote 127.0.0.1 port {port}");
                    let has_connecting = c_log
                        .iter()
                        .any(|(l, m)| *l == ffi::GIT_TRACE_DEBUG && m == connecting.as_bytes());
                    assert_eq!(has_connecting, level >= TraceLevel::Debug, "{level:?}");
                    let has_received = c_log
                        .iter()
                        .any(|(l, m)| *l == ffi::GIT_TRACE_TRACE && m.starts_with(b"Received:\n"));
                    assert_eq!(has_received, level >= TraceLevel::Trace, "{level:?}");
                }
                // A C function adopted through `from_fn` receives what it would directly.
                trace_set(TraceLevel::Trace, Some(c_record));
                let c = connect(&url);
                let c_log = take(&C_LOG);
                // SAFETY: `c_record` reads `msg` only during the call, from any thread
                // (its log is a `Mutex`), and never unwinds for a live C string.
                let adopted = unsafe { TraceCallback::from_fn(c_record) };
                trace_set(TraceLevel::Trace, adopted.raw());
                let r = connect(&url);
                assert_eq!(r, c);
                assert_eq!(take(&C_LOG), c_log);
                trace_set(TraceLevel::None, None);
            },
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn call_passes_what_a_direct_c_call_passes() {
        isolated(
            "trace::callback::call_passes_what_a_direct_c_call_passes",
            || {
                let messages: [&CStr; 4] = [c"", c"one", c"\xff\xfe bytes", c"Received:\nline\r\n"];
                let rust = TraceCallback::new::<RustRecord>();
                // SAFETY: as in the test above.
                let adopted = unsafe { TraceCallback::from_fn(c_record) };
                for level in LEVELS {
                    for message in messages {
                        // SAFETY: a named level and a C string live for the call.
                        unsafe { c_record(level.raw(), message.as_ptr()) };
                        let direct = take(&C_LOG);
                        rust.call(level, message);
                        assert_eq!(take(&RUST_LOG), direct, "{level:?} {message:?}");
                        adopted.call(level, message);
                        assert_eq!(take(&C_LOG), direct, "{level:?} {message:?}");
                    }
                }
            },
        );
    }
}

// --- Configuration -------------------------------------------------------------------
//
// `trace::set` / `trace::clear` against `git_trace_set` over the same sequences of
// configurations. libgit2 has one trace state, observable only through the messages it
// reports and the thread's error state, so each step installs a configuration through
// C, connects to a local HTTP server and records what reached the callback, then
// installs the same configuration through Rust and does the same, and compares the
// two after every step. The Rust side runs twice: over the C recorder adopted through
// `from_fn`, so both sides call the very same function, and over a `Tracer`. The whole
// sequence runs alone in a child process.

mod config {
    use std::ffi::{CStr, CString, c_char, c_int};
    use std::sync::Mutex;

    use libgit2::ffi;
    use libgit2::libgit2::core::trace::{self, TraceCallback, TraceLevel, Tracer};

    use super::callback::{connect, init, isolated, serve};

    type Message = (ffi::git_trace_level_t, Vec<u8>);

    static C_LOG: Mutex<Vec<Message>> = Mutex::new(Vec::new());
    static RUST_LOG: Mutex<Vec<Message>> = Mutex::new(Vec::new());

    fn take(log: &Mutex<Vec<Message>>) -> Vec<Message> {
        std::mem::take(&mut *log.lock().unwrap())
    }

    /// The reference callback, installed by both sides.
    unsafe extern "C" fn c_record(level: ffi::git_trace_level_t, msg: *const c_char) {
        // SAFETY: libgit2 passes a C string live for the call.
        let bytes = unsafe { CStr::from_ptr(msg) }.to_bytes().to_vec();
        C_LOG.lock().unwrap().push((level, bytes));
    }

    struct RustRecord;

    impl Tracer for RustRecord {
        fn trace(level: TraceLevel, message: &CStr) {
            RUST_LOG
                .lock()
                .unwrap()
                .push((level.raw(), message.to_bytes().to_vec()));
        }
    }

    /// One configuration: a level with the callback, or no callback (`Clear`).
    #[derive(Clone, Copy, Debug)]
    enum Step {
        Set(TraceLevel),
        Clear,
    }

    /// Marks the thread's error state, so a step that touches it shows.
    fn mark_error() {
        let class = ffi::GIT_ERROR_CALLBACK.cast_signed();
        // SAFETY: a class and a C string, which libgit2 copies; libgit2 is initialised.
        let rc = unsafe { ffi::git_error_set_str(class, c"mark".as_ptr()) };
        assert_eq!(rc, 0);
    }

    /// The thread's last error: its class and message.
    fn last_error() -> (c_int, CString) {
        {
            // SAFETY: never NULL; copied out before the next libgit2 call.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: as above.
            let klass = unsafe { (*e).klass };
            // SAFETY: as above.
            let message2 = unsafe { (*e).message };
            // SAFETY: as above.
            unsafe { (klass, CStr::from_ptr(message2).to_owned()) }
        }
    }

    /// The C step: its status, the error state after it, and what one connection then
    /// reported to `c_record`.
    fn c_step(step: Step, url: &CStr) -> (c_int, (c_int, CString), Vec<Message>) {
        mark_error();
        let rc = match step {
            // SAFETY: the child runs one thread while it changes the trace state;
            // `c_record` is valid for the process and callable from any thread.
            Step::Set(level) => unsafe { ffi::git_trace_set(level.raw(), Some(c_record)) },
            // SAFETY: as above, with no callback at `GIT_TRACE_NONE`.
            Step::Clear => unsafe { ffi::git_trace_set(ffi::GIT_TRACE_NONE, None) },
        };
        let error = last_error();
        assert!(connect(url).0 < 0, "the server answers 404");
        (rc, error, take(&C_LOG))
    }

    /// The Rust step over `callback`: the status C's call has when it succeeds (Rust's
    /// returns nothing), the error state after it, and what one connection then
    /// reported to `log`.
    fn rust_step(
        step: Step,
        callback: TraceCallback,
        log: &Mutex<Vec<Message>>,
        url: &CStr,
    ) -> (c_int, (c_int, CString), Vec<Message>) {
        mark_error();
        match step {
            // SAFETY: the child runs one thread while it changes the trace state.
            Step::Set(level) => unsafe { trace::set(level, callback) },
            // SAFETY: as above.
            Step::Clear => unsafe { trace::clear() },
        }
        let error = last_error();
        assert!(connect(url).0 < 0, "the server answers 404");
        (0, error, take(log))
    }

    const SEQUENCE: [Step; 14] = [
        Step::Clear,
        Step::Set(TraceLevel::Debug),
        Step::Set(TraceLevel::Trace),
        Step::Set(TraceLevel::Trace),
        Step::Set(TraceLevel::None),
        Step::Set(TraceLevel::Info),
        Step::Set(TraceLevel::Fatal),
        Step::Clear,
        Step::Clear,
        Step::Set(TraceLevel::Warn),
        Step::Set(TraceLevel::Debug),
        Step::Set(TraceLevel::Error),
        Step::Set(TraceLevel::Trace),
        Step::Clear,
    ];

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn set_and_clear_configure_libgit2_as_git_trace_set_does() {
        isolated(
            "trace::config::set_and_clear_configure_libgit2_as_git_trace_set_does",
            || {
                init();
                let url = serve();
                // SAFETY: `c_record` reads `msg` only during the call, from any thread
                // (its log is a `Mutex`), and never unwinds for a live C string.
                let adopted = unsafe { TraceCallback::from_fn(c_record) };
                let tracer = TraceCallback::new::<RustRecord>();
                for (i, step) in SEQUENCE.into_iter().enumerate() {
                    let c = c_step(step, &url);
                    let r = rust_step(step, adopted, &C_LOG, &url);
                    assert_eq!(r, c, "step {i}: {step:?}, adopted");
                    let c = c_step(step, &url);
                    let r = rust_step(step, tracer, &RUST_LOG, &url);
                    assert_eq!(r, c, "step {i}: {step:?}, tracer");
                    assert_eq!(take(&C_LOG), [], "step {i}: C received a Rust step");

                    assert_eq!(c.0, 0);
                    assert_eq!(
                        c.1,
                        (ffi::GIT_ERROR_CALLBACK.cast_signed(), c"mark".to_owned())
                    );
                    let level = match step {
                        Step::Set(level) => level,
                        Step::Clear => TraceLevel::None,
                    };
                    assert!(c.2.iter().all(|(l, _)| *l <= level.raw()), "{step:?}");
                    // The HTTP client traces at `Debug` and `Trace`.
                    assert_eq!(c.2.is_empty(), level < TraceLevel::Debug, "{step:?}");
                }
            },
        );
    }

    /// The same sequence with each step's configuration changed from inside the
    /// callback, on the thread libgit2 traces from: the first message installs the next
    /// step's configuration, so the rest of that connection runs under it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn set_and_clear_from_the_callback_as_git_trace_set_does() {
        isolated(
            "trace::config::set_and_clear_from_the_callback_as_git_trace_set_does",
            || {
                init();
                let url = serve();
                for side in [Side::C, Side::Rust] {
                    *SIDE.lock().unwrap() = Some(side);
                    // SAFETY: as in `c_step`; one thread.
                    let rc = unsafe { ffi::git_trace_set(ffi::GIT_TRACE_TRACE, Some(switching)) };
                    assert_eq!(rc, 0);
                    assert!(connect(&url).0 < 0);
                    // A second connection runs under whatever the first one left.
                    assert!(connect(&url).0 < 0);
                    let log = take(&C_LOG);
                    LOGS.lock().unwrap().push(log);
                    // SAFETY: as above.
                    unsafe { ffi::git_trace_set(ffi::GIT_TRACE_NONE, None) };
                }
                let logs = std::mem::take(&mut *LOGS.lock().unwrap());
                assert_eq!(logs.len(), 2);
                assert_eq!(logs[1], logs[0]);
                // The switch on the first message took effect for the rest: from
                // `Trace` to `Debug`, so the connection's `Received` reached no one.
                assert!(!logs[0].is_empty(), "{:?}", logs[0]);
                assert!(
                    logs[0]
                        .iter()
                        .all(|(l, _)| *l == ffi::GIT_TRACE_DEBUG || *l == ffi::GIT_TRACE_TRACE)
                );
                assert!(!logs[0].iter().any(|(_, m)| m.starts_with(b"Received:\n")));
            },
        );
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Side {
        C,
        Rust,
    }

    static SIDE: Mutex<Option<Side>> = Mutex::new(None);
    static LOGS: Mutex<Vec<Vec<Message>>> = Mutex::new(Vec::new());

    /// Records the message, then lowers the level to `Debug` with `c_record` as the
    /// callback, through the side under test.
    unsafe extern "C" fn switching(level: ffi::git_trace_level_t, msg: *const c_char) {
        // SAFETY: libgit2 passes a C string live for the call.
        unsafe { c_record(level, msg) };
        let side = SIDE.lock().unwrap().expect("a side");
        match side {
            // SAFETY: on the thread libgit2 traces from, the only one in libgit2.
            Side::C => unsafe {
                ffi::git_trace_set(ffi::GIT_TRACE_DEBUG, Some(c_record));
            },
            Side::Rust => {
                // SAFETY: `c_record` meets `from_fn`'s contract (see the test above).
                let cb = unsafe { TraceCallback::from_fn(c_record) };
                // SAFETY: on the thread libgit2 traces from, the only one in libgit2:
                // a traced call on the same thread may change the configuration.
                unsafe { trace::set(TraceLevel::Debug, cb) };
            }
        }
    }

    /// Reached before `git_libgit2_init`, as `git_trace_set` may be: the configuration
    /// holds once libgit2 starts, as C's installed after it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn set_before_init_holds_after_it() {
        isolated("trace::config::set_before_init_holds_after_it", || {
            // SAFETY: `c_record` meets `from_fn`'s contract (see above).
            let cb = unsafe { TraceCallback::from_fn(c_record) };
            // SAFETY: libgit2 is not initialised yet and this thread is alone.
            unsafe { trace::set(TraceLevel::Debug, cb) };
            init();
            let url = serve();
            assert!(connect(&url).0 < 0);
            let r = take(&C_LOG);
            let c = c_step(Step::Set(TraceLevel::Debug), &url);
            assert_eq!(r, c.2);
            assert!(!r.is_empty(), "{r:?}");
            // SAFETY: one thread.
            unsafe { trace::clear() };
        });
    }
}
