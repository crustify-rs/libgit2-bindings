//! Equivalence of `git2/common.h`'s [`Feature`], [`Features`] and [`BuildInfo`] with
//! the `git_feature_t` and `git_buildinfo_t` values libgit2 declares and accepts.
//!
//! [`Feature`] and [`BuildInfo`] must name exactly the values of their C enums, each
//! with its C value. libgit2 observes them through three routines: the bits
//! `git_libgit2_features` returns must read back unchanged through
//! [`Features::from_bits_retain`], with [`Features::contains`] and [`Features::iter`]
//! agreeing with C's bit tests and no bit left unnamed; `git_libgit2_feature_backend`
//! must answer NULL for every value [`Feature::from_raw`] refuses, a backend only for
//! a feature the set contains, and the same backend on every call; and
//! `git_libgit2_buildinfo` must answer NULL exactly for the values
//! [`BuildInfo::from_raw`] refuses. Every call is repeated and compared with the
//! first answer, since the routines keep no state.
//!
//! None of the three touches libgit2's runtime state (no allocation, no error) for
//! the values probed here, so they run without `git_libgit2_init`. The safe wrappers
//! of the routines themselves belong to the `core` symbol batches.
//!
//! Then [`LibraryOption`], against the `git_libgit2_opt_t` keys libgit2 declares and
//! `git_libgit2_opts` dispatches on.
//!
//! [`LibraryOption`] must name exactly the `git_libgit2_opt_t` constants, each with
//! its C value, and `git_libgit2_opts` must recognise exactly those keys: it refuses
//! every other `int` with `GIT_ERROR_INVALID` ("invalid option key"), reading no
//! argument after it, and handles every named one. Each call's outcome is predicted
//! from [`LibraryOption::from_key`] alone.
//!
//! The named keys are called in a child process, so no other test shares the
//! settings they write, each with the arguments its variant documents, one step
//! after another, comparing after every step. A setter is then paired with the
//! getter its variant names: what one writes, the other must read back.

use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_char, c_int, c_ulong};
use std::process::Command;
use std::ptr;

use libgit2::ffi;
use libgit2::libgit2::core::common::{
    BuildInfo, Feature, Features, LibraryOption, UnnamedBuildInfo, UnnamedFeature,
    UnnamedLibraryOption,
};

/// Every `git_feature_t` constant and the variant carrying it.
const FEATURES: [(ffi::git_feature_t, Feature); 13] = [
    (ffi::GIT_FEATURE_THREADS, Feature::Threads),
    (ffi::GIT_FEATURE_HTTPS, Feature::Https),
    (ffi::GIT_FEATURE_SSH, Feature::Ssh),
    (ffi::GIT_FEATURE_NSEC, Feature::Nsec),
    (ffi::GIT_FEATURE_HTTP_PARSER, Feature::HttpParser),
    (ffi::GIT_FEATURE_REGEX, Feature::Regex),
    (ffi::GIT_FEATURE_I18N, Feature::I18n),
    (ffi::GIT_FEATURE_AUTH_NTLM, Feature::AuthNtlm),
    (ffi::GIT_FEATURE_AUTH_NEGOTIATE, Feature::AuthNegotiate),
    (ffi::GIT_FEATURE_COMPRESSION, Feature::Compression),
    (ffi::GIT_FEATURE_SHA1, Feature::Sha1),
    (ffi::GIT_FEATURE_SHA256, Feature::Sha256),
    (ffi::GIT_FEATURE_HTTP, Feature::Http),
];

/// Every `git_buildinfo_t` constant and the variant carrying it.
const BUILD_INFOS: [(ffi::git_buildinfo_t, BuildInfo); 2] = [
    (ffi::GIT_BUILDINFO_CPU, BuildInfo::Cpu),
    (ffi::GIT_BUILDINFO_COMMIT, BuildInfo::Commit),
];

/// Every value up to past the highest feature bit, each single bit and its
/// neighbours, and a coarse stride across the whole type; BorrowSanitizer
/// instruments each FFI call and Miri interprets each step, so the dense range
/// shrinks under them.
fn probes() -> impl Iterator<Item = u32> {
    let dense = if cfg!(any(bsan, miri)) { 64 } else { 1 << 14 };
    (0..=dense)
        .chain((0..32).flat_map(|k| {
            let bit = 1u32 << k;
            [bit - 1, bit, bit + 1]
        }))
        .chain((0..=u32::MAX).step_by(1 << 26))
        .chain([u32::MAX - 1, u32::MAX])
}

fn c_features() -> c_int {
    // SAFETY: no arguments; returns a constant computed from the build's defines.
    unsafe { ffi::git_libgit2_features() }
}

/// `git_libgit2_feature_backend`'s answer, copied out; `None` for NULL.
fn c_backend(raw: ffi::git_feature_t) -> Option<String> {
    // SAFETY: takes any value by value; returns NULL or a static string. No probed
    // value reaches its assertions, which only an unknown backend of a compiled-in
    // feature does.
    let ptr = unsafe { ffi::git_libgit2_feature_backend(raw) };
    c_string(ptr)
}

/// `git_libgit2_buildinfo`'s answer, copied out; `None` for NULL.
fn c_buildinfo(raw: ffi::git_buildinfo_t) -> Option<String> {
    // SAFETY: takes any value by value; returns NULL or a static string.
    let ptr = unsafe { ffi::git_libgit2_buildinfo(raw) };
    c_string(ptr)
}

fn c_string(ptr: *const c_char) -> Option<String> {
    // SAFETY: both routines return NULL or a NUL-terminated string literal, which
    // lives for the whole program.
    (!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned())
}

#[test]
fn feature_from_raw_names_exactly_the_c_constants() {
    for (c, feature) in FEATURES {
        assert_eq!(feature.raw(), c, "{feature:?}");
        assert_eq!(ffi::git_feature_t::from(feature), c, "{feature:?}");
    }
    assert_eq!(Feature::ALL.map(Feature::raw), FEATURES.map(|(c, _)| c));
    for raw in probes() {
        let c = FEATURES.iter().find(|(c, _)| *c == raw).map(|&(_, f)| f);
        assert_eq!(Feature::from_raw(raw), c, "{raw}");
        assert_eq!(
            Feature::try_from(raw).map_err(UnnamedFeature::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
fn build_info_from_raw_names_exactly_the_c_constants() {
    for (c, info) in BUILD_INFOS {
        assert_eq!(info.raw(), c, "{info:?}");
        assert_eq!(ffi::git_buildinfo_t::from(info), c, "{info:?}");
    }
    for raw in probes() {
        let c = BUILD_INFOS.iter().find(|(c, _)| *c == raw).map(|&(_, i)| i);
        assert_eq!(BuildInfo::from_raw(raw), c, "{raw}");
        assert_eq!(
            BuildInfo::try_from(raw).map_err(UnnamedBuildInfo::raw),
            c.ok_or(raw),
            "{raw}"
        );
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_libgit2_features_reads_back_through_features() {
    let c = c_features();
    let set = Features::from_bits_retain(c);
    assert_eq!(set.bits(), c);
    // The header names every bit this libgit2 sets.
    assert_eq!(set.unnamed_bits(), 0, "{set:?}");
    for (raw, feature) in FEATURES {
        assert_eq!(set.contains(feature), c & raw as c_int != 0, "{feature:?}");
    }
    let named: Vec<_> = FEATURES
        .iter()
        .filter(|&&(raw, _)| c & raw as c_int != 0)
        .map(|&(_, f)| f)
        .collect();
    assert_eq!(set.iter().collect::<Vec<_>>(), named);
    // Rebuilding the set from its features gives C's bits back.
    assert_eq!(set.iter().collect::<Features>().bits(), c);
    // The header's "always available" features.
    for feature in [
        Feature::HttpParser,
        Feature::Regex,
        Feature::Compression,
        Feature::Sha1,
        Feature::Sha256,
    ] {
        assert!(set.contains(feature), "{feature:?} missing from {set:?}");
    }
    // Repeated calls answer the same set.
    for _ in 0..4 {
        assert_eq!(Features::from_bits_retain(c_features()), set);
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_libgit2_feature_backend_answers_only_named_features() {
    let set = Features::from_bits_retain(c_features());
    let mut backed = BTreeSet::new();
    for raw in probes() {
        let first = c_backend(raw);
        match Feature::from_raw(raw) {
            None => assert_eq!(first, None, "{raw}"),
            Some(feature) => {
                if let Some(backend) = &first {
                    assert!(!backend.is_empty(), "{feature:?}");
                    // A backend implies the feature is compiled in.
                    assert!(
                        set.contains(feature),
                        "{feature:?}: {backend} not in {set:?}"
                    );
                    backed.insert(feature);
                }
            }
        }
        assert_eq!(c_backend(raw), first, "{raw}");
    }
    // HTTP has no backend selection; every always-available feature has one.
    assert!(!backed.contains(&Feature::Http));
    for feature in [
        Feature::HttpParser,
        Feature::Regex,
        Feature::Compression,
        Feature::Sha1,
        Feature::Sha256,
    ] {
        assert!(backed.contains(&feature), "{feature:?}");
    }
    // Each single feature of the set, through the Rust value, answers as its raw bit.
    for feature in set {
        assert_eq!(c_backend(feature.raw()), c_backend(feature.raw()));
    }
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn git_libgit2_buildinfo_answers_exactly_the_named_items() {
    for raw in probes() {
        let first = c_buildinfo(raw);
        match BuildInfo::from_raw(raw) {
            None => assert_eq!(first, None, "{raw}"),
            Some(BuildInfo::Cpu) => {
                // cmake always records the processor.
                assert!(first.as_deref().is_some_and(|s| !s.is_empty()), "{first:?}");
            }
            Some(BuildInfo::Commit) => {
                // Recorded when `git rev-parse HEAD` worked at configure time.
                if let Some(commit) = &first {
                    assert!(
                        matches!(commit.len(), 40 | 64)
                            && commit.bytes().all(|b| b.is_ascii_hexdigit()),
                        "{commit}"
                    );
                }
            }
            Some(other) => panic!("{other:?} has no C counterpart"),
        }
        assert_eq!(c_buildinfo(raw), first, "{raw}");
    }
}

/// Every `git_libgit2_opt_t` constant, beside the variant that must carry it.
const OPTIONS: [(ffi::git_libgit2_opt_t, LibraryOption); 48] = [
    (ffi::GIT_OPT_GET_MWINDOW_SIZE, LibraryOption::GetMwindowSize),
    (ffi::GIT_OPT_SET_MWINDOW_SIZE, LibraryOption::SetMwindowSize),
    (
        ffi::GIT_OPT_GET_MWINDOW_MAPPED_LIMIT,
        LibraryOption::GetMwindowMappedLimit,
    ),
    (
        ffi::GIT_OPT_SET_MWINDOW_MAPPED_LIMIT,
        LibraryOption::SetMwindowMappedLimit,
    ),
    (ffi::GIT_OPT_GET_SEARCH_PATH, LibraryOption::GetSearchPath),
    (ffi::GIT_OPT_SET_SEARCH_PATH, LibraryOption::SetSearchPath),
    (
        ffi::GIT_OPT_SET_CACHE_OBJECT_LIMIT,
        LibraryOption::SetCacheObjectLimit,
    ),
    (
        ffi::GIT_OPT_SET_CACHE_MAX_SIZE,
        LibraryOption::SetCacheMaxSize,
    ),
    (ffi::GIT_OPT_ENABLE_CACHING, LibraryOption::EnableCaching),
    (
        ffi::GIT_OPT_GET_CACHED_MEMORY,
        LibraryOption::GetCachedMemory,
    ),
    (
        ffi::GIT_OPT_GET_TEMPLATE_PATH,
        LibraryOption::GetTemplatePath,
    ),
    (
        ffi::GIT_OPT_SET_TEMPLATE_PATH,
        LibraryOption::SetTemplatePath,
    ),
    (
        ffi::GIT_OPT_SET_SSL_CERT_LOCATIONS,
        LibraryOption::SetSslCertLocations,
    ),
    (ffi::GIT_OPT_SET_USER_AGENT, LibraryOption::SetUserAgent),
    (
        ffi::GIT_OPT_ENABLE_STRICT_OBJECT_CREATION,
        LibraryOption::EnableStrictObjectCreation,
    ),
    (
        ffi::GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION,
        LibraryOption::EnableStrictSymbolicRefCreation,
    ),
    (ffi::GIT_OPT_SET_SSL_CIPHERS, LibraryOption::SetSslCiphers),
    (ffi::GIT_OPT_GET_USER_AGENT, LibraryOption::GetUserAgent),
    (ffi::GIT_OPT_ENABLE_OFS_DELTA, LibraryOption::EnableOfsDelta),
    (
        ffi::GIT_OPT_ENABLE_FSYNC_GITDIR,
        LibraryOption::EnableFsyncGitDir,
    ),
    (
        ffi::GIT_OPT_GET_WINDOWS_SHAREMODE,
        LibraryOption::GetWindowsShareMode,
    ),
    (
        ffi::GIT_OPT_SET_WINDOWS_SHAREMODE,
        LibraryOption::SetWindowsShareMode,
    ),
    (
        ffi::GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION,
        LibraryOption::EnableStrictHashVerification,
    ),
    (ffi::GIT_OPT_SET_ALLOCATOR, LibraryOption::SetAllocator),
    (
        ffi::GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY,
        LibraryOption::EnableUnsavedIndexSafety,
    ),
    (
        ffi::GIT_OPT_GET_PACK_MAX_OBJECTS,
        LibraryOption::GetPackMaxObjects,
    ),
    (
        ffi::GIT_OPT_SET_PACK_MAX_OBJECTS,
        LibraryOption::SetPackMaxObjects,
    ),
    (
        ffi::GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS,
        LibraryOption::DisablePackKeepFileChecks,
    ),
    (
        ffi::GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE,
        LibraryOption::EnableHttpExpectContinue,
    ),
    (
        ffi::GIT_OPT_GET_MWINDOW_FILE_LIMIT,
        LibraryOption::GetMwindowFileLimit,
    ),
    (
        ffi::GIT_OPT_SET_MWINDOW_FILE_LIMIT,
        LibraryOption::SetMwindowFileLimit,
    ),
    (
        ffi::GIT_OPT_SET_ODB_PACKED_PRIORITY,
        LibraryOption::SetOdbPackedPriority,
    ),
    (
        ffi::GIT_OPT_SET_ODB_LOOSE_PRIORITY,
        LibraryOption::SetOdbLoosePriority,
    ),
    (ffi::GIT_OPT_GET_EXTENSIONS, LibraryOption::GetExtensions),
    (ffi::GIT_OPT_SET_EXTENSIONS, LibraryOption::SetExtensions),
    (
        ffi::GIT_OPT_GET_OWNER_VALIDATION,
        LibraryOption::GetOwnerValidation,
    ),
    (
        ffi::GIT_OPT_SET_OWNER_VALIDATION,
        LibraryOption::SetOwnerValidation,
    ),
    (ffi::GIT_OPT_GET_HOMEDIR, LibraryOption::GetHomeDir),
    (ffi::GIT_OPT_SET_HOMEDIR, LibraryOption::SetHomeDir),
    (
        ffi::GIT_OPT_SET_SERVER_CONNECT_TIMEOUT,
        LibraryOption::SetServerConnectTimeout,
    ),
    (
        ffi::GIT_OPT_GET_SERVER_CONNECT_TIMEOUT,
        LibraryOption::GetServerConnectTimeout,
    ),
    (
        ffi::GIT_OPT_SET_SERVER_TIMEOUT,
        LibraryOption::SetServerTimeout,
    ),
    (
        ffi::GIT_OPT_GET_SERVER_TIMEOUT,
        LibraryOption::GetServerTimeout,
    ),
    (
        ffi::GIT_OPT_SET_USER_AGENT_PRODUCT,
        LibraryOption::SetUserAgentProduct,
    ),
    (
        ffi::GIT_OPT_GET_USER_AGENT_PRODUCT,
        LibraryOption::GetUserAgentProduct,
    ),
    (
        ffi::GIT_OPT_ADD_SSL_X509_CERT,
        LibraryOption::AddSslX509Cert,
    ),
    (
        ffi::GIT_OPT_GET_PACK_MAX_OBJECT_SIZE,
        LibraryOption::GetPackMaxObjectSize,
    ),
    (
        ffi::GIT_OPT_SET_PACK_MAX_OBJECT_SIZE,
        LibraryOption::SetPackMaxObjectSize,
    ),
];

/// `GIT_CONFIG_LEVEL_SYSTEM`, `_XDG` and `_GLOBAL` from `git2/config.h`, the levels
/// the search path options take.
const CONFIG_LEVELS: [c_int; 3] = [2, 3, 4];
const CONFIG_LEVEL_GLOBAL: c_int = 4;

const CHILD: &str = "CRUSTIFY_EQUIV_CHILD";

/// Runs `body` alone in a child process, so no other test shares libgit2's globals.
fn isolated(test: &str, body: fn()) {
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

/// Initialises libgit2; never released.
fn init() {
    // SAFETY: no arguments; the reference is never released.
    assert!(unsafe { ffi::git_libgit2_init() } >= 1);
}

/// The class and message of the calling thread's last error.
fn c_last() -> (c_int, CString) {
    // SAFETY: the runtime is initialised; the result is a static error or this
    // thread's, live until the next libgit2 call on this thread.
    let e = unsafe { ffi::git_error_last() };
    assert!(!e.is_null());
    // SAFETY: `e` is a live `git_error` whose message is NULL or a C string.
    unsafe {
        let message = (*e).message;
        let message = if message.is_null() {
            c""
        } else {
            CStr::from_ptr(message)
        };
        ((*e).klass, message.to_owned())
    }
}

/// What one `git_libgit2_opts` call reported: its return, and the class and message
/// of the error it left (0 and empty when it left none).
type Reported = (c_int, c_int, CString);

/// Clears the thread's error, runs `call`, and reports it.
fn report(call: impl FnOnce() -> c_int) -> Reported {
    // SAFETY: the runtime is initialised.
    unsafe { ffi::git_error_clear() };
    let rc = call();
    let (class, message) = c_last();
    // A cleared thread reports the static "no error" with class 0.
    let class = if message.as_bytes() == b"no error" {
        0
    } else {
        class
    };
    let message = if class == 0 {
        CString::default()
    } else {
        message
    };
    (rc, class, message)
}

/// What `git_libgit2_opts` reports for a key it does not recognise.
fn refused() -> Reported {
    (
        -1,
        ffi::GIT_ERROR_INVALID as c_int,
        c"invalid option key".to_owned(),
    )
}

#[test]
fn options_carry_their_c_values() {
    for (raw, opt) in OPTIONS {
        assert_eq!(opt.raw(), raw, "{opt:?}");
        assert_eq!(opt.key(), raw as c_int, "{opt:?}");
        assert_eq!(LibraryOption::from_raw(raw), Some(opt));
    }
}

#[test]
fn from_raw_names_exactly_the_c_keys() {
    for raw in (0..=1000).chain([u32::MAX / 2, u32::MAX - 1, u32::MAX]) {
        let c = OPTIONS.iter().find(|(r, _)| *r == raw).map(|&(_, o)| o);
        assert_eq!(LibraryOption::from_raw(raw), c, "{raw}");
        assert_eq!(
            LibraryOption::try_from(raw).map_err(UnnamedLibraryOption::raw),
            c.ok_or(raw),
            "{raw}"
        );
        if let Ok(key) = c_int::try_from(raw) {
            assert_eq!(LibraryOption::from_key(key), c, "{key}");
        }
    }
}

/// Keys `git_libgit2_opts` must refuse: negative ones, and the first values past the
/// header's.
fn unnamed_keys() -> impl Iterator<Item = c_int> {
    [c_int::MIN, -2, -1]
        .into_iter()
        .chain(48..=300)
        .chain([c_int::MAX - 1, c_int::MAX])
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn opts_refuses_exactly_the_unnamed_keys() {
    init();
    for key in unnamed_keys() {
        assert_eq!(LibraryOption::from_key(key), None, "{key}");
        // SAFETY: `git_libgit2_opts` reads no argument after a key it refuses.
        let got = report(|| unsafe { ffi::git_libgit2_opts(key) });
        assert_eq!(got, refused(), "{key}");
    }
}

/// A `git_buf` that libgit2 fills, released on drop.
struct Buf(ffi::git_buf);

impl Buf {
    fn new() -> Self {
        Self(ffi::git_buf::default())
    }

    fn as_mut_ptr(&mut self) -> *mut ffi::git_buf {
        &raw mut self.0
    }

    /// The bytes libgit2 wrote.
    fn bytes(&self) -> Vec<u8> {
        if self.0.ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: libgit2 left `ptr` pointing at `size` initialised bytes.
        unsafe { std::slice::from_raw_parts(self.0.ptr.cast::<u8>(), self.0.size) }.to_vec()
    }
}

impl Drop for Buf {
    fn drop(&mut self) {
        // SAFETY: the buffer is zeroed or as libgit2 filled it, disposed once.
        unsafe { ffi::git_buf_dispose(&raw mut self.0) };
    }
}

/// Calls `opt` with the arguments its variant documents, values that leave the
/// setting as the child found it (or as an earlier step of this child set it).
fn call(opt: LibraryOption) -> Reported {
    use LibraryOption as O;
    let key = opt.key();
    // SAFETY: every arm passes `key` the argument list its variant
    // documents, every pointer live and writable for the call, every string
    // NUL-terminated; the outputs libgit2 fills are released by `Buf` or below.
    report(|| unsafe {
        match opt {
            O::GetMwindowSize
            | O::GetMwindowMappedLimit
            | O::GetMwindowFileLimit
            | O::GetPackMaxObjects
            | O::GetPackMaxObjectSize => {
                let mut out: usize = 0;
                ffi::git_libgit2_opts(key, &raw mut out)
            }
            O::SetMwindowSize
            | O::SetMwindowMappedLimit
            | O::SetMwindowFileLimit
            | O::SetPackMaxObjects
            | O::SetPackMaxObjectSize => {
                let getter = getter_of(opt).unwrap();
                let mut out: usize = 0;
                assert_eq!(ffi::git_libgit2_opts(getter.key(), &raw mut out), 0);
                ffi::git_libgit2_opts(key, out)
            }
            O::GetSearchPath => {
                let mut buf = Buf::new();
                ffi::git_libgit2_opts(key, CONFIG_LEVEL_GLOBAL, buf.as_mut_ptr())
            }
            O::SetSearchPath => {
                ffi::git_libgit2_opts(key, CONFIG_LEVEL_GLOBAL, ptr::null::<c_char>())
            }
            O::SetCacheObjectLimit => ffi::git_libgit2_opts(key, ffi::GIT_OBJECT_BLOB, 0usize),
            O::SetCacheMaxSize => {
                let (mut current, mut allowed): (isize, isize) = (0, 0);
                let get = O::GetCachedMemory.key();
                assert_eq!(
                    ffi::git_libgit2_opts(get, &raw mut current, &raw mut allowed),
                    0
                );
                ffi::git_libgit2_opts(key, allowed)
            }
            O::EnableCaching
            | O::EnableStrictObjectCreation
            | O::EnableStrictSymbolicRefCreation
            | O::EnableOfsDelta
            | O::EnableStrictHashVerification => ffi::git_libgit2_opts(key, 1 as c_int),
            O::EnableFsyncGitDir
            | O::EnableUnsavedIndexSafety
            | O::DisablePackKeepFileChecks
            | O::EnableHttpExpectContinue => ffi::git_libgit2_opts(key, 0 as c_int),
            O::GetCachedMemory => {
                let (mut current, mut allowed): (isize, isize) = (-1, -1);
                let rc = ffi::git_libgit2_opts(key, &raw mut current, &raw mut allowed);
                assert!(current >= 0 && allowed >= 0);
                rc
            }
            O::GetTemplatePath | O::GetUserAgent | O::GetHomeDir | O::GetUserAgentProduct => {
                let mut buf = Buf::new();
                ffi::git_libgit2_opts(key, buf.as_mut_ptr())
            }
            O::SetTemplatePath | O::SetUserAgent | O::SetHomeDir | O::SetUserAgentProduct => {
                ffi::git_libgit2_opts(key, ptr::null::<c_char>())
            }
            O::SetSslCertLocations => ffi::git_libgit2_opts(
                key,
                c"/nonexistent/crustify-ca.pem".as_ptr(),
                ptr::null::<c_char>(),
            ),
            O::SetSslCiphers => ffi::git_libgit2_opts(key, c"DEFAULT".as_ptr()),
            O::GetWindowsShareMode => {
                // Outside Windows libgit2 leaves the value unwritten.
                let mut out: c_ulong = 0x5a5a;
                let rc = ffi::git_libgit2_opts(key, &raw mut out);
                assert_eq!(out, 0x5a5a);
                rc
            }
            O::SetWindowsShareMode => ffi::git_libgit2_opts(key, 0 as c_ulong),
            O::SetAllocator => ffi::git_libgit2_opts(key, ptr::null_mut::<ffi::git_allocator>()),
            O::SetOdbPackedPriority => ffi::git_libgit2_opts(key, 2 as c_int),
            O::SetOdbLoosePriority => ffi::git_libgit2_opts(key, 1 as c_int),
            O::GetExtensions => {
                let mut out = ffi::git_strarray::default();
                let rc = ffi::git_libgit2_opts(key, &raw mut out);
                ffi::git_strarray_dispose(&raw mut out);
                rc
            }
            O::SetExtensions => ffi::git_libgit2_opts(key, ptr::null::<*const c_char>(), 0usize),
            O::GetOwnerValidation | O::GetServerConnectTimeout | O::GetServerTimeout => {
                let mut out: c_int = -1;
                let rc = ffi::git_libgit2_opts(key, &raw mut out);
                assert!(out >= 0);
                rc
            }
            O::SetOwnerValidation | O::SetServerConnectTimeout | O::SetServerTimeout => {
                let getter = getter_of(opt).unwrap();
                let mut out: c_int = -1;
                assert_eq!(ffi::git_libgit2_opts(getter.key(), &raw mut out), 0);
                ffi::git_libgit2_opts(key, out)
            }
            O::AddSslX509Cert => ffi::git_libgit2_opts(key, ptr::null::<u8>()),
            other => panic!("no documented call for {other:?}"),
        }
    })
}

/// What `call(opt)` must report. Every named key is dispatched; only the TLS
/// locations fail, because the file does not exist.
fn expected(opt: LibraryOption) -> Option<(c_int, c_int)> {
    match opt {
        LibraryOption::SetSslCertLocations => Some((-1, ffi::GIT_ERROR_SSL as c_int)),
        _ => None,
    }
}

/// The getter a setter's variant pairs it with.
fn getter_of(opt: LibraryOption) -> Option<LibraryOption> {
    use LibraryOption as O;
    Some(match opt {
        O::SetMwindowSize => O::GetMwindowSize,
        O::SetMwindowMappedLimit => O::GetMwindowMappedLimit,
        O::SetMwindowFileLimit => O::GetMwindowFileLimit,
        O::SetPackMaxObjects => O::GetPackMaxObjects,
        O::SetPackMaxObjectSize => O::GetPackMaxObjectSize,
        O::SetOwnerValidation => O::GetOwnerValidation,
        O::SetServerConnectTimeout => O::GetServerConnectTimeout,
        O::SetServerTimeout => O::GetServerTimeout,
        O::SetSearchPath => O::GetSearchPath,
        O::SetTemplatePath => O::GetTemplatePath,
        O::SetHomeDir => O::GetHomeDir,
        O::SetUserAgent => O::GetUserAgent,
        O::SetUserAgentProduct => O::GetUserAgentProduct,
        O::SetExtensions => O::GetExtensions,
        O::SetCacheMaxSize => O::GetCachedMemory,
        _ => return None,
    })
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn opts_dispatches_every_named_key() {
    isolated(
        concat!(module_path!(), "::opts_dispatches_every_named_key"),
        || {
            init();
            // Every key, named or not, in order: a named key is dispatched, an unnamed
            // one refused, and a refusal leaves the next named key working.
            let keys = (-2..=50).chain(unnamed_keys());
            for key in keys {
                let got = match LibraryOption::from_key(key) {
                    Some(opt) => {
                        let got = call(opt);
                        match expected(opt) {
                            None => assert_eq!(got, (0, 0, CString::default()), "{opt:?}"),
                            Some((rc, class)) => {
                                assert_eq!((got.0, got.1), (rc, class), "{opt:?}: {got:?}")
                            }
                        }
                        got
                    }
                    // SAFETY: `git_libgit2_opts` reads no argument after a key it refuses.
                    None => report(|| unsafe { ffi::git_libgit2_opts(key) }),
                };
                assert_eq!(
                    got == refused(),
                    LibraryOption::from_key(key).is_none(),
                    "{key}: {got:?}"
                );
            }
        },
    );
}

/// Writes `value` through a `size_t` or `int` setter and reads it back through the
/// getter its variant names.
fn set_then_get(set: LibraryOption, value: i64) -> i64 {
    let get = getter_of(set).unwrap();
    // SAFETY: each setter takes the scalar its variant documents and each getter a
    // pointer to one, live and writable for the call.
    unsafe {
        match set {
            LibraryOption::SetOwnerValidation
            | LibraryOption::SetServerConnectTimeout
            | LibraryOption::SetServerTimeout => {
                assert_eq!(ffi::git_libgit2_opts(set.key(), value as c_int), 0);
                let mut out: c_int = -1;
                assert_eq!(ffi::git_libgit2_opts(get.key(), &raw mut out), 0);
                i64::from(out)
            }
            LibraryOption::SetCacheMaxSize => {
                assert_eq!(ffi::git_libgit2_opts(set.key(), value as isize), 0);
                let (mut current, mut allowed): (isize, isize) = (0, 0);
                assert_eq!(
                    ffi::git_libgit2_opts(get.key(), &raw mut current, &raw mut allowed),
                    0
                );
                allowed as i64
            }
            _ => {
                assert_eq!(ffi::git_libgit2_opts(set.key(), value as usize), 0);
                let mut out: usize = 0;
                assert_eq!(ffi::git_libgit2_opts(get.key(), &raw mut out), 0);
                out as i64
            }
        }
    }
}

/// Writes `value` through a string setter and reads it back through the getter its
/// variant names; the search path's at config `level`.
fn set_then_get_str(set: LibraryOption, level: c_int, value: &CStr) -> Vec<u8> {
    let get = getter_of(set).unwrap();
    let mut buf = Buf::new();
    // SAFETY: each setter takes the string (after a config level for the search
    // path) its variant documents, which libgit2 copies; each getter fills `buf`.
    unsafe {
        if set == LibraryOption::SetSearchPath {
            assert_eq!(ffi::git_libgit2_opts(set.key(), level, value.as_ptr()), 0);
            assert_eq!(ffi::git_libgit2_opts(get.key(), level, buf.as_mut_ptr()), 0);
        } else {
            assert_eq!(ffi::git_libgit2_opts(set.key(), value.as_ptr()), 0);
            assert_eq!(ffi::git_libgit2_opts(get.key(), buf.as_mut_ptr()), 0);
        }
    }
    buf.bytes()
}

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn setters_write_what_their_getters_read() {
    isolated(
        concat!(module_path!(), "::setters_write_what_their_getters_read"),
        || {
            init();
            use LibraryOption as O;
            for set in [
                O::SetMwindowSize,
                O::SetMwindowMappedLimit,
                O::SetMwindowFileLimit,
                O::SetPackMaxObjects,
                O::SetPackMaxObjectSize,
                O::SetCacheMaxSize,
            ] {
                for value in [12345, 1, 1 << 40] {
                    assert_eq!(set_then_get(set, value), value, "{set:?}");
                }
            }
            for set in [O::SetServerConnectTimeout, O::SetServerTimeout] {
                for value in [250, 0, c_int::MAX as i64] {
                    assert_eq!(set_then_get(set, value), value, "{set:?}");
                }
            }
            // Owner validation stores a flag.
            for (value, read) in [(0, 0), (7, 1), (1, 1)] {
                assert_eq!(set_then_get(O::SetOwnerValidation, value), read);
            }
            // The user-agent getters read what their setters wrote; the path getters
            // read the directory their setter names.
            let level = CONFIG_LEVEL_GLOBAL;
            for set in [O::SetUserAgent, O::SetUserAgentProduct] {
                assert_eq!(
                    set_then_get_str(set, level, c"crustify/1.0"),
                    b"crustify/1.0"
                );
                assert_eq!(set_then_get_str(set, level, c""), b"");
            }
            for set in [O::SetTemplatePath, O::SetHomeDir] {
                assert_eq!(
                    set_then_get_str(set, level, c"/tmp/crustify-opt"),
                    b"/tmp/crustify-opt",
                    "{set:?}"
                );
            }
            for level in CONFIG_LEVELS {
                let path = CString::new(format!("/tmp/crustify-opt-{level}")).unwrap();
                assert_eq!(
                    set_then_get_str(O::SetSearchPath, level, &path),
                    path.as_bytes(),
                    "{level}"
                );
            }

            // The timeout setters refuse a negative value and keep the old one.
            for (set, message) in [
                (O::SetServerConnectTimeout, c"invalid connect timeout"),
                (O::SetServerTimeout, c"invalid timeout"),
            ] {
                assert_eq!(set_then_get(set, 250), 250);
                // SAFETY: the setter takes the `int` its variant documents.
                let got = report(|| unsafe { ffi::git_libgit2_opts(set.key(), -1 as c_int) });
                assert_eq!(
                    got,
                    (-1, ffi::GIT_ERROR_INVALID as c_int, message.to_owned()),
                    "{set:?}"
                );
                let get = getter_of(set).unwrap();
                let mut out: c_int = -1;
                // SAFETY: the getter takes a pointer to an `int`, live and writable.
                let rc = unsafe { ffi::git_libgit2_opts(get.key(), &raw mut out) };
                assert_eq!((rc, out), (0, 250), "{set:?}");
            }

            // The extension list: what `SetExtensions` adds, `GetExtensions` lists.
            let custom = [c"crustifyext".as_ptr()];
            // SAFETY: `custom` holds one NUL-terminated string, which libgit2 copies.
            let rc = unsafe {
                ffi::git_libgit2_opts(O::SetExtensions.key(), custom.as_ptr(), custom.len())
            };
            assert_eq!(rc, 0);
            let mut out = ffi::git_strarray::default();
            // SAFETY: `out` is a zeroed strarray libgit2 fills, disposed once after the
            // strings are copied.
            let listed = unsafe {
                assert_eq!(
                    ffi::git_libgit2_opts(O::GetExtensions.key(), &raw mut out),
                    0
                );
                let listed: Vec<CString> = (0..out.count)
                    .map(|i| CStr::from_ptr(*out.strings.add(i)).to_owned())
                    .collect();
                ffi::git_strarray_dispose(&raw mut out);
                listed
            };
            assert!(listed.contains(&c"crustifyext".to_owned()), "{listed:?}");
        },
    );
}
