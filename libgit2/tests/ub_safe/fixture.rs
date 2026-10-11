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
use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
use libgit2::libgit2::util::errors::{ErrorCode, last};

/// Takes a `git_libgit2_init` reference that is never released.
pub fn init() {
    core::mem::forget(libgit2::libgit2::core::global::init().unwrap());
}

/// A fresh bare repository in a temporary directory, freed and removed on drop.
pub struct TempRepo {
    pub path: PathBuf,
    repo: Option<RepositoryBox>,
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
        let repo = Repository::init(&c_path, true).unwrap();
        Self {
            path,
            repo: Some(repo),
        }
    }

    /// The repository, for the raw calls of the fixtures below.
    fn raw(&self) -> *mut ffi::git_repository {
        self.repo.as_ref().unwrap().as_c_ptr()
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
        // Every stream the tests opened on the repository is gone by now (they borrow
        // the fixture); free it before its directory goes.
        drop(self.repo.take());
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `git_blob_create_from_stream` adopted into a `WriteStreamBox`: the one fixture
/// adopting a C stream. The blob wrapper (`object::blob::BlobWriter`) never lends its
/// stream as a `WriteStreamBox`, since a closed blob stream can no longer be committed,
/// so the `git_writestream` tests over a C stream keep this.
///
/// The repository must outlive the stream, which the borrow does not enforce.
pub fn blob_stream(repo: &TempRepo, hint: Option<&CStr>) -> WriteStreamBox {
    let mut out = core::ptr::null_mut();
    // SAFETY: a live slot, the live repository and a NULL or live hint path, which
    // libgit2 copies.
    let rc = unsafe {
        ffi::git_blob_create_from_stream(
            &mut out,
            repo.raw(),
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

/// [`RepositoryRef::item_path`](libgit2::libgit2::repository::repository::RepositoryRef::item_path)
/// as a string: the path, or the return.
pub fn item_path(repo: &TempRepo, item: Item) -> Result<String, c_int> {
    match repo.repo.as_ref().unwrap().as_ref().item_path(item) {
        Ok(buf) => Ok(buf.as_ref().to_str().unwrap().to_owned()),
        Err(error) => Err(error.raw_code()),
    }
}

// --- `git2/commit.h`: `git_commitarray`; `git2/config.h`: `git_config` --------------

/// Fixtures over the repositories that hand out commit arrays and configs, and
/// standing in for the `git_commit_id` and config-routine wrappers the tests read
/// through; replace each once its safe counterpart lands.
pub mod commit_and_config {
    use std::ffi::{CStr, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::config::config::ConfigArc;
    use libgit2::libgit2::object::commit::{CommitArrayVal, CommitRef};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    /// A fixture repository opened read-only, freed on drop.
    pub struct OpenRepo(RepositoryBox);

    /// `testrepo.git` (`HEAD` at `a65fedf`), `mergedrepo` (merging `e280915` into
    /// `3a34580`) and `empty_bare.git` (an unborn `HEAD`).
    pub const TESTREPO: &str = "testrepo.git";
    pub const MERGEDREPO: &str = "mergedrepo/.gitted";
    pub const EMPTY: &str = "empty_bare.git";

    impl OpenRepo {
        pub fn open(name: &str) -> Self {
            super::init();
            let path = std::ffi::CString::new(format!(
                "{}/../../../tests/resources/{name}",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap();
            Self(Repository::open(&path).unwrap())
        }
    }

    /// The commit parents, through the safe wrapper.
    pub fn commit_parents(repo: &OpenRepo) -> CommitArrayVal<'_> {
        repo.0.as_ref().commit_parents().unwrap()
    }

    /// The hex id of a commit; stands in for the `git_commit_id` wrapper.
    pub fn commit_id(commit: CommitRef<'_, '_>) -> String {
        let commit = commit.to_commit();
        let mut out = [0u8; 41];
        // SAFETY: the owner keeps the commit alive; 41 writable bytes, NUL included.
        unsafe {
            ffi::git_oid_tostr(
                out.as_mut_ptr().cast(),
                out.len(),
                ffi::git_commit_id(commit.as_c_ptr()),
            )
        };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// The repository's config, through the safe wrapper: a shared reference, which
    /// borrows the repository (its file backends read it on every reparse).
    pub fn config(repo: &OpenRepo) -> ConfigArc<'_> {
        repo.0.as_ref().config().unwrap()
    }

    /// `git_config_get_int32` on the config an owner's `as_c_ptr` gives; the value or
    /// the return code.
    pub fn get_int32(config: *mut ffi::git_config, name: &CStr) -> Result<i32, c_int> {
        let mut value = 0;
        // SAFETY: the tests pass a live owner's pointer; the call only reads it.
        let rc = unsafe { ffi::git_config_get_int32(&mut value, config, name.as_ptr()) };
        if rc < 0 { Err(rc) } else { Ok(value) }
    }

    /// `git_config_set_int32` on the config an owner's `as_c_ptr` gives; the return
    /// code.
    pub fn set_int32(config: *mut ffi::git_config, name: &CStr, value: i32) -> c_int {
        // SAFETY: the tests pass a live owner's pointer; the name is NUL-terminated.
        unsafe { ffi::git_config_set_int32(config, name.as_ptr(), value) }
    }

    /// `git_config_add_file_ondisk` then `git_config_set_int32`; the status of each.
    pub fn write_through_file(
        config: *mut ffi::git_config,
        file: &CStr,
        name: &CStr,
        value: i32,
    ) -> (c_int, c_int) {
        // SAFETY: the tests pass a live owner's pointer; the path is copied.
        let added = unsafe {
            ffi::git_config_add_file_ondisk(
                config,
                file.as_ptr(),
                ffi::GIT_CONFIG_LEVEL_LOCAL,
                ptr::null(),
                0,
            )
        };
        // SAFETY: as above; the name is NUL-terminated.
        let set = unsafe { ffi::git_config_set_int32(config, name.as_ptr(), value) };
        (added, set)
    }
}

// --- `git2/commit.h`: `git_commit_create_ext_options` ---------------------------------

/// Commit creation through C, standing in for the `git_commit_create_ext` wrapper (a
/// later run), which replaces it.
pub mod create_ext {
    use std::ffi::{CStr, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::object::commit::CreateExtOptions;

    use super::TempRepo;

    /// `git_commit_create_ext` of a root commit of the empty tree in `repo`, with a
    /// fixed signature and message, reading `options` in place: the new commit's raw
    /// text, or the code libgit2 failed with.
    pub fn create(repo: &TempRepo, options: &CreateExtOptions<'_>) -> Result<String, c_int> {
        let raw = repo.raw();
        let (mut builder, mut tree_id, mut tree, mut sig) = (
            ptr::null_mut(),
            ffi::git_oid::default(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        // SAFETY: a live repository; each out-parameter receives an object freed below.
        unsafe {
            assert_eq!(ffi::git_treebuilder_new(&mut builder, raw, ptr::null()), 0);
            assert_eq!(ffi::git_treebuilder_write(&mut tree_id, builder), 0);
            ffi::git_treebuilder_free(builder);
            assert_eq!(ffi::git_tree_lookup(&mut tree, raw, &tree_id), 0);
            assert_eq!(
                ffi::git_signature_new(&mut sig, c"A".as_ptr(), c"a@b".as_ptr(), 1, 0),
                0
            );
        }
        let mut id = ffi::git_oid::default();
        // SAFETY: live objects of the repository; the options are a live value Rust
        // holds (`CreateExtOptions` is transparent over the C struct), whose borrows
        // outlive the call by their `'o`.
        let rc = unsafe {
            ffi::git_commit_create_ext(
                &mut id,
                raw,
                sig,
                sig,
                c"msg\n".as_ptr(),
                tree,
                0,
                ptr::null(),
                ptr::from_ref(options).cast(),
            )
        };
        // SAFETY: each object is released once.
        unsafe {
            ffi::git_signature_free(sig);
            ffi::git_tree_free(tree);
        }
        if rc != 0 {
            return Err(rc);
        }
        let (mut odb, mut obj) = (ptr::null_mut(), ptr::null_mut());
        // SAFETY: the commit just written is read back and copied; each object is
        // released once.
        unsafe {
            assert_eq!(ffi::git_repository_odb(&mut odb, raw), 0);
            assert_eq!(ffi::git_odb_read(&mut obj, odb, &id), 0);
            let data = std::slice::from_raw_parts(
                ffi::git_odb_object_data(obj).cast::<u8>(),
                ffi::git_odb_object_size(obj),
            );
            let text = String::from_utf8(data.to_vec()).unwrap();
            ffi::git_odb_object_free(obj);
            ffi::git_odb_free(odb);
            Ok(text)
        }
    }

    /// Whether the reference `name` of `repo` exists.
    pub fn has_reference(repo: &TempRepo, name: &CStr) -> bool {
        let mut reference = ptr::null_mut();
        // SAFETY: a live repository and a NUL-terminated name; the reference is
        // released once.
        unsafe {
            let found = ffi::git_reference_lookup(&mut reference, repo.raw(), name.as_ptr()) == 0;
            ffi::git_reference_free(reference);
            found
        }
    }
}

// --- `git2/refdb.h`: `git_refdb` ---------------------------------------------------

use libgit2::libgit2::refs::refdb::{Refdb, RefdbBox};

impl TempRepo {
    /// A refdb of this repository, through [`Refdb::open`]. The borrow of the fixture is
    /// the refdb's `'r`, so the repository outlives it.
    pub fn refdb(&self) -> RefdbBox<'_> {
        Refdb::open(self.repo.as_ref().unwrap().as_ref()).unwrap()
    }
}

// --- `git2/odb.h`: `git_odb` -------------------------------------------------------

pub mod odb {
    //! Stand-ins for the odb readers and writers and for `git_repository_odb`, which
    //! their wrappers replace; each reaches the odb through its owner's raw seam.

    use libgit2::ffi;
    use libgit2::libgit2::odb::odb::OdbArc;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    /// `git_odb_write` of a blob; the id, or the error code.
    pub fn write_blob(odb: &OdbArc<'_>, data: &[u8]) -> Result<ffi::git_oid, i32> {
        let mut id = ffi::git_oid::default();
        // SAFETY: the odb is live while `odb` is borrowed; `id` is writable and `data`
        // live for its length, read for the call only.
        let rc = unsafe {
            ffi::git_odb_write(
                &mut id,
                odb.as_c_ptr(),
                data.as_ptr().cast(),
                data.len(),
                ffi::GIT_OBJECT_BLOB,
            )
        };
        if rc == 0 { Ok(id) } else { Err(rc) }
    }

    /// `git_odb_read_header`: the object's length and type, or the error code.
    pub fn read_header(
        odb: &OdbArc<'_>,
        id: &ffi::git_oid,
    ) -> Result<(usize, ffi::git_object_t), i32> {
        let (mut len, mut kind) = (0, 0);
        // SAFETY: the odb is live while `odb` is borrowed; the out slots are writable
        // and `id` live, read for the call only.
        let rc = unsafe { ffi::git_odb_read_header(&mut len, &mut kind, odb.as_c_ptr(), id) };
        if rc == 0 { Ok((len, kind)) } else { Err(rc) }
    }

    /// `git_odb_num_backends`.
    pub fn backends(odb: &OdbArc<'_>) -> usize {
        // SAFETY: the odb is live while `odb` is borrowed; the routine reads the count
        // under the odb's lock.
        unsafe { ffi::git_odb_num_backends(odb.as_c_ptr()) }
    }

    /// A repository over an odb ([`Repository::wrap_odb`]), which holds a reference of
    /// its own and becomes the odb's owner; freed on drop. [`odb`](Self::odb) stands in
    /// for the `git_repository_odb` wrapper, which lends the odb back, borrowing the
    /// repository.
    ///
    /// [`Repository::wrap_odb`]: libgit2::libgit2::repository::repository::Repository::wrap_odb
    pub struct Repo(RepositoryBox);

    impl Repo {
        pub fn wrap(odb: OdbArc<'static>) -> Self {
            Self(Repository::wrap_odb(odb).unwrap())
        }

        /// A new reference to the repository's odb (`RepositoryRef::odb`), borrowing
        /// the repository that owns it.
        pub fn odb(&self) -> OdbArc<'_> {
            self.0.as_ref().odb().unwrap()
        }
    }
}

// --- `git2/refs.h`: `git_reference` ------------------------------------------------

/// Attached references, read through a repository the fixture owns, and C's view of
/// any reference; stands in for the repository and reference wrappers.
pub mod refs {
    use core::ptr;
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use libgit2::ffi;
    use libgit2::libgit2::refs::refs::{ReferenceBox, UnattachedReferenceBox};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    /// A fresh bare repository in a temporary directory, freed and removed on drop.
    ///
    /// Its references borrow it.
    pub struct Repo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl Repo {
        pub fn new(tag: &str) -> Self {
            super::init();
            let path =
                std::env::temp_dir().join(format!("crustify-refs-ub-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, true).unwrap();
            // A packed tag with a peeled id, which no loose file overrides.
            std::fs::write(
                path.join("packed-refs"),
                format!(
                    "# pack-refs with: peeled fully-peeled sorted \n{} refs/tags/v1\n^{}\n",
                    "11".repeat(20),
                    "22".repeat(20)
                ),
            )
            .unwrap();
            Self {
                path,
                repo: Some(repo),
            }
        }

        /// `git_reference_lookup(name)`, adopted: the one fixture adopting an attached
        /// reference, which the reference lookup wrapper replaces.
        pub fn lookup(&self, name: &CStr) -> ReferenceBox<'_> {
            let mut out = ptr::null_mut();
            // SAFETY: a live slot, the live repository and a NUL-terminated name, read
            // for the call.
            let rc = unsafe { ffi::git_reference_lookup(&mut out, self.raw(), name.as_ptr()) };
            assert_eq!(rc, 0, "lookup {name:?}");
            // SAFETY: a fresh attached reference the caller owns, released by
            // `git_reference_free`; it borrows `self`, which frees the repository last.
            unsafe { ReferenceBox::from_c(out) }.unwrap()
        }

        /// The repository, as `git_reference_owner` reports it.
        pub fn raw(&self) -> *mut ffi::git_repository {
            self.repo.as_ref().unwrap().as_c_ptr()
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            // Its references borrowed `self` and are gone; free it before its directory.
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// What C reads out of a reference: its name, type, target id, peeled id and
    /// symbolic target.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct View {
        pub name: CString,
        pub kind: ffi::git_reference_t,
        pub target: Option<(u8, [u8; 32])>,
        pub peel: Option<(u8, [u8; 32])>,
        pub symbolic: Option<CString>,
    }

    fn id(oid: *const ffi::git_oid) -> Option<(u8, [u8; 32])> {
        // SAFETY: NULL, or an id inside a live reference, copied out at once.
        (!oid.is_null()).then(|| unsafe { ((*oid).type_, (*oid).id) })
    }

    /// C's view of the live reference at `p`.
    fn view_raw(p: *const ffi::git_reference) -> View {
        // SAFETY: `p` is a live reference, read for the calls only; every string and id
        // it hands back lives as long as it and is copied at once.
        unsafe {
            let symbolic = ffi::git_reference_symbolic_target(p);
            View {
                name: CStr::from_ptr(ffi::git_reference_name(p)).to_owned(),
                kind: ffi::git_reference_type(p),
                target: id(ffi::git_reference_target(p)),
                peel: id(ffi::git_reference_target_peel(p)),
                symbolic: (!symbolic.is_null()).then(|| CStr::from_ptr(symbolic).to_owned()),
            }
        }
    }

    /// C's view of an attached reference.
    pub fn view(reference: &ReferenceBox<'_>) -> View {
        view_raw(reference.as_c_ptr())
    }

    /// C's view of an unattached reference.
    pub fn view_unattached(reference: &UnattachedReferenceBox) -> View {
        view_raw(reference.as_c_ptr())
    }

    /// `git_reference_owner`: the repository an attached reference reaches.
    pub fn owner(reference: &ReferenceBox<'_>) -> *mut ffi::git_repository {
        // SAFETY: a live attached reference, whose repository outlives its borrow.
        unsafe { ffi::git_reference_owner(reference.as_c_ptr()) }
    }

    /// `git_reference_cmp` of two references, attached or not.
    pub fn cmp(a: *const ffi::git_reference, b: *const ffi::git_reference) -> i32 {
        // SAFETY: two live references, read for the call only.
        unsafe { ffi::git_reference_cmp(a, b) }
    }

    /// `UnattachedReferenceBox::from_c`, for a pointer an owner gave up with `into_c`,
    /// as libgit2 takes one from a backend.
    pub fn adopt_unattached(raw: *mut ffi::git_reference) -> UnattachedReferenceBox {
        // SAFETY: the caller passes a pointer `into_c` surrendered: a live unattached
        // reference nothing else owns, released by `git_reference_free`.
        unsafe { UnattachedReferenceBox::from_c(raw) }.unwrap()
    }
}

// --- `git2/index.h`: `git_index` ---------------------------------------------------

/// Fixtures over `git_index`, standing in for the index and repository symbol
/// wrappers, which replace them: they observe and change an index through raw calls on
/// an owner's pointer, which `as_c_ptr` hands out.
pub mod index {
    use std::ffi::{CStr, CString, c_int};

    use libgit2::ffi;
    use libgit2::libgit2::index::index::{IndexArc, IndexBox};

    use super::{CHILD, TempRepo};

    /// As [`isolated`](super::isolated), with LeakSanitizer off in the child: for bodies
    /// that drive libgit2's index reader down a path on which it leaks (a version 4
    /// entry cut short leaks its path, `read_entry` in `src/libgit2/index.c`). Every
    /// other sanitizer check stays on.
    pub fn isolated_tolerating_leaks(test: &str, body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
        let separator = if inherited.is_empty() { "" } else { ":" };
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .env(
                "ASAN_OPTIONS",
                format!("{inherited}{separator}detect_leaks=0"),
            )
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child `{name}` failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Either index owner, by its C pointer.
    pub trait Owner {
        fn c_ptr(&self) -> *mut ffi::git_index;
    }

    impl Owner for IndexBox {
        fn c_ptr(&self) -> *mut ffi::git_index {
            self.as_c_ptr()
        }
    }

    impl Owner for IndexArc {
        fn c_ptr(&self) -> *mut ffi::git_index {
            self.as_c_ptr()
        }
    }

    /// The repository's index, through the safe wrapper: a counted reference.
    pub fn repository_index(repo: &TempRepo) -> IndexArc {
        repo.repo.as_ref().unwrap().as_ref().index().unwrap()
    }

    /// `RepositoryMut::set_index` on the fixture's repository, which keeps a
    /// reference of its own: it takes an `IndexArc`.
    pub fn set_repository_index(repo: &mut TempRepo, index: &IndexArc) {
        repo.repo.as_mut().unwrap().as_mut().set_index(Some(index));
    }

    /// `git_index_entrycount`.
    pub fn entry_count(index: &impl Owner) -> usize {
        // SAFETY: a live index, only read.
        unsafe { ffi::git_index_entrycount(index.c_ptr()) }
    }

    /// `git_index_path`, copied out.
    pub fn path(index: &impl Owner) -> Option<CString> {
        // SAFETY: a live index, only read.
        let path = unsafe { ffi::git_index_path(index.c_ptr()) };
        // SAFETY: NULL or the index's own NUL-terminated path, copied before any other
        // call.
        (!path.is_null()).then(|| unsafe { CStr::from_ptr(path) }.to_owned())
    }

    /// Whether `git_index_owner` names a repository.
    pub fn has_owner(index: &impl Owner) -> bool {
        // SAFETY: a live index, only read; the repository pointer is not dereferenced.
        !unsafe { ffi::git_index_owner(index.c_ptr()) }.is_null()
    }

    /// `git_index_add` of a regular file at `path`; the status.
    pub fn add(index: &mut IndexBox, path: &CStr) -> c_int {
        add_flagged(index, path, 0)
    }

    /// `git_index_add` of a regular file at `path` marked intent-to-add, which needs a
    /// version 3 entry; the status.
    pub fn add_intent_to_add(index: &mut IndexBox, path: &CStr) -> c_int {
        add_flagged(index, path, ffi::GIT_INDEX_ENTRY_INTENT_TO_ADD as u16)
    }

    /// `git_index_set_version`; the status.
    pub fn set_version(index: &mut IndexBox, version: u32) -> c_int {
        // SAFETY: the box's live index, borrowed exclusively.
        unsafe { ffi::git_index_set_version(index.as_c_ptr(), version) }
    }

    fn add_flagged(index: &mut IndexBox, path: &CStr, flags_extended: u16) -> c_int {
        let mut entry = ffi::git_index_entry {
            mode: 0o100644,
            path: path.as_ptr(),
            flags_extended,
            ..Default::default()
        };
        entry.id.id[0] = 1;
        entry.id.type_ = ffi::GIT_OID_SHA1 as _;
        // SAFETY: the box's live index, borrowed exclusively; `entry` and its path are
        // read and copied for the call.
        unsafe { ffi::git_index_add(index.as_c_ptr(), &entry) }
    }

    /// `git_index_write`; the status.
    pub fn write(index: &mut IndexBox) -> c_int {
        // SAFETY: the box's live index, borrowed exclusively.
        unsafe { ffi::git_index_write(index.as_c_ptr()) }
    }
}

// --- `git2/worktree.h` ---------------------------------------------------------------

pub mod worktree {
    //! Repositories with linked worktrees laid out on disk, for the worktree wrappers
    //! to look up.

    use std::ffi::{CStr, CString};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};
    use libgit2::libgit2::repository::worktree::{Worktree, WorktreeBox};

    /// A non-bare repository in a temporary directory, to which [`add`](Self::add) lays
    /// out linked worktrees on disk as `git_worktree_add` would; freed and removed on
    /// drop.
    pub struct WorktreeRepo {
        pub base: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl WorktreeRepo {
        pub fn new(tag: &str) -> Self {
            super::init();
            let base = std::env::temp_dir()
                .join(format!("crustify-worktree-ub-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&base);
            let c_path = CString::new(base.join("repo").as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            Self {
                base,
                repo: Some(repo),
            }
        }

        /// The worktree `name`'s administrative directory.
        pub fn admin(&self, name: &str) -> PathBuf {
            self.base.join("repo/.git/worktrees").join(name)
        }

        /// The worktree `name`'s working directory.
        pub fn workdir(&self, name: &str) -> PathBuf {
            self.base.join("worktrees").join(name)
        }

        /// Lays out the worktree `name`: its administrative directory under
        /// `.git/worktrees/` and its working directory beside the repository.
        pub fn add(&self, name: &str) {
            let admin = self.admin(name);
            let workdir = self.workdir(name);
            fs::create_dir_all(&admin).unwrap();
            fs::create_dir_all(&workdir).unwrap();
            fs::write(admin.join("commondir"), "../..\n").unwrap();
            let gitlink = workdir.join(".git");
            fs::write(admin.join("gitdir"), format!("{}\n", gitlink.display())).unwrap();
            fs::write(admin.join("HEAD"), format!("ref: refs/heads/{name}\n")).unwrap();
            fs::write(gitlink, format!("gitdir: {}\n", admin.display())).unwrap();
        }

        /// The repository.
        pub fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }

        /// The worktree `name`.
        pub fn lookup(&self, name: &CStr) -> WorktreeBox {
            Worktree::lookup(self.repo(), name).unwrap()
        }

        /// Frees the repository and removes its directory, leaving the worktrees looked
        /// up in it.
        pub fn close(self) {}
    }

    impl Drop for WorktreeRepo {
        fn drop(&mut self) {
            // The worktrees do not borrow the repository; free it before its directory.
            drop(self.repo.take());
            let _ = fs::remove_dir_all(&self.base);
        }
    }
}

// --- `git2/annotated_commit.h` -----------------------------------------------------

/// Repositories holding commits to annotate, built through raw libgit2 calls (no safe
/// object writer exists yet), and the C views the tests compare against.
pub mod annotated_commit {
    use std::ffi::{CStr, CString, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr::{self, NonNull};
    use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::SeqCst};

    use libgit2::ffi;
    use libgit2::libgit2::oid::oid::OidType;
    use libgit2::libgit2::refs::annotated_commit::AnnotatedCommitBox;
    use libgit2::libgit2::refs::refs::ReferenceBox;
    use libgit2::libgit2::repository::repository::{
        InitFlag, InitOptions, Repository, RepositoryBox, RepositoryRef,
    };
    use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

    /// A branch name 99 bytes long, a length no other allocation of these tests asks
    /// for (100 bytes with its NUL).
    pub const LONG_BRANCH: &CStr = c"refs/heads/oooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooooo";
    /// A remote URL 155 bytes long (156 with its NUL).
    pub const LONG_URL: &CStr = c"https://example.invalid/uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuu.git";

    /// The hex form of `id`.
    pub fn hex(id: &ffi::git_oid) -> String {
        let mut buf = [0 as std::ffi::c_char; 65];
        // SAFETY: `buf` holds 65 bytes, which bounds the write; `id` is a live oid.
        unsafe { ffi::git_oid_tostr(buf.as_mut_ptr(), buf.len(), id) };
        // SAFETY: `git_oid_tostr` NUL-terminates within `buf`.
        unsafe { CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    }

    /// The commit id `commit` reads, as hex, and its reference name, through
    /// `git_annotated_commit_id` / `_ref`; replace with the accessors' wrappers.
    pub fn read(commit: &AnnotatedCommitBox<'_>) -> (String, Option<String>) {
        // SAFETY: `commit` is a live annotated commit, only read; both results point
        // into it and are copied while it lives.
        unsafe {
            let name = ffi::git_annotated_commit_ref(commit.as_c_ptr());
            (
                hex(&*ffi::git_annotated_commit_id(commit.as_c_ptr())),
                (!name.is_null()).then(|| CStr::from_ptr(name).to_string_lossy().into_owned()),
            )
        }
    }

    /// A fresh bare repository of object id type `kind` whose `main` (and `HEAD`) is a
    /// second commit over a first, with `refs/tags/v1` an annotated tag of it,
    /// `refs/tags/tree` a tree and [`LONG_BRANCH`] the first commit; freed and removed
    /// on drop.
    pub struct CommitRepo {
        path: PathBuf,
        repo: Option<RepositoryBox>,
        pub tree: ffi::git_oid,
        pub first: ffi::git_oid,
        pub second: ffi::git_oid,
        pub tag: ffi::git_oid,
    }

    impl CommitRepo {
        pub fn new(tag: &str, kind: ffi::git_oid_t) -> Self {
            super::init();
            let path = std::env::temp_dir().join(format!(
                "crustify-annotated-commit-ub-safe-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mut opts = InitOptions::new();
            let mut m = opts.as_mut();
            m.set_flags(InitFlag::Bare | InitFlag::Mkdir);
            m.set_initial_head(Some(c"main"));
            m.set_oid_type(OidType::from_raw(kind));
            let repo = Repository::init_ext(&c_path, opts.as_mut()).unwrap();
            let raw = repo.as_c_ptr();
            let mut odb = ptr::null_mut();
            // SAFETY: a live slot and the live repository; released below.
            assert_eq!(unsafe { ffi::git_repository_odb(&mut odb, raw) }, 0);
            let write = |data: &[u8], kind: ffi::git_object_t| {
                let mut id = ffi::git_oid::default();
                let rc =
                    // SAFETY: a live slot, the live odb and `data` readable for its length
                    // at a non-NULL address.
                    unsafe { ffi::git_odb_write(&mut id, odb, data.as_ptr().cast(), data.len(), kind) };
                assert_eq!(rc, 0);
                id
            };
            const WHO: &str = "A U Thor <author@example.com> 1700000000 +0000";
            let tree = write(b"", ffi::GIT_OBJECT_TREE);
            let first = write(
                format!(
                    "tree {}\nauthor {WHO}\ncommitter {WHO}\n\nfirst\n",
                    hex(&tree)
                )
                .as_bytes(),
                ffi::GIT_OBJECT_COMMIT,
            );
            let second = write(
                format!(
                    "tree {}\nparent {}\nauthor {WHO}\ncommitter {WHO}\n\nsecond\n",
                    hex(&tree),
                    hex(&first)
                )
                .as_bytes(),
                ffi::GIT_OBJECT_COMMIT,
            );
            let tag = write(
                format!(
                    "object {}\ntype commit\ntag v1\ntagger {WHO}\n\nv1\n",
                    hex(&second)
                )
                .as_bytes(),
                ffi::GIT_OBJECT_TAG,
            );
            // SAFETY: the odb reference taken above, released once.
            unsafe { ffi::git_odb_free(odb) };
            // References are written as loose files: `git_reference_create` takes a
            // reflog signature at the current time, whose glibc timezone lookup
            // ThreadSanitizer reports against other tests'.
            for (name, id) in [
                ("refs/heads/main", &second),
                ("refs/tags/v1", &tag),
                ("refs/tags/tree", &tree),
                (LONG_BRANCH.to_str().unwrap(), &first),
            ] {
                let file = path.join(name);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, format!("{}\n", hex(id))).unwrap();
            }
            Self {
                path,
                repo: Some(repo),
                tree,
                first,
                second,
                tag,
            }
        }

        pub fn repo(&self) -> RepositoryRef<'_> {
            self.repo.as_ref().unwrap().as_ref()
        }

        /// `git_reference_lookup(name)`, adopted: stands in for the reference lookup
        /// wrapper, which replaces it.
        pub fn reference(&self, name: &CStr) -> ReferenceBox<'_> {
            let mut out = ptr::null_mut();
            let raw = self.repo.as_ref().unwrap().as_c_ptr();
            // SAFETY: a live slot, the live repository and a NUL-terminated name.
            let rc = unsafe { ffi::git_reference_lookup(&mut out, raw, name.as_ptr()) };
            assert_eq!(rc, 0, "{name:?}");
            // SAFETY: a fresh attached reference the caller owns, released by
            // `git_reference_free`; it borrows `self`, which frees the repository last.
            unsafe { ReferenceBox::from_c(out) }.unwrap()
        }

        /// Leaves the repository open until the process exits, reachable from a
        /// static, so LeakSanitizer counts nothing its caches still reach as leaked.
        pub fn keep_open(mut self) {
            static KEPT: AtomicPtr<ffi::git_repository> = AtomicPtr::new(ptr::null_mut());
            KEPT.store(self.repo.take().unwrap().into_c(), SeqCst);
        }
    }

    impl Drop for CommitRepo {
        fn drop(&mut self) {
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    static LEN: AtomicUsize = AtomicUsize::new(0);
    static NTH: AtomicUsize = AtomicUsize::new(0);

    /// libc's allocator, refusing the `NTH` next request of exactly `LEN` bytes.
    struct Refusing;

    // SAFETY: forwards to libc's `malloc` family unchanged, or fails, which `Alloc`
    // allows; the counters are atomics.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if len == LEN.load(SeqCst) && NTH.fetch_sub(1, SeqCst) == 1 {
                LEN.store(0, SeqCst);
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
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            NonNull::new(unsafe { realloc(p, len) }.cast())
        }

        unsafe fn free(p: NonNull<u8>) {
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Installs an allocator refusing the `nth` next request of `len` bytes; libgit2's
    /// own allocator is libc's too, so every live block stays releasable. Call alone
    /// in a child process ([`isolated`](super::isolated)).
    pub fn refuse(len: usize, nth: usize) {
        static ALLOCATOR: Allocator = Allocator::new::<Refusing>();
        NTH.store(nth, SeqCst);
        LEN.store(len, SeqCst);
        super::install(&ALLOCATOR);
    }

    /// Whether the refusal armed by [`refuse`] was spent.
    pub fn refused() -> bool {
        LEN.load(SeqCst) == 0
    }
}

// --- `git2/worktree.h`: `git_worktree_add` -----------------------------------------

/// A repository to add linked worktrees to, through `Worktree::add`; its reference
/// lookup stands in for that routine's wrapper, which replaces it.
pub mod worktree_add {
    use std::ffi::{CStr, CString, c_int};
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::refs::refs::ReferenceBox;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};
    use libgit2::libgit2::repository::worktree::{AddOptions, Worktree, WorktreeBox};

    /// A fresh non-bare repository at `<base>/repo` whose `HEAD` is `main`, a commit of
    /// the empty tree (so a checkout loads no filter), with the branches `feature` and
    /// `alpha` and the lightweight tag `v1` on it; new worktrees go to
    /// `<base>/wt/<name>`. Freed and removed on drop.
    pub struct AddRepo {
        pub base: PathBuf,
        repo: Option<RepositoryBox>,
    }

    impl AddRepo {
        pub fn new(tag: &str) -> Self {
            super::init();
            let base = std::env::temp_dir().join(format!(
                "crustify-worktree-add-ub-{}-{tag}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            let path = base.join("repo");
            let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            let (mut odb, raw) = (ptr::null_mut(), repo.as_c_ptr());
            // SAFETY: a live slot and the live repository; released below.
            let rc = unsafe { ffi::git_repository_odb(&mut odb, raw) };
            assert_eq!(rc, 0);
            let write = |data: &[u8], kind: ffi::git_object_t| {
                let mut id = ffi::git_oid::default();
                let (bytes, len) = (data.as_ptr().cast(), data.len());
                // SAFETY: a live slot, the live odb and `data` readable for its length at
                // a non-NULL address.
                let rc = unsafe { ffi::git_odb_write(&mut id, odb, bytes, len, kind) };
                assert_eq!(rc, 0);
                super::annotated_commit::hex(&id)
            };
            const WHO: &str = "A U Thor <author@example.com> 1700000000 +0000";
            let tree = write(b"", ffi::GIT_OBJECT_TREE);
            let commit = format!("tree {tree}\nauthor {WHO}\ncommitter {WHO}\n\nfirst\n");
            let commit = write(commit.as_bytes(), ffi::GIT_OBJECT_COMMIT);
            // SAFETY: the odb reference taken above, released once.
            unsafe { ffi::git_odb_free(odb) };
            // Loose files: `git_reference_create` would take a reflog signature at the
            // current time, whose timezone lookup TSan reports against other tests'.
            for name in [
                "refs/heads/main",
                "refs/heads/feature",
                "refs/heads/alpha",
                "refs/tags/v1",
            ] {
                std::fs::write(path.join(".git").join(name), format!("{commit}\n")).unwrap();
            }
            std::fs::write(path.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
            // `git_worktree_add` creates the worktree's directory, not its parents.
            std::fs::create_dir(base.join("wt")).unwrap();
            Self {
                base,
                repo: Some(repo),
            }
        }

        /// `git_reference_lookup(name)`, adopted.
        pub fn reference(&self, name: &CStr) -> ReferenceBox<'_> {
            let mut out = ptr::null_mut();
            let raw = self.repo.as_ref().unwrap().as_c_ptr();
            // SAFETY: a live slot, the live repository and a NUL-terminated name.
            let rc = unsafe { ffi::git_reference_lookup(&mut out, raw, name.as_ptr()) };
            assert_eq!(rc, 0, "{name:?}");
            // SAFETY: a fresh attached reference the caller owns, released by
            // `git_reference_free`; it borrows `self`, which frees the repository last.
            unsafe { ReferenceBox::from_c(out) }.unwrap()
        }

        /// `Worktree::add(repo, name, <base>/wt/<name>, opts)`: the new worktree, or the
        /// error code and message.
        pub fn add(
            &self,
            name: &str,
            opts: Option<&AddOptions<'_>>,
        ) -> Result<WorktreeBox, (c_int, String)> {
            let c_name = CString::new(name).unwrap();
            let dir = self.base.join("wt").join(name);
            let c_dir = CString::new(dir.as_os_str().as_bytes()).unwrap();
            let repo = self.repo.as_ref().unwrap().as_ref();
            Worktree::add(repo, &c_name, &c_dir, opts.map(AddOptions::as_ref))
                .map_err(|e| (e.raw_code(), e.message().to_string_lossy().into_owned()))
        }

        /// The repository.
        pub fn repo(&self) -> &RepositoryBox {
            self.repo.as_ref().unwrap()
        }

        /// The new worktree `name`'s administrative directory.
        pub fn admin(&self, name: &str) -> PathBuf {
            self.base.join("repo/.git/worktrees").join(name)
        }
    }

    impl Drop for AddRepo {
        fn drop(&mut self) {
            // Its references borrowed `self` and are gone; free it before its directory.
            drop(self.repo.take());
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }
}

// --- `git2/sys/repository.h`: the submodule cache ------------------------------------

pub mod submodule {
    //! A repository with one configured submodule, and submodules looked up through
    //! `git_submodule_lookup`, standing in for the submodule wrappers.

    use std::ffi::{CStr, CString};
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    use libgit2::ffi;
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    /// A non-bare repository whose `.gitmodules` configures the submodule `sub`;
    /// removed on drop.
    pub struct ModulesRepo {
        pub base: PathBuf,
        pub repo: RepositoryBox,
    }

    impl ModulesRepo {
        pub fn new(tag: &str) -> Self {
            super::init();
            let base = std::env::temp_dir().join(format!(
                "crustify-submodule-ub-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&base);
            let c_path = CString::new(base.as_os_str().as_bytes()).unwrap();
            let repo = Repository::init(&c_path, false).unwrap();
            let this = Self { base, repo };
            this.set_ignore("none");
            this
        }

        /// Rewrites `.gitmodules` with `sub`'s `ignore` setting.
        pub fn set_ignore(&self, ignore: &str) {
            fs::write(
                self.base.join(".gitmodules"),
                format!(
                    "[submodule \"sub\"]\n\tpath = sub\n\turl = https://example.com/sub\n\
                     \tignore = {ignore}\n"
                ),
            )
            .unwrap();
        }
    }

    impl Drop for ModulesRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    /// A counted submodule reference from `git_submodule_lookup`, released by
    /// `git_submodule_free`. It keeps a bare pointer to its repository, but neither
    /// reader here (`git_submodule_ignore`) nor the free follows it, so it may outlive
    /// the repository.
    pub struct Submodule(*mut ffi::git_submodule);

    impl Submodule {
        pub fn lookup(repo: &RepositoryBox, name: &CStr) -> Result<Self, i32> {
            let mut sm = core::ptr::null_mut();
            // SAFETY: a writable slot, the box's live repository on this thread and a
            // NUL-terminated name, read for the call.
            let rc = unsafe { ffi::git_submodule_lookup(&mut sm, repo.as_c_ptr(), name.as_ptr()) };
            if rc == 0 { Ok(Self(sm)) } else { Err(rc) }
        }

        /// `git_submodule_ignore`: the setting the submodule was loaded with.
        pub fn ignore(&self) -> ffi::git_submodule_ignore_t {
            // SAFETY: the live submodule this reference counts; only its fields are read.
            unsafe { ffi::git_submodule_ignore(self.0) }
        }
    }

    impl Drop for Submodule {
        fn drop(&mut self) {
            // SAFETY: the one reference this value holds, released once.
            unsafe { ffi::git_submodule_free(self.0) }
        }
    }
}

// --- `git2/repository.h`: routines (head and lookups) --------------------------------

/// Copies of libgit2's test repositories.
pub mod head_and_lookups {
    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox};

    const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/resources");

    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            let kind = entry.file_type().unwrap();
            if kind.is_symlink() {
                std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), &target).unwrap();
            } else if kind.is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// Copies of `tests/resources` repositories under a scratch directory, removed on
    /// drop: `bare` (`testrepo.git`), `testrepo` with its linked worktree
    /// `testrepo-worktree`, and `merged` (`mergedrepo`, mid-merge).
    pub struct Sandbox(pub PathBuf);

    impl Sandbox {
        pub fn new(tag: &str) -> Self {
            super::init();
            let root = std::env::temp_dir()
                .join(format!("crustify-routines-ub-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            for (name, to) in [
                ("testrepo.git", "bare"),
                ("testrepo", "testrepo"),
                ("testrepo-worktree", "testrepo-worktree"),
                ("mergedrepo", "merged"),
            ] {
                let to = root.join(to);
                copy_tree(&Path::new(RESOURCES).join(name), &to);
                if to.join(".gitted").exists() {
                    fs::rename(to.join(".gitted"), to.join(".git")).unwrap();
                }
            }
            Self(root)
        }

        pub fn c_path(&self, rel: &str) -> CString {
            CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
        }

        pub fn open(&self, rel: &str) -> RepositoryBox {
            Repository::open(&self.c_path(rel)).unwrap()
        }

        pub fn write(&self, rel: &str, contents: &str) {
            fs::write(self.0.join(rel), contents).unwrap();
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

// --- `git2/tree.h`: `git_treebuilder` ----------------------------------------------

/// Fixtures over the trees and tree builders the tests drive, through their safe
/// getters.
pub mod tree {
    use std::ffi::{CStr, CString};

    use libgit2::ffi;
    use libgit2::libgit2::object::tree::{TreeArc, TreeBuilderBox};
    use libgit2::libgit2::oid::oid::{Oid, OidType};

    /// Each entry's name and raw file mode, of `tree`, through the safe getters.
    pub fn entries(tree: &TreeArc<'_>) -> Vec<(CString, ffi::git_filemode_t)> {
        let tree = tree.as_ref();
        let entries: Vec<_> = tree
            .iter()
            .map(|entry| (entry.name().to_owned(), entry.filemode_raw()))
            .collect();
        assert_eq!(entries.len(), tree.len());
        entries
    }

    /// A SHA1 object id of `byte` repeated, for entries libgit2 does not check.
    pub fn id(byte: u8) -> Oid {
        Oid::from_raw(&[byte; 20], OidType::Sha1).unwrap()
    }

    /// The builder's entry count, through the safe `len`.
    pub fn count(builder: &TreeBuilderBox<'_>) -> usize {
        builder.as_ref().len()
    }

    /// The raw file mode of the builder's entry `name`, if it holds one.
    pub fn mode_of(builder: &TreeBuilderBox<'_>, name: &CStr) -> Option<ffi::git_filemode_t> {
        Some(builder.as_ref().get(name)?.filemode_raw())
    }
}

// --- `git2/object.h`: `git_object`; `git2/tree.h`: `git_tree_entry` -----------------

/// Fixtures over `git_object` and `git_tree_entry`: C's view of an owner, through the
/// pointer `as_c_ptr` hands out, standing in for the object and entry getters.
pub mod object_and_tree_entry {
    use std::ffi::CString;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::{FileMode, ObjectType};
    use libgit2::libgit2::object::blob::create_from_buffer;
    use libgit2::libgit2::object::object::{Object, ObjectArc};
    use libgit2::libgit2::object::tree::{TreeBuilder, TreeEntryBox};
    use libgit2::libgit2::oid::oid::{Oid, OidType};
    use libgit2::libgit2::repository::repository::{Repository, RepositoryBox, RepositoryRef};

    use super::TempRepo;

    /// `testrepo.git`, opened read-only: lookups only fill its object cache.
    pub fn open_testrepo() -> RepositoryBox {
        super::init();
        let path = CString::new(format!(
            "{}/../../../tests/resources/testrepo.git",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Repository::open(&path).unwrap()
    }

    /// A temporary repository's handle.
    pub fn repository(repo: &TempRepo) -> RepositoryRef<'_> {
        repo.repo.as_ref().unwrap().as_ref()
    }

    /// The hex id and C's type of the object an owner holds; the type stands in for
    /// `git_object_type`'s wrapper (a later run), which replaces it.
    pub fn object_view(object: &ObjectArc<'_>) -> (String, ffi::git_object_t) {
        let id = object.as_ref().id().to_hex_string().unwrap();
        // SAFETY: the live object the owner holds, only read.
        (id, unsafe { ffi::git_object_type(object.as_c_ptr()) })
    }

    /// A new blob of `content` in `repo`, looked up as an object; its hex id is read
    /// through [`object_view`].
    pub fn write_blob<'r>(repo: &'r RepositoryBox, content: &[u8]) -> ObjectArc<'r> {
        let id = create_from_buffer(repo.as_ref(), content).unwrap();
        Object::lookup(repo.as_ref(), id.as_ref(), Some(ObjectType::Blob)).unwrap()
    }

    /// The name, hex id and raw file mode of the entry an owner holds, through the
    /// safe getters.
    pub fn entry_view(entry: &TreeEntryBox) -> (CString, String, u32) {
        let entry = entry.as_ref();
        (
            entry.name().to_owned(),
            entry.id().to_hex_string().unwrap(),
            entry.filemode_raw(),
        )
    }

    /// Writes a blob holding `data` and a tree naming it once under each of `names`,
    /// through a tree builder; the tree's id.
    pub fn write_tree(repo: &TempRepo, data: &[u8], names: &[CString]) -> Oid {
        let mut blob = ffi::git_oid::default();
        // SAFETY: a writable id, the live repository, and `data`, read for the call.
        let rc = unsafe {
            ffi::git_blob_create_from_buffer(
                &mut blob,
                repo.raw(),
                data.as_ptr().cast(),
                data.len(),
            )
        };
        assert_eq!(rc, 0);
        let blob = Oid::from_raw(&blob.id[..20], OidType::Sha1).unwrap();
        let mut builder = TreeBuilder::new(repository(repo)).unwrap();
        for name in names {
            let inserted = builder.as_mut().insert(name, blob.as_ref(), FileMode::Blob);
            assert!(inserted.is_ok(), "{name:?}: {inserted:?}");
        }
        builder.as_mut().write().unwrap()
    }

    /// `git_odb_write` of `data` as an object of type `kind`, which the object
    /// database stores without parsing it; the id.
    pub fn write_raw(repo: &TempRepo, data: &[u8], kind: ffi::git_object_t) -> Oid {
        let odb = repository(repo).odb().unwrap();
        let mut id = ffi::git_oid::default();
        // SAFETY: the odb is live while `odb` is held; `id` is writable and `data` live
        // for its length, read for the call only.
        let rc = unsafe {
            ffi::git_odb_write(
                &mut id,
                odb.as_c_ptr(),
                data.as_ptr().cast(),
                data.len(),
                kind,
            )
        };
        assert_eq!(rc, 0);
        Oid::from_raw(&id.id[..20], OidType::Sha1).unwrap()
    }

    /// Sets libgit2's object cache limit, in a child process that runs nothing else.
    pub fn set_cache_max_size(bytes: isize) {
        // SAFETY: the isolated children that call it run one thread meanwhile.
        unsafe { libgit2::libgit2::core::common::set_cache_max_size(bytes) }
    }

    /// Enables or disables libgit2's object caches, in a child process that runs
    /// nothing else.
    pub fn enable_caching(enabled: bool) {
        // SAFETY: as above.
        unsafe { libgit2::libgit2::core::common::enable_caching(enabled) }
    }
}

// --- `git2/filter.h`: `git_filter_list` ---------------------------------------------

use libgit2::libgit2::filter::filter::FilterListBox;

/// `git_filter_list_apply_to_buffer(out, list, data)`, standing in for the filter
/// list's apply wrapper (a later run), which replaces it: the filtered bytes, or the
/// code libgit2 failed with. Filters write the list's state while it applies, hence
/// the exclusive borrow.
pub fn apply(list: &mut FilterListBox<'_>, data: &[u8]) -> Result<Vec<u8>, c_int> {
    let mut out = ffi::git_buf::default();
    // SAFETY: a zeroed buffer libgit2 fills; the live list, borrowed exclusively for the
    // call, whose repository outlives it by its borrow; `data` live for the call.
    let rc = unsafe {
        ffi::git_filter_list_apply_to_buffer(
            &mut out,
            list.as_c_ptr(),
            data.as_ptr().cast(),
            data.len(),
        )
    };
    let bytes = if rc < 0 || out.ptr.is_null() {
        Vec::new()
    } else {
        // SAFETY: on success `out` holds `size` initialised bytes at `ptr`.
        unsafe { std::slice::from_raw_parts(out.ptr.cast::<u8>(), out.size) }.to_vec()
    };
    // SAFETY: the buffer libgit2 filled (or left zeroed), released once.
    unsafe { ffi::git_buf_dispose(&mut out) };
    if rc < 0 { Err(rc) } else { Ok(bytes) }
}

/// The number of filters in `list`: `git_filter_list_length`, standing in for its
/// wrapper (a later run).
pub fn filter_count(list: &FilterListBox<'_>) -> usize {
    // SAFETY: the live list, only read.
    unsafe { ffi::git_filter_list_length(list.as_c_ptr()) }
}

/// A probe filter for the blob-taking filter list constructors: registered once as
/// `crustify-probe` and selected where the `crustify-probe` attribute is set, its
/// `check` records the id the filter source carries (the blob's) and keeps a payload of
/// its own in the list, which its `cleanup` frees when the list is. Stands in for a
/// filter written through the `git2/sys/filter.h` wrappers (a later run).
pub mod filter_probe {
    use std::cell::RefCell;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::OnceLock;

    use libgit2::ffi;

    thread_local! {
        /// What the probe's `check` saw on this thread: the source's id, hex, if any.
        static SEEN: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
        /// Payloads the probe's `check` made and its `cleanup` freed on this thread.
        static PAYLOADS: RefCell<(usize, usize)> = const { RefCell::new((0, 0)) };
    }

    fn hex(id: *const ffi::git_oid) -> String {
        let mut out = [0u8; 41];
        // SAFETY: a live id; `out` holds 41 bytes, NUL included.
        unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), id) };
        CStr::from_bytes_until_nul(&out)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned()
    }

    /// `git_filter.check` of the probe: records the source's id and keeps a payload.
    unsafe extern "C" fn probe_check(
        _filter: *mut ffi::git_filter,
        payload: *mut *mut c_void,
        src: *const ffi::git_filter_source,
        _values: *mut *const c_char,
    ) -> c_int {
        // SAFETY: libgit2's live source, read for the call.
        let id = unsafe { ffi::git_filter_source_id(src) };
        SEEN.with_borrow_mut(|seen| seen.push((!id.is_null()).then(|| hex(id))));
        PAYLOADS.with_borrow_mut(|p| p.0 += 1);
        // SAFETY: libgit2's live payload slot; the list hands it back to `probe_cleanup`.
        unsafe { payload.write(Box::into_raw(Box::new(0x5eed_u64)).cast()) };
        0
    }

    /// `git_filter.cleanup` of the probe: frees the payload `probe_check` made.
    unsafe extern "C" fn probe_cleanup(_filter: *mut ffi::git_filter, payload: *mut c_void) {
        // SAFETY: the payload `probe_check` boxed, handed back once by the list's free.
        let value = unsafe { Box::from_raw(payload.cast::<u64>()) };
        assert_eq!(*value, 0x5eed);
        PAYLOADS.with_borrow_mut(|p| p.1 += 1);
    }

    /// Registers the probe once; it is never unregistered, so outlives every list. The
    /// caller then loads a list selecting it before any test loads its own: libgit2
    /// marks a registered filter initialised, unsynchronised, the first time a list
    /// selects it (the crate does so for the built-in ones only).
    pub fn register() {
        static PROBE: OnceLock<()> = OnceLock::new();
        PROBE.get_or_init(|| {
            super::init();
            let filter = Box::leak(Box::new(ffi::git_filter::default()));
            // SAFETY: a live filter struct, filled with the version's defaults.
            let rc = unsafe { ffi::git_filter_init(filter, ffi::GIT_FILTER_VERSION) };
            assert_eq!(rc, 0);
            filter.attributes = c"+crustify-probe".as_ptr();
            filter.check = Some(probe_check);
            filter.cleanup = Some(probe_cleanup);
            // SAFETY: a NUL-terminated name and a filter that lives for the process.
            let rc = unsafe { ffi::git_filter_register(c"crustify-probe".as_ptr(), filter, 0) };
            assert_eq!(rc, 0);
        });
    }

    /// The ids the probe saw on this thread since the last call, and the payloads
    /// still unfreed.
    pub fn seen() -> (Vec<Option<String>>, usize) {
        let seen = SEEN.with_borrow_mut(std::mem::take);
        let (made, freed) = PAYLOADS.with_borrow(|p| *p);
        (seen, made - freed)
    }
}

// --- `git2/blob.h`: `git_blob`, `git_blob_filter_options` ----------------------------

/// Stand-ins for the blob routines a later run wraps (`git_blob_rawcontent`,
/// `git_blob_rawsize`, `git_blob_filter`), which replaces them, and the blob cache
/// limit, a process-global option.
pub mod blob {
    use std::ffi::{CStr, c_int};
    use std::ptr;

    use libgit2::ffi;
    use libgit2::libgit2::core::types::ObjectType;
    use libgit2::libgit2::object::blob::{BlobArc, FilterOptions};

    /// A copy of the content of the blob an owner holds.
    pub fn content(blob: &BlobArc<'_>) -> Vec<u8> {
        let p = blob.as_c_ptr();
        // SAFETY: `p` is the live blob the owner holds; its content lives as long as
        // it does and is copied at once.
        unsafe {
            let size = usize::try_from(ffi::git_blob_rawsize(p)).unwrap();
            let data = ffi::git_blob_rawcontent(p).cast::<u8>();
            if size == 0 {
                return Vec::new();
            }
            std::slice::from_raw_parts(data, size).to_vec()
        }
    }

    /// Lets the object cache keep blobs of up to `bytes`, in a child process that runs
    /// nothing else.
    pub fn set_blob_cache_limit(bytes: usize) {
        // SAFETY: the isolated children that call it run one thread meanwhile.
        unsafe { libgit2::libgit2::core::common::set_cache_object_limit(ObjectType::Blob, bytes) }
    }

    /// `git_blob_filter` of `blob` as `path`, with `opts` read in place (or none): the
    /// filtered bytes, or the code libgit2 failed with. Filter lists race on their first
    /// load, so callers run in an isolated child.
    pub fn filter(
        blob: &BlobArc<'_>,
        path: &CStr,
        opts: Option<&mut FilterOptions>,
    ) -> Result<Vec<u8>, c_int> {
        let mut out = ffi::git_buf::default();
        // `FilterOptions` is `#[repr(transparent)]` over `git_blob_filter_options`.
        let opts = opts.map_or(ptr::null_mut(), |o| {
            ptr::from_mut(o).cast::<ffi::git_blob_filter_options>()
        });
        // SAFETY: a zeroed buffer libgit2 fills; the live blob the owner holds, whose
        // repository outlives it by its borrow; `path` NUL-terminated; `opts` NULL or
        // live, initialised options, which libgit2 copies for the call.
        let rc = unsafe { ffi::git_blob_filter(&mut out, blob.as_c_ptr(), path.as_ptr(), opts) };
        let bytes = if rc < 0 || out.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: on success `out` holds `size` initialised bytes at `ptr`.
            unsafe { std::slice::from_raw_parts(out.ptr.cast::<u8>(), out.size) }.to_vec()
        };
        // SAFETY: the buffer libgit2 filled (or left zeroed), released once.
        unsafe { ffi::git_buf_dispose(&mut out) };
        if rc < 0 { Err(rc) } else { Ok(bytes) }
    }
}

// --- `git2/object.h`: `git_object_id_options` ---------------------------------------

/// `git_object_id_from_buffer(out, data, len, opts)`, standing in for its wrapper (a
/// later run), which replaces it: the id's hex digits, or the code libgit2 failed with.
/// Applying the options' filter list writes its filters' state, hence the exclusive
/// borrow.
pub fn object_id(
    opts: &mut libgit2::libgit2::object::object::ObjectIdOptions<'_>,
    data: &[u8],
) -> Result<String, c_int> {
    let mut out = ffi::git_oid::default();
    // An empty slice's pointer is dangling, and libgit2 hashes from it even for a length
    // of 0: lend a live byte instead.
    let bytes = if data.is_empty() { &[0u8][..] } else { data };
    // `ObjectIdOptions` is transparent over the C struct, which Rust owns inline.
    let raw = core::ptr::from_mut(opts).cast::<ffi::git_object_id_options>();
    // SAFETY: a writable id; `data.len()` readable bytes; live options, borrowed
    // exclusively for the call, whose list (if any) is live and lent to them alone.
    let rc =
        unsafe { ffi::git_object_id_from_buffer(&mut out, bytes.as_ptr().cast(), data.len(), raw) };
    if rc < 0 {
        return Err(rc);
    }
    let mut hex = [0u8; 65];
    // SAFETY: the id libgit2 wrote; `hex` holds 65 bytes, NUL included.
    unsafe { ffi::git_oid_tostr(hex.as_mut_ptr().cast(), hex.len(), &out) };
    Ok(CStr::from_bytes_until_nul(&hex)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned())
}
