//! UB tests for the unsafe API of `git2/common.h`: the `git_libgit2_opts` setters, the
//! cache-size query and the allocator switch, each called within its contract, edge
//! cases included.
//!
//! Their contract is that no other thread calls into libgit2 while they run, and this
//! binary's other suites call libgit2 from parallel test threads, so every suite here
//! reruns alone in a child process, single-threaded around each call. Threads that
//! read a setting start after the setter returned, which orders them after it.
//! Strings set replace strings libgit2 frees (ASan, LSan); numbers set reach libgit2's
//! globals with no out-of-range read (UBSan); the string results are copied out.

use std::ffi::{CStr, CString, c_int, c_void};
use std::process::Command;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering::SeqCst;
use std::thread;

use libgit2::libgit2::core::common::{self, SearchPathLevel};
use libgit2::libgit2::core::types::ObjectType;
use libgit2::libgit2::util::errors::{ErrorBuf, ErrorClass};
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

const CHILD: &str = "CRUSTIFY_UB_UNSAFE_CHILD";

/// Runs `body` alone in a child process, where nothing has initialised libgit2 and no
/// other thread calls it.
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

/// Reads `get` on four threads, after the setter that ran before this returned.
fn read_on_threads<T: PartialEq + std::fmt::Debug + Send + Sync>(get: fn() -> T, want: T) {
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| assert_eq!(get(), want));
        }
    });
}

/// A size setter round-trips through its getter, extremes included.
fn sizes(set: unsafe fn(usize), get: fn() -> usize) {
    let default = get();
    for n in [0, 1, 4096, usize::MAX / 2, usize::MAX, default] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { set(n) };
        assert_eq!(get(), n);
    }
    read_on_threads(get, default);
}

/// A switch takes both values, repeatedly.
fn switch(set: unsafe fn(bool)) {
    for enabled in [false, true, true, false, true] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { set(enabled) };
    }
}

macro_rules! isolated_test {
    ($(#[$doc:meta])* $name:ident, $body:block) => {
        $(#[$doc])*
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn $name() {
            isolated(concat!(module_path!(), "::", stringify!($name)), || $body);
        }
    };
}

isolated_test!(set_mwindow_size, {
    sizes(common::set_mwindow_size, common::mwindow_size);
});

isolated_test!(set_mwindow_mapped_limit, {
    sizes(
        common::set_mwindow_mapped_limit,
        common::mwindow_mapped_limit,
    );
});

isolated_test!(set_mwindow_file_limit, {
    sizes(common::set_mwindow_file_limit, common::mwindow_file_limit);
});

isolated_test!(set_pack_max_objects, {
    sizes(common::set_pack_max_objects, common::pack_max_objects);
});

isolated_test!(set_pack_max_object_size, {
    sizes(
        common::set_pack_max_object_size,
        common::pack_max_object_size,
    );
});

isolated_test!(
    /// Every level: set, `$PATH` expansion, reset, an empty and a long path, each string
    /// replacing (and freeing) the last; the first call pins libgit2 itself.
    set_search_path,
    {
        let long = CString::new("d/".repeat(32 * 1024)).unwrap();
        for level in [
            SearchPathLevel::System,
            SearchPathLevel::Xdg,
            SearchPathLevel::Global,
        ] {
            // SAFETY: this child runs no other thread while setting.
            unsafe {
                common::set_search_path(level, Some(c"/one")).unwrap();
                assert_eq!(common::search_path(level).unwrap(), c"/one".to_owned());
                common::set_search_path(level, Some(c"/zero:$PATH:/two")).unwrap();
                assert_eq!(
                    common::search_path(level).unwrap(),
                    c"/zero:/one:/two".to_owned()
                );
                common::set_search_path(level, Some(c"")).unwrap();
                assert_eq!(common::search_path(level).unwrap(), CString::default());
                common::set_search_path(level, Some(c"$PATH")).unwrap();
                assert_eq!(common::search_path(level).unwrap(), CString::default());
                common::set_search_path(level, Some(&long)).unwrap();
                assert_eq!(common::search_path(level).unwrap(), long);
                common::set_search_path(level, None).unwrap();
            }
            let _ = common::search_path(level).unwrap();
        }
        read_on_threads(
            || common::search_path(SearchPathLevel::Xdg).unwrap(),
            common::search_path(SearchPathLevel::Xdg).unwrap(),
        );
    }
);

isolated_test!(set_template_path, {
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_template_path(Some(c"/tmp/templates")).unwrap();
        assert_eq!(
            common::template_path().unwrap(),
            c"/tmp/templates".to_owned()
        );
        common::set_template_path(Some(c"")).unwrap();
        assert_eq!(common::template_path().unwrap(), CString::default());
        common::set_template_path(None).unwrap();
        common::set_template_path(Some(c"/again")).unwrap();
    }
    read_on_threads(|| common::template_path().unwrap(), c"/again".to_owned());
});

isolated_test!(set_homedir, {
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_homedir(Some(c"/home/crustify")).unwrap();
        assert_eq!(common::homedir().unwrap(), c"/home/crustify".to_owned());
        common::set_homedir(Some(c"")).unwrap();
        assert_eq!(common::homedir().unwrap(), CString::default());
        common::set_homedir(None).unwrap();
        common::set_homedir(Some(c"/h")).unwrap();
    }
    read_on_threads(|| common::homedir().unwrap(), c"/h".to_owned());
});

isolated_test!(set_user_agent, {
    let long = CString::new("a".repeat(100_000)).unwrap();
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_user_agent(Some(c"crustify/1")).unwrap();
        assert_eq!(common::user_agent().unwrap(), c"crustify/1".to_owned());
        common::set_user_agent(Some(c"")).unwrap();
        assert_eq!(common::user_agent().unwrap(), CString::default());
        common::set_user_agent(Some(&long)).unwrap();
        assert_eq!(common::user_agent().unwrap(), long);
        common::set_user_agent(None).unwrap();
        assert_eq!(common::user_agent().unwrap(), c"libgit2 1.9.0".to_owned());
        common::set_user_agent(Some(c"kept")).unwrap();
    }
    read_on_threads(|| common::user_agent().unwrap(), c"kept".to_owned());
});

isolated_test!(set_user_agent_product, {
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_user_agent_product(Some(c"git/2.47")).unwrap();
        assert_eq!(
            common::user_agent_product().unwrap(),
            c"git/2.47".to_owned()
        );
        common::set_user_agent_product(Some(c"")).unwrap();
        assert_eq!(common::user_agent_product().unwrap(), CString::default());
        common::set_user_agent_product(None).unwrap();
        assert_eq!(common::user_agent_product().unwrap(), c"git/2.0".to_owned());
        common::set_user_agent_product(Some(c"kept")).unwrap();
    }
    read_on_threads(|| common::user_agent_product().unwrap(), c"kept".to_owned());
});

isolated_test!(set_ssl_ciphers, {
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_ssl_ciphers(c"ECDHE-RSA-AES128-GCM-SHA256").unwrap();
        common::set_ssl_ciphers(c"").unwrap();
        common::set_ssl_ciphers(c"HIGH:!aNULL").unwrap();
    }
});

isolated_test!(set_extensions, {
    /// libgit2's built-in extensions, sorted.
    const BUILTIN: [&CStr; 6] = [
        c"noop",
        c"objectformat",
        c"preciousobjects",
        c"refstorage",
        c"relativeworktrees",
        c"worktreeconfig",
    ];
    let sorted = |extra: &[&CStr]| {
        let mut v: Vec<CString> = BUILTIN
            .iter()
            .chain(extra)
            .map(|s| (*s).to_owned())
            .collect();
        v.sort();
        v
    };
    let current = || {
        let e = common::extensions().unwrap();
        e.as_ref()
            .iter()
            .map(CStr::to_owned)
            .collect::<Vec<CString>>()
    };
    let negated: Vec<CString> = BUILTIN
        .iter()
        .map(|s| CString::new([b"!", s.to_bytes()].concat()).unwrap())
        .collect();
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_extensions::<&CStr>(&[]).unwrap();
        assert_eq!(current(), sorted(&[]));
        common::set_extensions(&[c"zed", c"noop", c"alpha", c"zed"]).unwrap();
        assert_eq!(current(), sorted(&[c"alpha", c"zed"]));
        common::set_extensions(&negated).unwrap();
        assert_eq!(
            current(),
            Vec::<CString>::new(),
            "an empty list, still allocated"
        );
        let many: Vec<CString> = (0..500)
            .map(|i| CString::new(format!("x{i:03}")).unwrap())
            .collect();
        common::set_extensions(&many).unwrap();
        assert_eq!(current().len(), 506);
        common::set_extensions(&[c"kept"]).unwrap();
    }
    read_on_threads(current, sorted(&[c"kept"]));
});

isolated_test!(set_owner_validation, {
    for enabled in [false, true, false] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { common::set_owner_validation(enabled) };
        assert_eq!(common::owner_validation(), enabled);
    }
    read_on_threads(common::owner_validation, false);
});

/// A negative timeout is refused, with libgit2's error, and leaves the setting.
fn timeout(
    set: unsafe fn(c_int) -> Result<(), libgit2::libgit2::util::errors::ErrorBuf>,
    get: fn() -> c_int,
    message: &CStr,
) {
    for ms in [0, 1, 75_000, c_int::MAX] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { set(ms) }.unwrap();
        assert_eq!(get(), ms);
    }
    for ms in [-1, c_int::MIN] {
        // SAFETY: as above.
        let e = unsafe { set(ms) }.unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Invalid));
        assert_eq!(e.message(), message);
        assert_eq!(get(), c_int::MAX);
    }
    read_on_threads(get, c_int::MAX);
}

isolated_test!(set_server_connect_timeout, {
    timeout(
        common::set_server_connect_timeout,
        common::server_connect_timeout,
        c"invalid connect timeout",
    );
});

isolated_test!(set_server_timeout, {
    timeout(
        common::set_server_timeout,
        common::server_timeout,
        c"invalid timeout",
    );
});

isolated_test!(set_cache_object_limit, {
    for kind in [
        ObjectType::Commit,
        ObjectType::Tree,
        ObjectType::Blob,
        ObjectType::Tag,
    ] {
        for size in [0, 4096, usize::MAX] {
            // SAFETY: this child runs no other thread while setting.
            unsafe { common::set_cache_object_limit(kind, size) };
        }
    }
});

isolated_test!(set_cache_max_size, {
    for bytes in [0, 1, -1, isize::MIN, isize::MAX, 256 << 20] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { common::set_cache_max_size(bytes) };
        // SAFETY: as above; no repository is open, so no cache updates the size.
        assert_eq!(unsafe { common::cached_memory() }.allowed, bytes);
    }
});

isolated_test!(cached_memory, {
    // SAFETY: this child runs no other thread, and opens no repository.
    let before = unsafe { common::cached_memory() };
    assert_eq!(before.current, 0);
    assert_eq!(before.allowed, 256 << 20);
    let _ = common::extensions().unwrap();
    // SAFETY: as above.
    assert_eq!(unsafe { common::cached_memory() }, before);
});

isolated_test!(enable_caching, {
    switch(common::enable_caching);
});

isolated_test!(enable_strict_object_creation, {
    switch(common::enable_strict_object_creation);
});

isolated_test!(enable_strict_symbolic_ref_creation, {
    switch(common::enable_strict_symbolic_ref_creation);
});

isolated_test!(enable_ofs_delta, {
    switch(common::enable_ofs_delta);
});

isolated_test!(enable_fsync_gitdir, {
    switch(common::enable_fsync_gitdir);
});

isolated_test!(enable_strict_hash_verification, {
    switch(common::enable_strict_hash_verification);
});

isolated_test!(enable_unsaved_index_safety, {
    switch(common::enable_unsaved_index_safety);
});

isolated_test!(disable_pack_keep_file_checks, {
    switch(common::disable_pack_keep_file_checks);
});

isolated_test!(enable_http_expect_continue, {
    switch(common::enable_http_expect_continue);
});

isolated_test!(set_windows_sharemode, {
    for mode in [0, 7, std::ffi::c_ulong::MAX] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { common::set_windows_sharemode(mode) };
        assert_eq!(
            common::windows_sharemode(),
            None,
            "no share mode off Windows"
        );
    }
});

isolated_test!(set_odb_packed_priority, {
    for p in [0, 1, -1, c_int::MIN, c_int::MAX, 1] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { common::set_odb_packed_priority(p) };
    }
});

isolated_test!(set_odb_loose_priority, {
    for p in [0, 1, -1, c_int::MIN, c_int::MAX, 2] {
        // SAFETY: this child runs no other thread while setting.
        unsafe { common::set_odb_loose_priority(p) };
    }
});

/// A self-signed P-256 certificate, for the TLS settings.
const CERT_PEM: &str = include_str!("cert.pem");

/// A directory of this child's own, holding `cert.pem`.
fn cert_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("crustify-ub-unsafe-certs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cert.pem"), CERT_PEM).unwrap();
    dir
}

fn c_path(p: &std::path::Path) -> CString {
    use std::os::unix::ffi::OsStrExt;
    CString::new(p.as_os_str().as_bytes()).unwrap()
}

isolated_test!(set_ssl_cert_locations, {
    let dir = cert_dir();
    let file = c_path(&dir.join("cert.pem"));
    let path = c_path(&dir);
    // SAFETY: this child runs no other thread while setting.
    unsafe {
        common::set_ssl_cert_locations(Some(&file), None).unwrap();
        common::set_ssl_cert_locations(None, Some(&path)).unwrap();
        common::set_ssl_cert_locations(Some(&file), Some(&path)).unwrap();
        let e =
            common::set_ssl_cert_locations(Some(c"/nonexistent/crustify.pem"), None).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Ssl));
        assert!(
            e.message()
                .to_bytes()
                .starts_with(b"OpenSSL error: failed to load certificates")
        );
        let e = common::set_ssl_cert_locations(None, None).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Ssl));
    }
    std::fs::remove_dir_all(dir).unwrap();
});

#[link(name = "crypto")]
unsafe extern "C" {
    fn BIO_new_mem_buf(buf: *const c_void, len: c_int) -> *mut c_void;
    fn BIO_free(bio: *mut c_void) -> c_int;
    fn PEM_read_bio_X509(
        bio: *mut c_void,
        x: *mut *mut c_void,
        cb: *mut c_void,
        u: *mut c_void,
    ) -> *mut c_void;
    fn X509_free(x: *mut c_void);
}

/// [`CERT_PEM`] parsed by libcrypto; the caller frees it with `X509_free`.
fn x509() -> NonNull<c_void> {
    // SAFETY: a read-only memory BIO over the PEM text, which outlives it; the
    // certificate read from it is a fresh object the caller owns.
    unsafe {
        let bio = BIO_new_mem_buf(CERT_PEM.as_ptr().cast(), CERT_PEM.len() as c_int);
        assert!(!bio.is_null());
        let cert = PEM_read_bio_X509(
            bio,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
        BIO_free(bio);
        NonNull::new(cert).unwrap()
    }
}

isolated_test!(
    /// The store takes its own reference: the certificate is freed by the caller right
    /// after, and added twice.
    add_ssl_x509_cert,
    {
        let cert = x509();
        // SAFETY: a live libcrypto `X509`; this child runs no other thread.
        unsafe {
            common::add_ssl_x509_cert(cert).unwrap();
            common::add_ssl_x509_cert(cert).unwrap();
            X509_free(cert.as_ptr());
        }
        let again = x509();
        // SAFETY: as above.
        unsafe {
            common::add_ssl_x509_cert(again).unwrap();
            X509_free(again.as_ptr());
        }
    }
);

static CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    fn malloc(n: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// libc's allocator, counting calls: a family sharing storage with the system
/// allocator it replaces.
struct Counted;

// SAFETY: forwards to libc's `malloc` family unchanged; the counter is atomic.
unsafe impl Alloc for Counted {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        CALLS.fetch_add(1, SeqCst);
        // SAFETY: plain libc allocation.
        NonNull::new(unsafe { malloc(len) }.cast())
    }

    unsafe fn realloc(
        p: Option<NonNull<u8>>,
        len: usize,
        _: Option<&CStr>,
        _: i32,
    ) -> Option<NonNull<u8>> {
        CALLS.fetch_add(1, SeqCst);
        let p = p.map_or(std::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        CALLS.fetch_add(1, SeqCst);
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

isolated_test!(
    /// Installed before libgit2 initialises, the allocator serves initialisation and every
    /// later allocation; restoring the system allocator, which shares its storage, keeps
    /// the blocks it made releasable.
    set_allocator,
    {
        static COUNTED: Allocator = Allocator::new::<Counted>();
        // SAFETY: nothing has initialised libgit2 or allocated through it in this child,
        // and no other thread runs; `Counted` meets `Alloc`'s contract.
        unsafe { common::set_allocator(Some(COUNTED.as_ref())) };
        assert_eq!(CALLS.load(SeqCst), 0, "installing allocates nothing");
        let agent = common::user_agent().unwrap();
        assert!(agent.to_bytes().starts_with(b"libgit2"));
        let served = CALLS.load(SeqCst);
        assert!(served > 0, "initialisation allocated through it");
        let extensions = common::extensions().unwrap();
        assert!(CALLS.load(SeqCst) > served);
        // SAFETY: every live libgit2 block is a libc block, which the system allocator
        // releases; no other thread runs.
        unsafe { common::set_allocator(None) };
        let settled = CALLS.load(SeqCst);
        drop(extensions);
        // SAFETY: as above.
        unsafe { common::set_user_agent(Some(c"after")) }.unwrap();
        assert_eq!(
            CALLS.load(SeqCst),
            settled,
            "the system allocator serves again"
        );
        // SAFETY: as above, the other way round.
        unsafe { common::set_allocator(Some(COUNTED.as_ref())) };
        // SAFETY: as above.
        unsafe { common::set_user_agent(None) }.unwrap();
        assert!(
            CALLS.load(SeqCst) > settled,
            "freed through the counted family"
        );
    }
);

static REFUSE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// libc's allocator, refusing every request while [`REFUSE`] is set: a family sharing
/// storage with the system allocator.
struct Refusable;

// SAFETY: forwards to libc's `malloc` family unchanged, or fails, which `Alloc` allows.
unsafe impl Alloc for Refusable {
    fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
        if REFUSE.load(SeqCst) {
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
        if REFUSE.load(SeqCst) {
            return None;
        }
        let p = p.map_or(std::ptr::null_mut(), |p| p.as_ptr().cast());
        // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
        NonNull::new(unsafe { realloc(p, len) }.cast())
    }

    unsafe fn free(p: NonNull<u8>) {
        // SAFETY: `p` is a live libc block the caller gives up.
        unsafe { free(p.as_ptr().cast()) }
    }
}

static REFUSABLE: Allocator = Allocator::new::<Refusable>();

isolated_test!(
    /// Every option that allocates fails with libgit2's out-of-memory error while nothing
    /// can be allocated, leaking nothing (LSan) and leaving each setting usable: a string
    /// whose copy failed reads as its default.
    allocation_failures_report_errors,
    {
        // SAFETY: nothing has initialised libgit2 in this child; no other thread runs.
        unsafe { common::set_allocator(Some(REFUSABLE.as_ref())) };
        // SAFETY: as above.
        unsafe {
            common::set_user_agent(Some(c"before")).unwrap();
            common::set_user_agent_product(Some(c"before")).unwrap();
            // Allocates the thread's error state, so failures below need no allocation.
            assert!(common::set_server_timeout(-1).is_err());
        }
        let oom = |e: libgit2::libgit2::util::errors::ErrorBuf| {
            assert_eq!(e.class(), Some(ErrorClass::NoMemory), "{e:?}");
        };
        REFUSE.store(true, SeqCst);
        oom(common::search_path(SearchPathLevel::Global).unwrap_err());
        oom(common::template_path().unwrap_err());
        oom(common::homedir().unwrap_err());
        oom(common::user_agent().unwrap_err());
        oom(common::user_agent_product().unwrap_err());
        oom(common::extensions().unwrap_err());
        // SAFETY: as above.
        unsafe {
            oom(common::set_user_agent(Some(c"refused")).unwrap_err());
            oom(common::set_user_agent_product(Some(c"refused")).unwrap_err());
            oom(common::set_ssl_ciphers(c"HIGH").unwrap_err());
            oom(common::set_extensions(&[c"refused"]).unwrap_err());
        }
        REFUSE.store(false, SeqCst);
        assert_eq!(common::user_agent().unwrap(), c"libgit2 1.9.0".to_owned());
        assert_eq!(common::user_agent_product().unwrap(), c"git/2.0".to_owned());
        assert!(
            common::extensions()
                .unwrap()
                .as_ref()
                .iter()
                .all(|e| e != c"refused")
        );
        // SAFETY: as above.
        unsafe { common::set_user_agent(Some(c"after")).unwrap() };
        assert_eq!(common::user_agent().unwrap(), c"after".to_owned());
    }
);

/// A copy into a system directory that fails for want of memory leaves the directory
/// empty and readable, and a later copy fills it again. Without the wrappers' repair
/// (`sysdir_status`), libgit2 would leave it pointing at its one-byte out-of-memory
/// sentinel with the old value's `size`, so the next read would run past the
/// sentinel's end (ASan), and would refuse every later copy.
fn failed_copy_empties(
    set: impl Fn(Option<&CStr>) -> Result<(), ErrorBuf>,
    get: impl Fn() -> Result<CString, ErrorBuf>,
) {
    let oom = |r: Result<(), ErrorBuf>| {
        let e = r.unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::NoMemory), "{e:?}");
    };
    let short = CString::new("/".repeat(64)).unwrap();
    let long = CString::new("/".repeat(4096)).unwrap();
    // The same, expanding the old value: the scratch copy fails instead.
    let long_expanding = CString::new(format!("{}$PATH", "/".repeat(4096))).unwrap();
    // First over the default libgit2 guessed at initialisation, then over values set.
    REFUSE.store(true, SeqCst);
    oom(set(Some(&long)));
    REFUSE.store(false, SeqCst);
    assert_eq!(get().unwrap(), CString::default());
    for failing in [&long, &long_expanding] {
        set(Some(&short)).unwrap();
        assert_eq!(get().unwrap(), short);
        REFUSE.store(true, SeqCst);
        oom(set(Some(failing)));
        assert_eq!(get().unwrap(), CString::default(), "readable while short");
        oom(set(Some(&short)));
        assert_eq!(get().unwrap(), CString::default());
        REFUSE.store(false, SeqCst);
        assert_eq!(
            get().unwrap(),
            CString::default(),
            "empty once memory is back"
        );
        set(Some(&short)).unwrap();
        assert_eq!(get().unwrap(), short, "filled again");
        set(Some(c"/more:$PATH")).unwrap();
        assert_eq!(
            get().unwrap().to_bytes(),
            [b"/more:", short.to_bytes()].concat(),
            "expanded again"
        );
    }
    set(None).unwrap();
}

isolated_test!(
    /// Every system directory setter, after a copy that failed for want of memory.
    failed_sysdir_copies_leave_empty_readable_directories,
    {
        // SAFETY: nothing has initialised libgit2 in this child; no other thread runs.
        unsafe { common::set_allocator(Some(REFUSABLE.as_ref())) };
        // Allocates the thread's error state, so failures below read as out of memory.
        // SAFETY: as above.
        assert!(unsafe { common::set_server_timeout(-1) }.is_err());
        for level in [
            SearchPathLevel::System,
            SearchPathLevel::Xdg,
            SearchPathLevel::Global,
        ] {
            failed_copy_empties(
                // SAFETY: this child runs no other thread while setting.
                |path| unsafe { common::set_search_path(level, path) },
                || common::search_path(level),
            );
        }
        failed_copy_empties(
            // SAFETY: as above.
            |path| unsafe { common::set_template_path(path) },
            common::template_path,
        );
        failed_copy_empties(
            // SAFETY: as above.
            |path| unsafe { common::set_homedir(path) },
            common::homedir,
        );
    }
);

isolated_test!(
    /// When libgit2 cannot be initialised, every option that needs it fails without
    /// calling it, with the error `git_error_last` reports before initialisation.
    without_a_runtime_pinned_options_fail,
    {
        REFUSE.store(true, SeqCst);
        // SAFETY: nothing has initialised libgit2 in this child; no other thread runs.
        unsafe { common::set_allocator(Some(REFUSABLE.as_ref())) };
        let uninit = |e: libgit2::libgit2::util::errors::ErrorBuf| {
            assert_eq!(e.class(), Some(ErrorClass::Invalid), "{e:?}");
            assert_eq!(e.message(), c"library has not been initialized");
        };
        uninit(common::search_path(SearchPathLevel::System).unwrap_err());
        uninit(common::template_path().unwrap_err());
        uninit(common::homedir().unwrap_err());
        uninit(common::user_agent().unwrap_err());
        uninit(common::user_agent_product().unwrap_err());
        uninit(common::extensions().unwrap_err());
        // SAFETY: as above; `cert` is a live libcrypto `X509`, freed after.
        unsafe {
            uninit(common::set_search_path(SearchPathLevel::Xdg, Some(c"/x")).unwrap_err());
            uninit(common::set_template_path(None).unwrap_err());
            uninit(common::set_homedir(None).unwrap_err());
            uninit(common::set_user_agent(None).unwrap_err());
            uninit(common::set_user_agent_product(None).unwrap_err());
            uninit(common::set_ssl_ciphers(c"HIGH").unwrap_err());
            uninit(common::set_ssl_cert_locations(None, None).unwrap_err());
            let cert = x509();
            uninit(common::add_ssl_x509_cert(cert).unwrap_err());
            X509_free(cert.as_ptr());
            uninit(common::set_extensions::<&CStr>(&[]).unwrap_err());
            uninit(common::set_server_timeout(1).unwrap_err());
            uninit(common::set_server_connect_timeout(1).unwrap_err());
        }
        REFUSE.store(false, SeqCst);
        // The plain options need no runtime.
        // SAFETY: as above.
        unsafe { common::set_mwindow_size(1 << 20) };
        assert_eq!(common::mwindow_size(), 1 << 20);
        assert!(
            common::user_agent().is_err(),
            "a refused initialisation is final"
        );
    }
);
