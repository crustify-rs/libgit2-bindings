//! The suite's fixtures. `unsafe` is confined here, standing in for safe constructors
//! the crate does not have yet; replace each once its safe counterpart lands.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_int, c_void};
use std::mem::MaybeUninit;
use std::process::Command;

use libgit2::libgit2::raw::OdbDataVec;

const CHILD: &str = "CRUSTIFY_UB_SAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2.
pub fn isolated(test: &str, body: fn()) {
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

/// Promotes a run every element of which the test wrote; stands in for the typed
/// constructors that fill a run in one safe call.
pub fn filled<T>(v: OdbDataVec<MaybeUninit<T>>) -> OdbDataVec<T> {
    // SAFETY: the caller wrote every element, which is this fixture's contract.
    unsafe { v.assume_init() }
}

/// A plain C struct to place in an `OdbDataBox`; the crate wraps no C type yet.
///
/// Replace with a wrapped libgit2 type once one is resource-free and `CZeroable`.
#[repr(C)]
pub struct pair_st {
    pub a: u64,
    pub b: u32,
}
ffibox::define_ctype!(Pair, PairRef, PairMut, pair_st);

// SAFETY: any `a` and `b` are valid, zero included; nothing tears a pair down.
unsafe impl ffibox::CZeroable for Pair {}
// SAFETY: a pair is plain data with no thread affinity; its handles keep Rust's
// borrow rules, so moving or sharing one across threads races nothing.
unsafe impl Send for Pair {}
// SAFETY: as above.
unsafe impl Sync for Pair {}

impl PairRef<'_> {
    pub fn a(&self) -> u64 {
        // SAFETY: a read through the shared handle's pointer to a live `pair_st`.
        unsafe { core::ptr::addr_of!((*self.as_ptr()).a).read() }
    }
    pub fn b(&self) -> u32 {
        // SAFETY: as above.
        unsafe { core::ptr::addr_of!((*self.as_ptr()).b).read() }
    }
}

impl PairMut<'_> {
    pub fn set(&mut self, a: u64, b: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to a live `pair_st`.
        unsafe {
            core::ptr::addr_of_mut!((*p).a).write(a);
            core::ptr::addr_of_mut!((*p).b).write(b);
        }
    }
}

// --- `git2/sys/alloc.h` ------------------------------------------------------------

use core::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use libgit2::ffi;
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

static MALLOCS: AtomicUsize = AtomicUsize::new(0);
static REALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

/// libc's allocator behind `Alloc`, counting the calls that reach it. Storage is
/// libc's either way, so it may be installed while libc-allocated blocks are live.
///
/// It refuses [`REFUSED`] bytes: a request libc cannot meet would abort under ASan
/// (`allocator_may_return_null=0`) instead of failing.
pub struct Counted;

/// The one request size [`Counted`] refuses.
pub const REFUSED: usize = 1 << 40;

impl Counted {
    /// `(malloc, realloc, free)` calls so far, process-wide.
    pub fn calls() -> (usize, usize, usize) {
        (
            MALLOCS.load(SeqCst),
            REALLOCS.load(SeqCst),
            FREES.load(SeqCst),
        )
    }
}

// SAFETY: forwards to libc's `malloc` family unchanged, which meets every clause of
// `Alloc`'s contract; the counters are atomics.
unsafe impl Alloc for Counted {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        MALLOCS.fetch_add(1, SeqCst);
        if len == REFUSED {
            return None;
        }
        // SAFETY: plain libc allocation.
        NonNull::new(unsafe { malloc(len) }.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        REALLOCS.fetch_add(1, SeqCst);
        if len == REFUSED {
            return None;
        }
        let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        FREES.fetch_add(1, SeqCst);
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

/// A `git_allocator.gfree` that releases nothing, to store in an `Allocator`.
pub unsafe extern "C" fn leave_free(_: *mut c_void) {}

/// Installs `allocator` as libgit2's allocator through the unsafe
/// [`common::set_allocator`](libgit2::libgit2::core::common::set_allocator), whose
/// contract the tests meet as below.
///
/// Callers install a libc-backed family ([`Counted`]) or install before libgit2 has
/// allocated, so no live block changes family, and run single-threaded around it.
pub fn install(allocator: &Allocator) {
    // SAFETY: `allocator` is a coherent family (an `Allocator::new` value over an
    // `Alloc` implementation); live blocks stay releasable and no other thread calls
    // libgit2 meanwhile, per this function's contract.
    unsafe { libgit2::libgit2::core::common::set_allocator(Some(allocator.as_ref())) }
}

// --- `git2/types.h`: `git_writestream` ---------------------------------------------

use std::ffi::CString;
use std::path::PathBuf;
use std::sync::OnceLock;

use libgit2::libgit2::core::types::{RustWriteStream, StreamError, StreamWriter, WriteStreamBox};
use libgit2::libgit2::util::errors::{ErrorCode, last};

/// Takes a `git_libgit2_init` reference that is never released.
pub fn init() {
    core::mem::forget(libgit2::libgit2::core::global::init().unwrap());
}

/// A fresh bare repository in a temporary directory, freed and removed on drop; stands
/// in for the `git_repository` wrapper.
pub struct TempRepo {
    pub path: PathBuf,
    repo: *mut ffi::git_repository,
}

impl TempRepo {
    pub fn new(tag: &str) -> Self {
        init();
        let path = std::env::temp_dir().join(format!(
            "crustify-writestream-ub-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let c_path = CString::new(path.to_str().unwrap()).unwrap();
        let mut opts = ffi::git_repository_init_options::default();
        // SAFETY: `opts` is a live options struct, filled with the version's defaults.
        let rc = unsafe {
            ffi::git_repository_init_options_init(
                &mut opts,
                ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
            )
        };
        assert_eq!(rc, 0);
        opts.flags = ffi::GIT_REPOSITORY_INIT_BARE | ffi::GIT_REPOSITORY_INIT_MKDIR;
        let mut repo = core::ptr::null_mut();
        // SAFETY: a live slot, a NUL-terminated path and live options, read for the call.
        let rc = unsafe { ffi::git_repository_init_ext(&mut repo, c_path.as_ptr(), &mut opts) };
        assert_eq!(rc, 0);
        Self { path, repo }
    }

    /// Files under `objects/` whose name starts with `streamed`: a blob stream's
    /// temporary file while it is open.
    pub fn stream_files(&self) -> usize {
        std::fs::read_dir(self.path.join("objects"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("streamed")
            })
            .count()
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        // SAFETY: the repository this fixture opened, freed once; every stream the
        // tests opened on it is gone by now (they borrow the fixture).
        unsafe { ffi::git_repository_free(self.repo) };
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `git_blob_create_from_stream` adopted into a `WriteStreamBox`: the one fixture
/// adopting a C stream, standing in for the blob wrapper, which replaces it.
///
/// The repository must outlive the stream, which the borrow does not enforce.
pub fn blob_stream(repo: &TempRepo, hint: Option<&CStr>) -> WriteStreamBox {
    let mut out = core::ptr::null_mut();
    // SAFETY: a live slot, the live repository and a NULL or live hint path, which
    // libgit2 copies.
    let rc = unsafe {
        ffi::git_blob_create_from_stream(
            &mut out,
            repo.repo,
            hint.map_or(core::ptr::null(), CStr::as_ptr),
        )
    };
    assert_eq!(rc, 0);
    // SAFETY: a fresh, open blob stream, owned by the caller and released by its own
    // `free` member.
    unsafe { WriteStreamBox::from_c(out) }.unwrap()
}

/// `git_blob_create_from_stream_commit`, which consumes the stream: the new blob's id.
pub fn commit_blob(stream: WriteStreamBox) -> Result<Vec<u8>, c_int> {
    let mut oid = ffi::git_oid::default();
    // SAFETY: an open blob stream (only ever written, never closed), whose ownership
    // passes to the call, and a live id slot.
    let rc = unsafe { ffi::git_blob_create_from_stream_commit(&mut oid, stream.into_c()) };
    if rc < 0 {
        return Err(rc);
    }
    Ok(oid.id[..20].to_vec())
}

/// A filter list: none, libgit2's own `ident` filter cleaning, [`upper_filter`], or
/// [`refused_filter`].
#[derive(Clone, Copy)]
pub enum Filters {
    None,
    IdentClean,
    Upper,
    Refused,
}

/// A fresh list holding the filter, or NULL for none.
fn filter_list(filters: Filters) -> *mut ffi::git_filter_list {
    let filter = match filters {
        Filters::None => return core::ptr::null_mut(),
        // SAFETY: a NUL-terminated name; the registry outlives the call.
        Filters::IdentClean => unsafe { ffi::git_filter_lookup(c"ident".as_ptr()) },
        Filters::Upper => upper_filter(),
        Filters::Refused => refused_filter(),
    };
    assert!(!filter.is_null());
    let mut list = core::ptr::null_mut();
    // SAFETY: a live slot; no repository is needed by these filters.
    let rc = unsafe {
        ffi::git_filter_list_new(&mut list, core::ptr::null_mut(), ffi::GIT_FILTER_TO_ODB, 0)
    };
    assert_eq!(rc, 0);
    // SAFETY: a fresh list and a registered filter, with no payload.
    let rc = unsafe { ffi::git_filter_list_push(list, filter, core::ptr::null_mut()) };
    assert_eq!(rc, 0);
    list
}

/// `git_filter_list_stream_buffer(filters, data, target)`, standing in for the filter
/// wrapper: libgit2 writes into the target and closes it, and never frees it.
///
/// libgit2 initialises a filter on its first push without a lock (`filter.c` reads and
/// writes `fdef->initialized` outside the registry lock), so concurrent first pushes
/// race (TSan); every filter is pushed once first, serially.
pub fn stream_buffer(filters: Filters, data: &[u8], target: &mut WriteStreamBox) -> c_int {
    static PRIMED: OnceLock<()> = OnceLock::new();
    init();
    PRIMED.get_or_init(|| {
        for filters in [Filters::IdentClean, Filters::Upper, Filters::Refused] {
            // SAFETY: the fresh list, freed once.
            unsafe { ffi::git_filter_list_free(filter_list(filters)) };
        }
    });
    let list = filter_list(filters);
    // SAFETY: `list` is NULL or the live list above; `data` is live for the call; the
    // target is an open stream the caller owns, which the call writes and closes.
    let rc = unsafe {
        ffi::git_filter_list_stream_buffer(
            list,
            data.as_ptr().cast(),
            data.len(),
            target.as_c_ptr(),
        )
    };
    // SAFETY: the list above, or NULL, which libgit2 accepts.
    unsafe { ffi::git_filter_list_free(list) };
    rc
}

/// A stream writing upper-cased data into the next stream of a filter chain, and
/// closing it when closed: what [`upper_filter`]'s `stream` callback returns.
pub struct Upper {
    next: *mut ffi::git_writestream,
}

thread_local! {
    /// `Upper` streams made and dropped on this thread: the callback makes them,
    /// libgit2 frees them on the thread that runs the chain.
    static UPPER: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// `(made, dropped)` `Upper` streams on this thread so far.
pub fn upper_streams() -> (usize, usize) {
    UPPER.get()
}

fn checked(rc: c_int) -> Result<(), StreamError> {
    if rc < 0 {
        let code = ErrorCode::from_raw(rc).unwrap_or(ErrorCode::Generic);
        return Err(StreamError::from(last()).with_code(code));
    }
    Ok(())
}

impl StreamWriter for Upper {
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        let upper = data.to_ascii_uppercase();
        // SAFETY: `next` is the chain's next stream, which libgit2 keeps open and alive
        // until it frees this one; it reads the bytes for the call only.
        let rc =
            unsafe { (*self.next).write.unwrap()(self.next, upper.as_ptr().cast(), upper.len()) };
        checked(rc)
    }

    fn close(&mut self) -> Result<(), StreamError> {
        // SAFETY: as above; this stream closes its next once, as libgit2's do.
        checked(unsafe { (*self.next).close.unwrap()(self.next) })
    }
}

impl Drop for Upper {
    fn drop(&mut self) {
        UPPER.set((UPPER.get().0, UPPER.get().1 + 1));
    }
}

/// `git_filter.stream` for [`upper_filter`]: hands libgit2 an [`Upper`] stream.
unsafe extern "C" fn upper_stream(
    out: *mut *mut ffi::git_writestream,
    _filter: *mut ffi::git_filter,
    _payload: *mut *mut c_void,
    _source: *const ffi::git_filter_source,
    next: *mut ffi::git_writestream,
) -> c_int {
    UPPER.set((UPPER.get().0 + 1, UPPER.get().1));
    let stream = RustWriteStream::new(Upper { next }).into_box();
    // SAFETY: libgit2's live out-slot; it takes the stream over and frees it through
    // its `free` member.
    unsafe { out.write(stream.into_c()) };
    0
}

/// A filter registered under `name`, whose `stream` callback is `stream`; it is only
/// ever pushed onto a list explicitly.
struct Registered(*mut ffi::git_filter);

// SAFETY: the filter is leaked, immutable after registration, and libgit2 reads it
// under its registry lock.
unsafe impl Send for Registered {}
// SAFETY: as above.
unsafe impl Sync for Registered {}

impl Registered {
    fn new(name: &CStr, stream: ffi::git_filter_stream_fn) -> Self {
        init();
        let filter = Box::leak(Box::new(ffi::git_filter::default()));
        // SAFETY: a live filter struct, filled with the version's defaults.
        let rc = unsafe { ffi::git_filter_init(filter, ffi::GIT_FILTER_VERSION) };
        assert_eq!(rc, 0);
        filter.stream = stream;
        // Loaded only for paths that set `crustify-test`, which none do: a filter without
        // a required attribute applies to every blob a hint path selects filters for.
        filter.attributes = c"+crustify-test".as_ptr();
        // SAFETY: a NUL-terminated name and a filter that lives for the process.
        let rc = unsafe { ffi::git_filter_register(name.as_ptr(), filter, 0) };
        assert_eq!(rc, 0);
        Self(filter)
    }
}

/// A filter, registered once as `crustify-upper`, whose streams are Rust streams that
/// libgit2 owns and frees; stands in for the filter wrapper.
pub fn upper_filter() -> *mut ffi::git_filter {
    static FILTER: OnceLock<Registered> = OnceLock::new();
    FILTER
        .get_or_init(|| Registered::new(c"crustify-upper", Some(upper_stream)))
        .0
}

/// `git_filter.stream` for [`refused_filter`]: fails, as a filter that cannot start.
unsafe extern "C" fn refused_stream(
    _out: *mut *mut ffi::git_writestream,
    _filter: *mut ffi::git_filter,
    _payload: *mut *mut c_void,
    _source: *const ffi::git_filter_source,
    _next: *mut ffi::git_writestream,
) -> c_int {
    // SAFETY: a live message, which libgit2 copies; the runtime is initialised.
    unsafe { ffi::git_error_set_str(ffi::GIT_ERROR_FILTER as c_int, c"refused".as_ptr()) };
    ffi::GIT_EUSER
}

/// A filter, registered once as `crustify-refused`, whose `stream` callback fails:
/// libgit2 then closes the chain's last stream, the target, without writing it.
pub fn refused_filter() -> *mut ffi::git_filter {
    static FILTER: OnceLock<Registered> = OnceLock::new();
    FILTER
        .get_or_init(|| Registered::new(c"crustify-refused", Some(refused_stream)))
        .0
}

// --- `git2/strarray.h` -------------------------------------------------------------

/// Sets libgit2's user extensions, which `!name` entries disable, through the unsafe
/// [`common::set_extensions`](libgit2::libgit2::core::common::set_extensions).
/// Process-global: call it in an isolated child, single-threaded.
pub fn set_extensions(names: &[&CStr]) {
    // SAFETY: callers run alone in an isolated child, so no other thread calls into
    // libgit2 while the setting changes.
    unsafe { libgit2::libgit2::core::common::set_extensions(names) }.unwrap();
}

// --- `git2/buffer.h` ---------------------------------------------------------------

use libgit2::libgit2::core::buffer::{Buf, BufVal};

/// Sets a string option of `git_libgit2_opts` (`GIT_OPT_SET_USER_AGENT`,
/// `GIT_OPT_SET_TEMPLATE_PATH`); NULL restores the default. Stands in for the
/// `git_libgit2_opts` wrapper (`core`), which replaces it.
///
/// The options need an initialised libgit2: this takes a reference of its own, which
/// the process keeps.
pub fn set_str_option(option: u32, value: Option<&CStr>) {
    // SAFETY: no arguments; libgit2 counts initialisations under its own lock.
    assert!(unsafe { ffi::git_libgit2_init() } > 0);
    let value = value.map_or(core::ptr::null(), CStr::as_ptr);
    // SAFETY: the string options copy their `const char *`, or restore their default
    // for NULL.
    assert_eq!(unsafe { ffi::git_libgit2_opts(option as c_int, value) }, 0);
}

/// Has libgit2 fill `buf` through a `GIT_OPT_GET_*` option taking a `git_buf *`
/// (`GIT_OPT_GET_USER_AGENT`, `GIT_OPT_GET_TEMPLATE_PATH`), reusing what it owns, and
/// adopts the result. Stands in for the `git_libgit2_opts` wrapper (`core`) and the
/// other routines returning a `git_buf`, which replace it.
pub fn fill_from_option(option: u32, buf: &mut BufVal) {
    let mut raw = buf.as_mut().take().into_inner().into_raw();
    // SAFETY: the `GET` options taking a `git_buf *` accept one that is zeroed or owns
    // a libgit2 allocation, which they reuse or replace.
    let rc = unsafe { ffi::git_libgit2_opts(option as c_int, &raw mut raw) };
    assert_eq!(rc, 0);
    // SAFETY: `raw` is as libgit2 left it: owning a NUL-terminated allocation of its
    // allocator, or nothing.
    buf.as_mut().set(unsafe { Buf::from_raw(raw) });
}

// --- `git2/trace.h` ----------------------------------------------------------------

use libgit2::libgit2::core::trace::{self, TraceCallback, TraceLevel};

/// Installs `callback` as libgit2's tracer at `level` (`None`: no tracer), through
/// `trace::set` / `trace::clear`, which stay `unsafe`: libgit2 keeps the trace state in
/// a global its traced routines read without a lock. Callers run in an isolated child,
/// single-threaded around the call, which meets their contract.
pub fn set_trace(level: TraceLevel, callback: Option<TraceCallback>) {
    match callback {
        // SAFETY: per this function's contract, no other thread is inside libgit2.
        Some(callback) => unsafe { trace::set(level, callback) },
        None => {
            assert_eq!(
                level,
                TraceLevel::None,
                "a level above `None` needs a callback"
            );
            // SAFETY: as above.
            unsafe { trace::clear() }
        }
    }
}

/// An HTTP server on loopback answering every request `404`, for the life of the
/// process; its URL. libgit2 traces only from its HTTP client.
pub fn serve_404() -> CString {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
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

/// Connects a detached remote to `url` for a fetch and frees it; the status. Stands in
/// for the `git_remote` wrapper (`remote`), which replaces it.
///
/// libgit2's HTTP client fills its parser settings lazily, in a static, without a lock
/// (`http_client_parser_settings`, `src/libgit2/transports/httpclient.c`): concurrent
/// first connections race there (TSan). Connect once on one thread before connecting
/// from several.
pub fn connect(url: &CStr) -> c_int {
    init();
    let mut remote = core::ptr::null_mut();
    // SAFETY: a live out-parameter and a C string, which libgit2 copies.
    let rc = unsafe { ffi::git_remote_create_detached(&raw mut remote, url.as_ptr()) };
    assert_eq!(rc, 0);
    // SAFETY: a live remote; NULL callbacks, proxy and headers take the defaults.
    let rc = unsafe {
        ffi::git_remote_connect(
            remote,
            ffi::GIT_DIRECTION_FETCH,
            core::ptr::null(),
            core::ptr::null(),
            core::ptr::null(),
        )
    };
    // SAFETY: the remote created above, freed once.
    unsafe { ffi::git_remote_free(remote) };
    rc
}

// --- `git2/repository.h`: `git_repository_item_t` -----------------------------------

use libgit2::libgit2::repository::repository::Item;

/// `git_repository_item_path(repo, item)`, adopted into a `Buf`: the path, or the
/// return. Stands in for the `git_repository_item_path` wrapper (`repository`), which
/// replaces it.
pub fn item_path(repo: &TempRepo, item: Item) -> Result<String, c_int> {
    let mut raw = ffi::git_buf::default();
    // SAFETY: an empty buffer, the fixture's live repository, and a value the safe
    // `Item` produced, which is the only contract this fixture adds: every `Item` is
    // below `GIT_REPOSITORY_ITEM__LAST`, the bound of the table libgit2 indexes.
    let rc = unsafe { ffi::git_repository_item_path(&raw mut raw, repo.repo, item.raw()) };
    // SAFETY: `raw` is as libgit2 left it: empty, or owning a NUL-terminated allocation
    // of its allocator.
    let buf = unsafe { Buf::from_raw(raw) };
    if rc < 0 {
        return Err(rc);
    }
    Ok(buf.as_ref().to_str().unwrap().to_owned())
}
