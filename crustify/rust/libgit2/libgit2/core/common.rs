//! Safe wrappers over `include/git2/common.h`.
//!
//! libgit2 reports how it was built through two enums passed by value. A
//! `git_feature_t` names one compile-time feature, a single bit: `git_libgit2_features`
//! returns the features compiled in as an `int` OR-ing those bits ([`Features`]), and
//! `git_libgit2_feature_backend` takes one feature ([`Feature`]) and names the backend
//! providing it. A `git_buildinfo_t` ([`BuildInfo`]) selects the build detail
//! `git_libgit2_buildinfo` reports. Both routines switch on the value and answer NULL
//! for one the header does not name, so a raw value is converted fallibly
//! ([`Feature::from_raw`], [`BuildInfo::from_raw`]) and never transmuted; a feature
//! set keeps the bits it does not name ([`Features::from_bits_retain`]), so it
//! round-trips a later library's answer unchanged.
//!
//! A [`LibraryOption`] is a `git_libgit2_opt_t`, the key `git_libgit2_opts`
//! dispatches on; it refuses any other key, so a raw key is converted fallibly too
//! ([`LibraryOption::from_raw`], [`LibraryOption::from_key`]).
//!
//! [`version`], [`prerelease`] and [`features`] answer from constants compiled into
//! the loaded libgit2.so, so they need no initialisation.
//!
//! # Global options
//!
//! `git_libgit2_opts(key, ...)` sets or queries one of libgit2's global settings per
//! key, a variadic call whose arguments the key decides; each key has a wrapper here.
//! The settings are plain globals that libgit2's other routines read without
//! synchronisation (pack windows, object caches, timeouts, search paths, the
//! `User-Agent`), and the string settings free the string they replace while an HTTP
//! transport may be reading it. So every setter is `unsafe`: no other thread may call
//! into libgit2 while it runs. Nothing but those setters writes the settings, so the
//! getters are safe — except [`cached_memory`], whose read of the cache size races
//! the caches updating it.
//!
//! The keys that allocate or report an error (the strings, the search paths, the
//! extensions, the TLS settings, the timeouts) pin libgit2's runtime first
//! (`runtime_pinned`), which initialises libgit2: before initialisation its allocator
//! fails every request, and its error state has no TLS key. Being pinned for good, the
//! runtime is also never shut down under a string they set — libgit2 frees those
//! strings at its last shutdown without forgetting them, so a later initialisation
//! would read and free them again. The plain numbers and switches need no runtime, and
//! [`set_allocator`] must not initialise one: it is meant to run first. Failures
//! return libgit2's error, copied out, as an [`ErrorBuf`].

use core::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use core::fmt;
use core::ops::{BitOr, BitOrAssign};
use core::ptr::{self, NonNull};
use core::slice;
use std::ffi::CString;

use crate::ffi;
use crate::libgit2::core::strarray::{StrArray, StrArrayVal};
use crate::libgit2::core::types::ObjectType;
use crate::libgit2::util::errors::{ErrorBuf, last};
use crate::libgit2::util::sys::alloc::AllocatorRef;
use crate::libgit2::util::sys::errors::runtime_pinned;

/// One configurable feature of libgit2: an optional setting (threading, say), or a
/// capability that one of several backend providers supplies (HTTPS, through
/// OpenSSL, mbedTLS or a system library). Each is a single bit of a [`Features`] set.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_feature_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum Feature {
    /// libgit2 is thread-aware and can be used from multiple threads.
    Threads = ffi::GIT_FEATURE_THREADS,
    /// HTTPS remotes.
    Https = ffi::GIT_FEATURE_HTTPS,
    /// SSH remotes.
    Ssh = ffi::GIT_FEATURE_SSH,
    /// Sub-second resolution in index timestamps.
    Nsec = ffi::GIT_FEATURE_NSEC,
    /// HTTP parsing; always available.
    HttpParser = ffi::GIT_FEATURE_HTTP_PARSER,
    /// Regular expression support; always available.
    Regex = ffi::GIT_FEATURE_REGEX,
    /// Internationalization support for filename translation.
    I18n = ffi::GIT_FEATURE_I18N,
    /// NTLM support over HTTPS.
    AuthNtlm = ffi::GIT_FEATURE_AUTH_NTLM,
    /// Kerberos (SPNEGO) authentication support over HTTPS.
    AuthNegotiate = ffi::GIT_FEATURE_AUTH_NEGOTIATE,
    /// zlib support; always available.
    Compression = ffi::GIT_FEATURE_COMPRESSION,
    /// SHA1 object support; always available.
    Sha1 = ffi::GIT_FEATURE_SHA1,
    /// SHA256 object support.
    Sha256 = ffi::GIT_FEATURE_SHA256,
    /// HTTP remotes.
    Http = ffi::GIT_FEATURE_HTTP,
}

impl Feature {
    /// Every variant, in ascending bit order.
    pub const ALL: [Feature; 13] = [
        Self::Threads,
        Self::Https,
        Self::Ssh,
        Self::Nsec,
        Self::HttpParser,
        Self::Regex,
        Self::I18n,
        Self::AuthNtlm,
        Self::AuthNegotiate,
        Self::Compression,
        Self::Sha1,
        Self::Sha256,
        Self::Http,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_feature_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_feature_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value, as `git_libgit2_feature_backend` takes it.
    pub const fn raw(self) -> ffi::git_feature_t {
        self as ffi::git_feature_t
    }

    /// The feature's bit in the `int` `git_libgit2_features` returns. Every bit is
    /// below `1 << 13`, so the value is unchanged.
    const fn bit(self) -> c_int {
        self as c_int
    }
}

impl TryFrom<ffi::git_feature_t> for Feature {
    type Error = UnnamedFeature;

    fn try_from(raw: ffi::git_feature_t) -> Result<Self, UnnamedFeature> {
        Self::from_raw(raw).ok_or(UnnamedFeature(raw))
    }
}

impl From<Feature> for ffi::git_feature_t {
    fn from(value: Feature) -> ffi::git_feature_t {
        value.raw()
    }
}

/// A feature value that this version of libgit2's header does not name: `0`, a
/// combination of features, or a later header's bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedFeature(ffi::git_feature_t);

impl UnnamedFeature {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_feature_t {
        self.0
    }
}

impl fmt::Display for UnnamedFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 feature {} is not named by git2/common.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedFeature {}

/// A set of [`Feature`]s: the `int` combination of `GIT_FEATURE_*` bits that
/// `git_libgit2_features` returns.
///
/// It keeps every bit it is given, named or not, so [`bits`](Self::bits) returns
/// what C produced; [`iter`](Self::iter) and [`contains`](Self::contains) see the
/// named features, and [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_feature_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Features(c_int);

impl Features {
    /// The bits of every [`Feature`].
    const NAMED: c_int = {
        let mut bits = 0;
        let mut i = 0;
        while i < Feature::ALL.len() {
            bits |= Feature::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The set with no feature.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as `git_libgit2_features` returns it; bits
    /// that no [`Feature`] names are kept.
    pub const fn from_bits_retain(bits: c_int) -> Self {
        Self(bits)
    }

    /// The bits of the set, as `git_libgit2_features` returns them.
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// The bits of the set that no [`Feature`] names.
    pub const fn unnamed_bits(self) -> c_int {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `feature` is in the set.
    pub const fn contains(self, feature: Feature) -> bool {
        self.0 & feature.bit() != 0
    }

    /// Adds `feature` to the set.
    pub fn insert(&mut self, feature: Feature) {
        self.0 |= feature.bit();
    }

    /// Removes `feature` from the set.
    pub fn remove(&mut self, feature: Feature) {
        self.0 &= !feature.bit();
    }

    /// The named features in the set, in ascending bit order.
    pub fn iter(self) -> FeaturesIter {
        FeaturesIter { set: self, next: 0 }
    }
}

impl From<Feature> for Features {
    fn from(feature: Feature) -> Self {
        Self(feature.bit())
    }
}

impl BitOr for Features {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<Feature> for Features {
    type Output = Self;

    fn bitor(self, rhs: Feature) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for Feature {
    type Output = Features;

    fn bitor(self, rhs: Self) -> Features {
        Features::from(self) | rhs
    }
}

impl BitOrAssign for Features {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<Feature> for Features {
    fn bitor_assign(&mut self, rhs: Feature) {
        self.insert(rhs);
    }
}

impl FromIterator<Feature> for Features {
    fn from_iter<I: IntoIterator<Item = Feature>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<Feature> for Features {
    fn extend<I: IntoIterator<Item = Feature>>(&mut self, iter: I) {
        for feature in iter {
            self.insert(feature);
        }
    }
}

impl IntoIterator for Features {
    type Item = Feature;
    type IntoIter = FeaturesIter;

    fn into_iter(self) -> FeaturesIter {
        self.iter()
    }
}

/// Lists the named features, then any unnamed bits in hex: `{Threads | Sha1, 0x4000}`.
impl fmt::Debug for Features {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Features {")?;
        let mut sep = "";
        for feature in self.iter() {
            write!(f, "{sep}{feature:?}")?;
            sep = " | ";
        }
        if self.unnamed_bits() != 0 {
            let sep = if sep.is_empty() { "" } else { ", " };
            write!(f, "{sep}{:#x}", self.unnamed_bits())?;
        }
        f.write_str("}")
    }
}

/// The named features of a [`Features`] set, in ascending bit order
/// ([`Features::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct FeaturesIter {
    set: Features,
    /// The index into [`Feature::ALL`] of the next feature to test.
    next: usize,
}

impl Iterator for FeaturesIter {
    type Item = Feature;

    fn next(&mut self) -> Option<Feature> {
        while let Some(&feature) = Feature::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(feature) {
                return Some(feature);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(Feature::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for FeaturesIter {}

/// A detail of how libgit2 was built, which `git_libgit2_buildinfo` reports as a
/// static string, or NULL when the build did not record it.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant,
/// `0` included.
///
/// Wraps: git_buildinfo_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum BuildInfo {
    /// The CPU type that libgit2 was built for.
    Cpu = ffi::GIT_BUILDINFO_CPU,
    /// The commit that libgit2 was built from.
    Commit = ffi::GIT_BUILDINFO_COMMIT,
}

impl BuildInfo {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_buildinfo_t) -> Option<Self> {
        match raw {
            ffi::GIT_BUILDINFO_CPU => Some(Self::Cpu),
            ffi::GIT_BUILDINFO_COMMIT => Some(Self::Commit),
            _ => None,
        }
    }

    /// The C value, as `git_libgit2_buildinfo` takes it.
    pub const fn raw(self) -> ffi::git_buildinfo_t {
        self as ffi::git_buildinfo_t
    }
}

impl TryFrom<ffi::git_buildinfo_t> for BuildInfo {
    type Error = UnnamedBuildInfo;

    fn try_from(raw: ffi::git_buildinfo_t) -> Result<Self, UnnamedBuildInfo> {
        Self::from_raw(raw).ok_or(UnnamedBuildInfo(raw))
    }
}

impl From<BuildInfo> for ffi::git_buildinfo_t {
    fn from(value: BuildInfo) -> ffi::git_buildinfo_t {
        value.raw()
    }
}

/// A build-information item that this version of libgit2's header does not name.
/// [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedBuildInfo(ffi::git_buildinfo_t);

impl UnnamedBuildInfo {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_buildinfo_t {
        self.0
    }
}

impl fmt::Display for UnnamedBuildInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 build information item {} is not named by git2/common.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedBuildInfo {}

/// A library-wide setting that `git_libgit2_opts` reads or writes: the key it
/// dispatches on, which also fixes the arguments that follow the key. Each variant
/// gives its C call.
///
/// `git_libgit2_opts` takes the key as an `int` ([`key`](Self::key)) and fails with
/// `GIT_ERROR_INVALID` ("invalid option key") for every value without a variant,
/// reading no further argument; [`from_raw`](Self::from_raw) and
/// [`from_key`](Self::from_key) give `None` for exactly those.
///
/// Wraps: git_libgit2_opt_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum LibraryOption {
    /// `opts(GIT_OPT_GET_MWINDOW_SIZE, size_t *out)`: reads the largest mmap window libgit2
    /// maps for a pack file.
    GetMwindowSize = ffi::GIT_OPT_GET_MWINDOW_SIZE,
    /// `opts(GIT_OPT_SET_MWINDOW_SIZE, size_t size)`: sets the largest mmap window libgit2
    /// maps for a pack file.
    SetMwindowSize = ffi::GIT_OPT_SET_MWINDOW_SIZE,
    /// `opts(GIT_OPT_GET_MWINDOW_MAPPED_LIMIT, size_t *out)`: reads the most memory libgit2
    /// keeps mapped at once, over all files.
    GetMwindowMappedLimit = ffi::GIT_OPT_GET_MWINDOW_MAPPED_LIMIT,
    /// `opts(GIT_OPT_SET_MWINDOW_MAPPED_LIMIT, size_t limit)`: sets the most memory libgit2
    /// keeps mapped at once, over all files.
    SetMwindowMappedLimit = ffi::GIT_OPT_SET_MWINDOW_MAPPED_LIMIT,
    /// `opts(GIT_OPT_GET_SEARCH_PATH, int level, git_buf *out)`: reads the search path of a
    /// config level (`GIT_CONFIG_LEVEL_SYSTEM`, `_GLOBAL` or `_XDG`; any other level is
    /// `GIT_ERROR_INVALID`) into `out`.
    GetSearchPath = ffi::GIT_OPT_GET_SEARCH_PATH,
    /// `opts(GIT_OPT_SET_SEARCH_PATH, int level, const char *path)`: sets the search path
    /// of a config level, which attributes and ignore files share; NULL restores the
    /// default, and `$PATH` in `path` stands for the old value.
    SetSearchPath = ffi::GIT_OPT_SET_SEARCH_PATH,
    /// `opts(GIT_OPT_SET_CACHE_OBJECT_LIMIT, git_object_t type, size_t size)`: sets the
    /// largest object of a type the object cache keeps; 0 keeps none. The type travels as
    /// an `int`.
    SetCacheObjectLimit = ffi::GIT_OPT_SET_CACHE_OBJECT_LIMIT,
    /// `opts(GIT_OPT_SET_CACHE_MAX_SIZE, ssize_t max_storage_bytes)`: sets the soft limit
    /// on the bytes all repositories' object caches hold together.
    SetCacheMaxSize = ffi::GIT_OPT_SET_CACHE_MAX_SIZE,
    /// `opts(GIT_OPT_ENABLE_CACHING, int enabled)`: turns the object cache on or off.
    EnableCaching = ffi::GIT_OPT_ENABLE_CACHING,
    /// `opts(GIT_OPT_GET_CACHED_MEMORY, ssize_t *current, ssize_t *allowed)`: reads the
    /// bytes cached now and the limit on them.
    GetCachedMemory = ffi::GIT_OPT_GET_CACHED_MEMORY,
    /// `opts(GIT_OPT_GET_TEMPLATE_PATH, git_buf *out)`: reads the default template
    /// directory into `out`.
    GetTemplatePath = ffi::GIT_OPT_GET_TEMPLATE_PATH,
    /// `opts(GIT_OPT_SET_TEMPLATE_PATH, const char *path)`: sets the default template
    /// directory.
    SetTemplatePath = ffi::GIT_OPT_SET_TEMPLATE_PATH,
    /// `opts(GIT_OPT_SET_SSL_CERT_LOCATIONS, const char *file, const char *path)`: sets
    /// where TLS looks for certificate authorities: a bundle file, a directory of
    /// certificates, or both; with neither, the TLS backend fails (`GIT_ERROR_SSL`).
    SetSslCertLocations = ffi::GIT_OPT_SET_SSL_CERT_LOCATIONS,
    /// `opts(GIT_OPT_SET_USER_AGENT, const char *user_agent)`: sets the comment section of
    /// the User-Agent header; `""` sends none and NULL restores the default.
    SetUserAgent = ffi::GIT_OPT_SET_USER_AGENT,
    /// `opts(GIT_OPT_ENABLE_STRICT_OBJECT_CREATION, int enabled)`: turns validation of the
    /// objects a new object refers to on or off (on by default).
    EnableStrictObjectCreation = ffi::GIT_OPT_ENABLE_STRICT_OBJECT_CREATION,
    /// `opts(GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION, int enabled)`: turns validation
    /// of a new symbolic reference's target on or off (on by default).
    EnableStrictSymbolicRefCreation = ffi::GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION,
    /// `opts(GIT_OPT_SET_SSL_CIPHERS, const char *ciphers)`: sets the TLS cipher list.
    /// libgit2 copies `ciphers` with `strdup`, so it must not be NULL.
    SetSslCiphers = ffi::GIT_OPT_SET_SSL_CIPHERS,
    /// `opts(GIT_OPT_GET_USER_AGENT, git_buf *out)`: reads the comment section of the User-
    /// Agent header into `out`.
    GetUserAgent = ffi::GIT_OPT_GET_USER_AGENT,
    /// `opts(GIT_OPT_ENABLE_OFS_DELTA, int enabled)`: turns offset deltas in written and
    /// negotiated packfiles on or off (on by default).
    EnableOfsDelta = ffi::GIT_OPT_ENABLE_OFS_DELTA,
    /// `opts(GIT_OPT_ENABLE_FSYNC_GITDIR, int enabled)`: turns `fsync` of the files written
    /// under the git directory on or off (off by default).
    EnableFsyncGitDir = ffi::GIT_OPT_ENABLE_FSYNC_GITDIR,
    /// `opts(GIT_OPT_GET_WINDOWS_SHAREMODE, unsigned long *value)`: reads the share mode
    /// files are opened with on Windows; elsewhere it leaves `value` unwritten and
    /// succeeds.
    GetWindowsShareMode = ffi::GIT_OPT_GET_WINDOWS_SHAREMODE,
    /// `opts(GIT_OPT_SET_WINDOWS_SHAREMODE, unsigned long value)`: sets the share mode
    /// files are opened with on Windows; elsewhere it ignores `value` and succeeds.
    SetWindowsShareMode = ffi::GIT_OPT_SET_WINDOWS_SHAREMODE,
    /// `opts(GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION, int enabled)`: turns verification of
    /// an object's hash on every read on or off (on by default).
    EnableStrictHashVerification = ffi::GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION,
    /// `opts(GIT_OPT_SET_ALLOCATOR, git_allocator *allocator)`: replaces libgit2's
    /// allocator by a copy of `allocator`; NULL restores the system one.
    SetAllocator = ffi::GIT_OPT_SET_ALLOCATOR,
    /// `opts(GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY, int enabled)`: turns the refusal to
    /// reload an index with unsaved changes from disk on or off.
    EnableUnsavedIndexSafety = ffi::GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY,
    /// `opts(GIT_OPT_GET_PACK_MAX_OBJECTS, size_t *out)`: reads the most objects libgit2
    /// accepts in a fetched pack file.
    GetPackMaxObjects = ffi::GIT_OPT_GET_PACK_MAX_OBJECTS,
    /// `opts(GIT_OPT_SET_PACK_MAX_OBJECTS, size_t objects)`: sets the most objects libgit2
    /// accepts in a fetched pack file.
    SetPackMaxObjects = ffi::GIT_OPT_SET_PACK_MAX_OBJECTS,
    /// `opts(GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS, int disabled)`: turns off (or back on)
    /// the check for a pack file's `.keep` file.
    DisablePackKeepFileChecks = ffi::GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS,
    /// `opts(GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE, int enabled)`: turns `Expect:
    /// 100-continue` on NTLM or Negotiate POSTs on or off.
    EnableHttpExpectContinue = ffi::GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE,
    /// `opts(GIT_OPT_GET_MWINDOW_FILE_LIMIT, size_t *out)`: reads the most files libgit2
    /// keeps mapped at once.
    GetMwindowFileLimit = ffi::GIT_OPT_GET_MWINDOW_FILE_LIMIT,
    /// `opts(GIT_OPT_SET_MWINDOW_FILE_LIMIT, size_t limit)`: sets the most files libgit2
    /// keeps mapped at once; 0 (the default) is unlimited.
    SetMwindowFileLimit = ffi::GIT_OPT_SET_MWINDOW_FILE_LIMIT,
    /// `opts(GIT_OPT_SET_ODB_PACKED_PRIORITY, int priority)`: sets the priority of the
    /// packed backend a repository's default object database gets.
    SetOdbPackedPriority = ffi::GIT_OPT_SET_ODB_PACKED_PRIORITY,
    /// `opts(GIT_OPT_SET_ODB_LOOSE_PRIORITY, int priority)`: sets the priority of the loose
    /// backend a repository's default object database gets.
    SetOdbLoosePriority = ffi::GIT_OPT_SET_ODB_LOOSE_PRIORITY,
    /// `opts(GIT_OPT_GET_EXTENSIONS, git_strarray *out)`: reads the repository extensions
    /// libgit2 accepts, built-in and added, without the negated ones, into `out` (released
    /// with `git_strarray_dispose`).
    GetExtensions = ffi::GIT_OPT_GET_EXTENSIONS,
    /// `opts(GIT_OPT_SET_EXTENSIONS, const char **extensions, size_t len)`: declares the
    /// repository extensions the caller supports; a `!` prefix negates a built-in one.
    SetExtensions = ffi::GIT_OPT_SET_EXTENSIONS,
    /// `opts(GIT_OPT_GET_OWNER_VALIDATION, int *enabled)`: reads whether a repository's
    /// directory must belong to the current user.
    GetOwnerValidation = ffi::GIT_OPT_GET_OWNER_VALIDATION,
    /// `opts(GIT_OPT_SET_OWNER_VALIDATION, int enabled)`: sets whether a repository's
    /// directory must belong to the current user (it must by default).
    SetOwnerValidation = ffi::GIT_OPT_SET_OWNER_VALIDATION,
    /// `opts(GIT_OPT_GET_HOMEDIR, git_buf *out)`: reads the home directory libgit2 looks
    /// files up in into `out`.
    GetHomeDir = ffi::GIT_OPT_GET_HOMEDIR,
    /// `opts(GIT_OPT_SET_HOMEDIR, const char *path)`: sets the home directory libgit2 looks
    /// files up in.
    SetHomeDir = ffi::GIT_OPT_SET_HOMEDIR,
    /// `opts(GIT_OPT_SET_SERVER_CONNECT_TIMEOUT, int timeout)`: sets the timeout, in
    /// milliseconds, of a connection to a remote; 0 is the system default and a negative
    /// value is `GIT_ERROR_INVALID`.
    SetServerConnectTimeout = ffi::GIT_OPT_SET_SERVER_CONNECT_TIMEOUT,
    /// `opts(GIT_OPT_GET_SERVER_CONNECT_TIMEOUT, int *timeout)`: reads the timeout, in
    /// milliseconds, of a connection to a remote.
    GetServerConnectTimeout = ffi::GIT_OPT_GET_SERVER_CONNECT_TIMEOUT,
    /// `opts(GIT_OPT_SET_SERVER_TIMEOUT, int timeout)`: sets the timeout, in milliseconds,
    /// of a read from or write to a remote; 0 is the system default and a negative value is
    /// `GIT_ERROR_INVALID`.
    SetServerTimeout = ffi::GIT_OPT_SET_SERVER_TIMEOUT,
    /// `opts(GIT_OPT_GET_SERVER_TIMEOUT, int *timeout)`: reads the timeout, in
    /// milliseconds, of a read from or write to a remote.
    GetServerTimeout = ffi::GIT_OPT_GET_SERVER_TIMEOUT,
    /// `opts(GIT_OPT_SET_USER_AGENT_PRODUCT, const char *user_agent_product)`: sets the
    /// product portion of the User-Agent header (`git/2.0` by default); `""` sends no User-
    /// Agent and NULL restores the default.
    SetUserAgentProduct = ffi::GIT_OPT_SET_USER_AGENT_PRODUCT,
    /// `opts(GIT_OPT_GET_USER_AGENT_PRODUCT, git_buf *out)`: reads the product portion of
    /// the User-Agent header into `out`.
    GetUserAgentProduct = ffi::GIT_OPT_GET_USER_AGENT_PRODUCT,
    /// `opts(GIT_OPT_ADD_SSL_X509_CERT, const X509 *cert)`: adds an OpenSSL certificate to
    /// the TLS certificate store for the rest of the process; NULL adds nothing.
    AddSslX509Cert = ffi::GIT_OPT_ADD_SSL_X509_CERT,
    /// `opts(GIT_OPT_GET_PACK_MAX_OBJECT_SIZE, size_t *out)`: reads the largest object
    /// libgit2 accepts in a fetched pack file.
    GetPackMaxObjectSize = ffi::GIT_OPT_GET_PACK_MAX_OBJECT_SIZE,
    /// `opts(GIT_OPT_SET_PACK_MAX_OBJECT_SIZE, size_t object_size)`: sets the largest
    /// object libgit2 accepts in a fetched pack file (2 GiB by default).
    SetPackMaxObjectSize = ffi::GIT_OPT_SET_PACK_MAX_OBJECT_SIZE,
}

impl LibraryOption {
    /// The variant whose C value is `raw`; `None` for a key this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_libgit2_opt_t) -> Option<Self> {
        match raw {
            ffi::GIT_OPT_GET_MWINDOW_SIZE => Some(Self::GetMwindowSize),
            ffi::GIT_OPT_SET_MWINDOW_SIZE => Some(Self::SetMwindowSize),
            ffi::GIT_OPT_GET_MWINDOW_MAPPED_LIMIT => Some(Self::GetMwindowMappedLimit),
            ffi::GIT_OPT_SET_MWINDOW_MAPPED_LIMIT => Some(Self::SetMwindowMappedLimit),
            ffi::GIT_OPT_GET_SEARCH_PATH => Some(Self::GetSearchPath),
            ffi::GIT_OPT_SET_SEARCH_PATH => Some(Self::SetSearchPath),
            ffi::GIT_OPT_SET_CACHE_OBJECT_LIMIT => Some(Self::SetCacheObjectLimit),
            ffi::GIT_OPT_SET_CACHE_MAX_SIZE => Some(Self::SetCacheMaxSize),
            ffi::GIT_OPT_ENABLE_CACHING => Some(Self::EnableCaching),
            ffi::GIT_OPT_GET_CACHED_MEMORY => Some(Self::GetCachedMemory),
            ffi::GIT_OPT_GET_TEMPLATE_PATH => Some(Self::GetTemplatePath),
            ffi::GIT_OPT_SET_TEMPLATE_PATH => Some(Self::SetTemplatePath),
            ffi::GIT_OPT_SET_SSL_CERT_LOCATIONS => Some(Self::SetSslCertLocations),
            ffi::GIT_OPT_SET_USER_AGENT => Some(Self::SetUserAgent),
            ffi::GIT_OPT_ENABLE_STRICT_OBJECT_CREATION => Some(Self::EnableStrictObjectCreation),
            ffi::GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION => {
                Some(Self::EnableStrictSymbolicRefCreation)
            }
            ffi::GIT_OPT_SET_SSL_CIPHERS => Some(Self::SetSslCiphers),
            ffi::GIT_OPT_GET_USER_AGENT => Some(Self::GetUserAgent),
            ffi::GIT_OPT_ENABLE_OFS_DELTA => Some(Self::EnableOfsDelta),
            ffi::GIT_OPT_ENABLE_FSYNC_GITDIR => Some(Self::EnableFsyncGitDir),
            ffi::GIT_OPT_GET_WINDOWS_SHAREMODE => Some(Self::GetWindowsShareMode),
            ffi::GIT_OPT_SET_WINDOWS_SHAREMODE => Some(Self::SetWindowsShareMode),
            ffi::GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION => {
                Some(Self::EnableStrictHashVerification)
            }
            ffi::GIT_OPT_SET_ALLOCATOR => Some(Self::SetAllocator),
            ffi::GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY => Some(Self::EnableUnsavedIndexSafety),
            ffi::GIT_OPT_GET_PACK_MAX_OBJECTS => Some(Self::GetPackMaxObjects),
            ffi::GIT_OPT_SET_PACK_MAX_OBJECTS => Some(Self::SetPackMaxObjects),
            ffi::GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS => Some(Self::DisablePackKeepFileChecks),
            ffi::GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE => Some(Self::EnableHttpExpectContinue),
            ffi::GIT_OPT_GET_MWINDOW_FILE_LIMIT => Some(Self::GetMwindowFileLimit),
            ffi::GIT_OPT_SET_MWINDOW_FILE_LIMIT => Some(Self::SetMwindowFileLimit),
            ffi::GIT_OPT_SET_ODB_PACKED_PRIORITY => Some(Self::SetOdbPackedPriority),
            ffi::GIT_OPT_SET_ODB_LOOSE_PRIORITY => Some(Self::SetOdbLoosePriority),
            ffi::GIT_OPT_GET_EXTENSIONS => Some(Self::GetExtensions),
            ffi::GIT_OPT_SET_EXTENSIONS => Some(Self::SetExtensions),
            ffi::GIT_OPT_GET_OWNER_VALIDATION => Some(Self::GetOwnerValidation),
            ffi::GIT_OPT_SET_OWNER_VALIDATION => Some(Self::SetOwnerValidation),
            ffi::GIT_OPT_GET_HOMEDIR => Some(Self::GetHomeDir),
            ffi::GIT_OPT_SET_HOMEDIR => Some(Self::SetHomeDir),
            ffi::GIT_OPT_SET_SERVER_CONNECT_TIMEOUT => Some(Self::SetServerConnectTimeout),
            ffi::GIT_OPT_GET_SERVER_CONNECT_TIMEOUT => Some(Self::GetServerConnectTimeout),
            ffi::GIT_OPT_SET_SERVER_TIMEOUT => Some(Self::SetServerTimeout),
            ffi::GIT_OPT_GET_SERVER_TIMEOUT => Some(Self::GetServerTimeout),
            ffi::GIT_OPT_SET_USER_AGENT_PRODUCT => Some(Self::SetUserAgentProduct),
            ffi::GIT_OPT_GET_USER_AGENT_PRODUCT => Some(Self::GetUserAgentProduct),
            ffi::GIT_OPT_ADD_SSL_X509_CERT => Some(Self::AddSslX509Cert),
            ffi::GIT_OPT_GET_PACK_MAX_OBJECT_SIZE => Some(Self::GetPackMaxObjectSize),
            ffi::GIT_OPT_SET_PACK_MAX_OBJECT_SIZE => Some(Self::SetPackMaxObjectSize),
            _ => None,
        }
    }

    /// The variant whose key, as `git_libgit2_opts` takes it, is `key`; `None` for a
    /// key it refuses.
    pub const fn from_key(key: c_int) -> Option<Self> {
        if key < 0 {
            return None;
        }
        Self::from_raw(key as ffi::git_libgit2_opt_t)
    }

    /// The C value, as the header declares it.
    pub const fn raw(self) -> ffi::git_libgit2_opt_t {
        self as ffi::git_libgit2_opt_t
    }

    /// The key as `git_libgit2_opts` takes it, an `int`; every variant's value fits.
    pub const fn key(self) -> c_int {
        self as c_int
    }
}

impl TryFrom<ffi::git_libgit2_opt_t> for LibraryOption {
    type Error = UnnamedLibraryOption;

    fn try_from(raw: ffi::git_libgit2_opt_t) -> Result<Self, UnnamedLibraryOption> {
        Self::from_raw(raw).ok_or(UnnamedLibraryOption(raw))
    }
}

impl From<LibraryOption> for ffi::git_libgit2_opt_t {
    fn from(value: LibraryOption) -> ffi::git_libgit2_opt_t {
        value.raw()
    }
}

/// An option key that this version of libgit2's header does not name, which
/// `git_libgit2_opts` refuses; [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedLibraryOption(ffi::git_libgit2_opt_t);

impl UnnamedLibraryOption {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_libgit2_opt_t {
        self.0
    }
}

impl fmt::Display for UnnamedLibraryOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 option key {} is not named by git2/common.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedLibraryOption {}

// --- Version -------------------------------------------------------------------------

/// The version of the libgit2 library in use: `major.minor.revision`, as
/// `git_libgit2_version` reports it.
///
/// It is the version of the loaded libgit2.so, which may differ from the headers this
/// crate was generated from (`ffi::LIBGIT2_VERSION_*`). It displays as
/// `major.minor.revision`; [`prerelease`] has the rest of the version.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// The major version number.
    pub major: c_int,
    /// The minor version number.
    pub minor: c_int,
    /// The revision (patch) number.
    pub revision: c_int,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.revision)
    }
}

/// The version of the libgit2 library in use.
///
/// libgit2 answers from constants compiled into it, so this needs no initialisation
/// and cannot fail.
///
/// Wraps: git_libgit2_version
pub fn version() -> Version {
    let (mut major, mut minor, mut revision) = (0, 0, 0);
    // SAFETY: three out-parameters, each a live `int` local, written once and not
    // retained; the routine reads no other state and always returns 0.
    unsafe { ffi::git_libgit2_version(&raw mut major, &raw mut minor, &raw mut revision) };
    Version {
        major,
        minor,
        revision,
    }
}

/// The prerelease state of the libgit2 library in use: `"alpha"` for nightly builds,
/// `"beta"` or a release candidate (`"rc1"`, ...) for prereleases, and `None` for a
/// final release.
///
/// libgit2 returns a string literal of its own, which lives as long as libgit2.so
/// stays loaded: this crate links it for the life of the process. Needs no
/// initialisation.
///
/// Wraps: git_libgit2_prerelease
pub fn prerelease() -> Option<&'static CStr> {
    // SAFETY: no arguments, no state read.
    let s = unsafe { ffi::git_libgit2_prerelease() };
    // SAFETY: a non-NULL result is `LIBGIT2_VERSION_PRERELEASE`, a NUL-terminated
    // literal in libgit2.so's read-only data, never written or freed while the library
    // is loaded, which it is for the rest of the process.
    (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
}

/// The features compiled into the libgit2 library in use.
///
/// The set keeps any bit a later libgit2 adds that this crate's [`Feature`] does not
/// name. Needs no initialisation.
///
/// Wraps: git_libgit2_features
pub fn features() -> Features {
    // SAFETY: no arguments; the routine ORs compile-time constants.
    Features::from_bits_retain(unsafe { ffi::git_libgit2_features() })
}

// --- Options -------------------------------------------------------------------------
//
// `git_libgit2_opts(key, ...)` is one variadic entry point per setting; each wrapper
// below is one key, with the argument types that key reads.

/// `git_libgit2_opts` with a key whose only argument is a `size_t *` it writes.
///
/// # Safety
///
/// `key` must read exactly one `size_t *` and write a `size_t` through it.
unsafe fn get_size(key: LibraryOption) -> usize {
    let mut value = 0usize;
    // SAFETY: per this function's contract, `key` reads one `size_t *`, a live local
    // here, writes through it and keeps no pointer to it.
    let rc = unsafe { ffi::git_libgit2_opts(key.key(), &raw mut value) };
    debug_assert_eq!(rc, 0, "{key:?} cannot fail");
    value
}

/// `git_libgit2_opts` with a key whose only argument is an `int *` it writes.
///
/// # Safety
///
/// `key` must read exactly one `int *` and write an `int` through it.
unsafe fn get_int(key: LibraryOption) -> c_int {
    let mut value: c_int = 0;
    // SAFETY: per this function's contract, `key` reads one `int *`, a live local
    // here, writes through it and keeps no pointer to it.
    let rc = unsafe { ffi::git_libgit2_opts(key.key(), &raw mut value) };
    debug_assert_eq!(rc, 0, "{key:?} cannot fail");
    value
}

/// `git_libgit2_opts` with a key whose only argument is a `size_t` it stores.
///
/// # Safety
///
/// `key` must read exactly one `size_t` and store it in a global that no other thread
/// accesses while this runs.
unsafe fn set_size(key: LibraryOption, value: usize) {
    // SAFETY: per this function's contract, `key` reads one `size_t` and the global
    // it writes is not accessed concurrently.
    let rc = unsafe { ffi::git_libgit2_opts(key.key(), value) };
    debug_assert_eq!(rc, 0, "{key:?} cannot fail");
}

/// `git_libgit2_opts` with a key whose only argument is an `int` it stores.
///
/// # Safety
///
/// `key` must read exactly one `int` and store it (or whether it is non-zero) in a
/// global that no other thread accesses while this runs, without failing.
unsafe fn set_int(key: LibraryOption, value: c_int) {
    // SAFETY: per this function's contract, `key` reads one `int` and the global it
    // writes is not accessed concurrently.
    let rc = unsafe { ffi::git_libgit2_opts(key.key(), value) };
    debug_assert_eq!(rc, 0, "{key:?} cannot fail");
}

/// A status `git_libgit2_opts` returned: `Ok` for 0, the thread's libgit2 error for a
/// negative one. Only for keys that run with the runtime pinned.
fn status(rc: c_int) -> Result<(), ErrorBuf> {
    if rc < 0 { Err(last()) } else { Ok(()) }
}

/// Runs `fill` on a fresh, zeroed `git_buf` and returns what it left there, copied
/// out, disposing the buffer either way; `fill`'s failure is the thread's libgit2
/// error. Pins the runtime first.
///
/// `fill` hands the buffer to a `git_libgit2_opts` key that writes a `git_buf *`.
fn buf_string(fill: impl FnOnce(*mut ffi::git_buf) -> c_int) -> Result<CString, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut buf = ffi::git_buf {
        ptr: ptr::null_mut(),
        reserved: 0,
        size: 0,
    };
    let rc = fill(&raw mut buf);
    let out = if rc < 0 {
        Err(last())
    } else if buf.ptr.is_null() {
        Ok(CString::default())
    } else {
        // SAFETY: libgit2 filled the buffer (`git_buf_fromstr`): `ptr` addresses `size`
        // bytes followed by a NUL, in its allocator or in its static empty string, and
        // stays valid until the dispose below.
        let bytes = unsafe { slice::from_raw_parts(buf.ptr.cast::<u8>(), buf.size + 1) };
        Ok(CStr::from_bytes_until_nul(bytes).map_or_else(|_| CString::default(), CStr::to_owned))
    };
    // SAFETY: `buf` is zeroed or as libgit2 left it, so it owns `ptr` exactly when
    // `reserved > 0`, which is what `git_buf_dispose` frees; the runtime is pinned,
    // so `git__free` reads an initialised allocator. Nothing reads `buf` afterwards.
    unsafe { ffi::git_buf_dispose(&raw mut buf) };
    out
}

/// A configuration level whose search path libgit2 keeps ([`search_path`],
/// [`set_search_path`]): the three of `git_config_level_t` that name a file outside
/// any repository.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum SearchPathLevel {
    /// The system-wide configuration, typically `/etc/gitconfig`
    /// (`GIT_CONFIG_LEVEL_SYSTEM`).
    System = ffi::GIT_CONFIG_LEVEL_SYSTEM,
    /// The XDG configuration, typically `~/.config/git/config`
    /// (`GIT_CONFIG_LEVEL_XDG`).
    Xdg = ffi::GIT_CONFIG_LEVEL_XDG,
    /// The user's global configuration, typically `~/.gitconfig`
    /// (`GIT_CONFIG_LEVEL_GLOBAL`).
    Global = ffi::GIT_CONFIG_LEVEL_GLOBAL,
}

impl SearchPathLevel {
    /// The `git_config_level_t` value, as `git_libgit2_opts` reads it.
    pub const fn raw(self) -> ffi::git_config_level_t {
        self as ffi::git_config_level_t
    }
}

/// The current and the allowed number of bytes in libgit2's object caches, across
/// every repository ([`cached_memory`]).
///
/// Handwritten
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct CachedMemory {
    /// Bytes held in the caches now.
    pub current: isize,
    /// Bytes the caches may hold before libgit2 starts evicting
    /// ([`set_cache_max_size`]).
    pub allowed: isize,
}

/// The maximum size of an mmap window into a pack file, in bytes.
///
/// Wraps: git_libgit2_opts
pub fn mwindow_size() -> usize {
    // SAFETY: `GIT_OPT_GET_MWINDOW_SIZE` writes one `size_t` through one `size_t *`.
    // Its global is written only by `set_mwindow_size`, whose caller excludes this.
    unsafe { get_size(LibraryOption::GetMwindowSize) }
}

/// Sets the maximum size of an mmap window into a pack file, in bytes.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_mwindow_size(size: usize) {
    // SAFETY: `GIT_OPT_SET_MWINDOW_SIZE` reads one `size_t`; the caller excludes
    // concurrent access to its global.
    unsafe { set_size(LibraryOption::SetMwindowSize, size) }
}

/// The maximum number of bytes libgit2 maps from pack files at any time, in total.
///
/// Wraps: git_libgit2_opts
pub fn mwindow_mapped_limit() -> usize {
    // SAFETY: as in `mwindow_size`, for `GIT_OPT_GET_MWINDOW_MAPPED_LIMIT`.
    unsafe { get_size(LibraryOption::GetMwindowMappedLimit) }
}

/// Sets the maximum number of bytes libgit2 maps from pack files at any time.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_mwindow_mapped_limit(limit: usize) {
    // SAFETY: as in `set_mwindow_size`, for `GIT_OPT_SET_MWINDOW_MAPPED_LIMIT`.
    unsafe { set_size(LibraryOption::SetMwindowMappedLimit, limit) }
}

/// The maximum number of pack files libgit2 maps at any time; 0 is unlimited.
///
/// Wraps: git_libgit2_opts
pub fn mwindow_file_limit() -> usize {
    // SAFETY: as in `mwindow_size`, for `GIT_OPT_GET_MWINDOW_FILE_LIMIT`.
    unsafe { get_size(LibraryOption::GetMwindowFileLimit) }
}

/// Sets the maximum number of pack files libgit2 maps at any time; 0 (the default) is
/// unlimited.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_mwindow_file_limit(limit: usize) {
    // SAFETY: as in `set_mwindow_size`, for `GIT_OPT_SET_MWINDOW_FILE_LIMIT`.
    unsafe { set_size(LibraryOption::SetMwindowFileLimit, limit) }
}

/// The search path of configuration `level`: directories separated by `:`
/// (`GIT_PATH_LIST_SEPARATOR`), where libgit2 looks for that level's configuration,
/// attributes and ignore files.
///
/// Pins the runtime first: libgit2 computes the defaults from the environment when it
/// initialises. Fails, with libgit2's error, when the copy cannot be allocated.
///
/// Wraps: git_libgit2_opts
pub fn search_path(level: SearchPathLevel) -> Result<CString, ErrorBuf> {
    buf_string(|buf| {
        // SAFETY: `GIT_OPT_GET_SEARCH_PATH` reads an `int` level, a named one, and a
        // `git_buf *`, the zeroed local `buf_string` passes, which it fills and does
        // not retain. The runtime is pinned, so the search paths exist; they are
        // written only by `set_search_path`, whose caller excludes this.
        unsafe { ffi::git_libgit2_opts(LibraryOption::GetSearchPath.key(), level.raw(), buf) }
    })
}

/// Sets the search path of configuration `level`, or with `None` resets it to the
/// default libgit2 derives from the environment.
///
/// `path` lists directories separated by `:` (`GIT_PATH_LIST_SEPARATOR`); a `$PATH` in
/// it stands for the previous value, so `"/opt/git:$PATH"` prepends. libgit2 copies
/// it. Pins the runtime first. Fails, with libgit2's error, when the copy cannot be
/// allocated.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_search_path(level: SearchPathLevel, path: Option<&CStr>) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let path = path.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `GIT_OPT_SET_SEARCH_PATH` reads an `int` level, a named one, and a
    // `const char *`, NULL or NUL-terminated, which it copies and does not retain. The
    // runtime is pinned; the caller excludes concurrent access to the search paths.
    status(unsafe { ffi::git_libgit2_opts(LibraryOption::SetSearchPath.key(), level.raw(), path) })
}

/// Sets the largest object of type `kind` that libgit2 caches in memory, in bytes; 0
/// caches none. The defaults are 0 for blobs (never cached) and 4 KiB for commits,
/// trees and tags.
///
/// libgit2 refuses a type outside its cache's table, which no [`ObjectType`] is.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_cache_object_limit(kind: ObjectType, size: usize) {
    // SAFETY: `GIT_OPT_SET_CACHE_OBJECT_LIMIT` reads an `int` object type, a named one
    // that indexes its table in bounds, so it neither fails nor sets an error, and a
    // `size_t`. The caller excludes concurrent access to the limits.
    let rc = unsafe {
        ffi::git_libgit2_opts(LibraryOption::SetCacheObjectLimit.key(), kind.raw(), size)
    };
    debug_assert_eq!(rc, 0);
}

/// Sets how many bytes libgit2's object caches may hold, across every repository,
/// before libgit2 starts evicting: a soft limit, 256 MiB by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_cache_max_size(max_storage_bytes: isize) {
    // SAFETY: `GIT_OPT_SET_CACHE_MAX_SIZE` reads one `ssize_t`; the caller excludes
    // concurrent access to its global.
    let rc =
        unsafe { ffi::git_libgit2_opts(LibraryOption::SetCacheMaxSize.key(), max_storage_bytes) };
    debug_assert_eq!(rc, 0);
}

/// Enables or disables libgit2's object caches. Caches belong to repositories, so
/// disabling empties each one only the next time it would be updated.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_caching(enabled: bool) {
    // SAFETY: `GIT_OPT_ENABLE_CACHING` reads one `int`; the caller excludes concurrent
    // access to its global.
    unsafe { set_int(LibraryOption::EnableCaching, c_int::from(enabled)) }
}

/// How many bytes libgit2's object caches hold now, and may hold.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 reads the current size with a plain load, while every
/// repository's cache updates it atomically as it adds and evicts objects.
///
/// Wraps: git_libgit2_opts
pub unsafe fn cached_memory() -> CachedMemory {
    let mut memory = CachedMemory::default();
    // SAFETY: `GIT_OPT_GET_CACHED_MEMORY` writes one `ssize_t` through each of two
    // `ssize_t *`, fields of a live local, and keeps neither; the caller excludes the
    // concurrent cache updates its plain read would race.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            LibraryOption::GetCachedMemory.key(),
            &raw mut memory.current,
            &raw mut memory.allowed,
        )
    };
    debug_assert_eq!(rc, 0);
    memory
}

/// The default template directory, which repository initialisation copies from.
///
/// Pins the runtime first. Fails, with libgit2's error, when the copy cannot be
/// allocated.
///
/// Wraps: git_libgit2_opts
pub fn template_path() -> Result<CString, ErrorBuf> {
    buf_string(|buf| {
        // SAFETY: `GIT_OPT_GET_TEMPLATE_PATH` reads a `git_buf *`, the zeroed local
        // `buf_string` passes, which it fills and does not retain. The runtime is
        // pinned; the path is written only by `set_template_path`, whose caller
        // excludes this.
        unsafe { ffi::git_libgit2_opts(LibraryOption::GetTemplatePath.key(), buf) }
    })
}

/// Sets the default template directory, or with `None` resets it to libgit2's
/// default.
///
/// libgit2 copies `path`. Pins the runtime first. Fails, with libgit2's error, when
/// the copy cannot be allocated.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_template_path(path: Option<&CStr>) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let path = path.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `GIT_OPT_SET_TEMPLATE_PATH` reads one `const char *`, NULL or
    // NUL-terminated, which it copies and does not retain. The runtime is pinned; the
    // caller excludes concurrent access to the path.
    status(unsafe { ffi::git_libgit2_opts(LibraryOption::SetTemplatePath.key(), path) })
}

/// Sets where libgit2's TLS backend looks for certificate authorities: `file`, one
/// file of concatenated certificates, and `path`, a directory of certificates one per
/// file.
///
/// Either may be `None`; with both, OpenSSL loads nothing and the call fails. Pins the
/// runtime first. Fails, with libgit2's error (`GIT_ERROR_SSL`), when OpenSSL cannot
/// load them.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 loads them into the one OpenSSL context every HTTPS connection
/// is created from, which OpenSSL does not synchronise.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_ssl_cert_locations(
    file: Option<&CStr>,
    path: Option<&CStr>,
) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let file = file.map_or(ptr::null(), CStr::as_ptr);
    let path = path.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `GIT_OPT_SET_SSL_CERT_LOCATIONS` reads two `const char *`, each NULL or
    // NUL-terminated, which OpenSSL reads during the call. The runtime is pinned, so
    // the OpenSSL context exists; the caller excludes concurrent use of it.
    let rc = unsafe { ffi::git_libgit2_opts(LibraryOption::SetSslCertLocations.key(), file, path) };
    status(rc)
}

/// Adds the OpenSSL certificate `cert` to the certificate store of libgit2's TLS
/// backend, for the rest of the process.
///
/// libgit2 takes a reference of its own (`X509_STORE_add_cert` up-refs it), so the
/// caller keeps its own. Pins the runtime first. Fails, with libgit2's error
/// (`GIT_ERROR_SSL`), when OpenSSL cannot add it.
///
/// Raw seam: libgit2 publishes no type for the certificate, OpenSSL's `X509`, and
/// this crate binds no OpenSSL.
///
/// # Safety
///
/// `cert` must point to a live `X509` of the libcrypto libgit2 is linked against,
/// valid for the call.
///
/// Wraps: git_libgit2_opts
pub unsafe fn add_ssl_x509_cert(cert: NonNull<c_void>) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    // SAFETY: `GIT_OPT_ADD_SSL_X509_CERT` reads one `X509 *`, a live certificate per
    // this function's contract, which OpenSSL up-refs into the store under the
    // store's own lock. The runtime is pinned, so the OpenSSL context exists.
    let rc = unsafe { ffi::git_libgit2_opts(LibraryOption::AddSslX509Cert.key(), cert.as_ptr()) };
    status(rc)
}

/// The comment part of the `User-Agent` header libgit2 sends: `"libgit2 1.9.0"` (the
/// version) by default.
///
/// Pins the runtime first. Fails, with libgit2's error, when the copy cannot be
/// allocated.
///
/// Wraps: git_libgit2_opts
pub fn user_agent() -> Result<CString, ErrorBuf> {
    buf_string(|buf| {
        // SAFETY: `GIT_OPT_GET_USER_AGENT` reads a `git_buf *`, the zeroed local
        // `buf_string` passes, which it fills and does not retain. The runtime is
        // pinned; the string is replaced only by `set_user_agent`, whose caller
        // excludes this.
        unsafe { ffi::git_libgit2_opts(LibraryOption::GetUserAgent.key(), buf) }
    })
}

/// Sets the comment part of the `User-Agent` header, appended to its product part
/// ([`set_user_agent_product`]); `Some(c"")` sends no comment, and `None` restores the
/// default.
///
/// libgit2 copies `user_agent`. Pins the runtime first. Fails, with libgit2's error,
/// when the copy cannot be allocated; the default is then in effect.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 frees the previous string, which its HTTP transport reads
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_user_agent(user_agent: Option<&CStr>) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let user_agent = user_agent.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: `GIT_OPT_SET_USER_AGENT` reads one `const char *`, NULL or
    // NUL-terminated, which it copies and does not retain. The runtime is pinned, for
    // good, so the copy is never freed by a shutdown it would outlive; the caller
    // excludes concurrent readers of the string it frees.
    status(unsafe { ffi::git_libgit2_opts(LibraryOption::SetUserAgent.key(), user_agent) })
}

/// The product part of the `User-Agent` header libgit2 sends: `"git/2.0"` by default.
///
/// Pins the runtime first. Fails, with libgit2's error, when the copy cannot be
/// allocated.
///
/// Wraps: git_libgit2_opts
pub fn user_agent_product() -> Result<CString, ErrorBuf> {
    buf_string(|buf| {
        // SAFETY: as in `user_agent`, for `GIT_OPT_GET_USER_AGENT_PRODUCT`, whose string
        // is replaced only by `set_user_agent_product`.
        unsafe { ffi::git_libgit2_opts(LibraryOption::GetUserAgentProduct.key(), buf) }
    })
}

/// Sets the product part of the `User-Agent` header; `Some(c"")` sends no
/// `User-Agent` at all, and `None` restores `"git/2.0"`. Servers detect clients by it,
/// so keep it `git/<version>`.
///
/// libgit2 copies `product`. Pins the runtime first. Fails, with libgit2's error, when
/// the copy cannot be allocated; the default is then in effect.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 frees the previous string, which its HTTP transport reads
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_user_agent_product(product: Option<&CStr>) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let product = product.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: as in `set_user_agent`, for `GIT_OPT_SET_USER_AGENT_PRODUCT`.
    let rc = unsafe { ffi::git_libgit2_opts(LibraryOption::SetUserAgentProduct.key(), product) };
    status(rc)
}

/// Enables or disables validating the inputs of new objects (that a new commit's tree
/// and parents exist, say). Enabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_strict_object_creation(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_STRICT_OBJECT_CREATION`.
    unsafe {
        set_int(
            LibraryOption::EnableStrictObjectCreation,
            c_int::from(enabled),
        )
    }
}

/// Enables or disables validating the target of a new symbolic reference, so that
/// `foobar`, not a valid reference name, is refused where `refs/heads/foobar` is
/// accepted. Enabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_strict_symbolic_ref_creation(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_STRICT_SYMBOLIC_REF_CREATION`.
    unsafe {
        set_int(
            LibraryOption::EnableStrictSymbolicRefCreation,
            c_int::from(enabled),
        )
    }
}

/// Sets the cipher list libgit2's TLS backend offers on HTTPS connections, in
/// OpenSSL's syntax.
///
/// libgit2 copies `ciphers`, and applies it when it sets OpenSSL up: when it
/// initialises, before this can run with OpenSSL linked in, or on first use with
/// OpenSSL loaded dynamically. Pins the runtime first. Fails, with libgit2's error,
/// when the copy cannot be allocated.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 frees the previous string, which it reads without
/// synchronisation when it sets OpenSSL up.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_ssl_ciphers(ciphers: &CStr) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    // SAFETY: `GIT_OPT_SET_SSL_CIPHERS` reads one `const char *`, NUL-terminated and
    // never NULL here (libgit2 would `strlen` it), which it copies and does not
    // retain. The runtime is pinned, for good, so the copy is never freed by a
    // shutdown it would outlive; the caller excludes concurrent readers.
    status(unsafe { ffi::git_libgit2_opts(LibraryOption::SetSslCiphers.key(), ciphers.as_ptr()) })
}

/// Enables or disables offset deltas when creating pack files and negotiating with a
/// server: a shorter encoding of a delta's base. Pack files holding them can be read
/// either way. Enabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_ofs_delta(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_OFS_DELTA`.
    unsafe { set_int(LibraryOption::EnableOfsDelta, c_int::from(enabled)) }
}

/// Enables or disables `fsync` of the files libgit2 writes in a repository's git
/// directory, so new objects reach permanent storage. Disabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_fsync_gitdir(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_FSYNC_GITDIR`.
    unsafe { set_int(LibraryOption::EnableFsyncGitDir, c_int::from(enabled)) }
}

/// The share mode libgit2 opens files with on Windows (see `CreateFile`); `None`
/// elsewhere, where libgit2 keeps none and writes nothing.
///
/// Wraps: git_libgit2_opts
pub fn windows_sharemode() -> Option<c_ulong> {
    let mut mode: c_ulong = 0;
    // SAFETY: `GIT_OPT_GET_WINDOWS_SHAREMODE` reads one `unsigned long *`, a live
    // local, and on Windows writes through it; elsewhere it reads no argument at all.
    // The global is written only by `set_windows_sharemode`, whose caller excludes
    // this.
    let rc =
        unsafe { ffi::git_libgit2_opts(LibraryOption::GetWindowsShareMode.key(), &raw mut mode) };
    debug_assert_eq!(rc, 0);
    cfg!(windows).then_some(mode)
}

/// Sets the share mode libgit2 opens files with on Windows; the default is
/// `FILE_SHARE_READ | FILE_SHARE_WRITE`. Ignored elsewhere.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: on Windows, libgit2 keeps the setting in a global that its other
/// routines read without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_windows_sharemode(mode: c_ulong) {
    // SAFETY: `GIT_OPT_SET_WINDOWS_SHAREMODE` reads one `unsigned long` on Windows and
    // nothing elsewhere; the caller excludes concurrent access to its global.
    let rc = unsafe { ffi::git_libgit2_opts(LibraryOption::SetWindowsShareMode.key(), mode) };
    debug_assert_eq!(rc, 0);
}

/// Enables or disables verifying the hash of every object read from disk, at the cost
/// of hashing it again. Enabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_strict_hash_verification(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_STRICT_HASH_VERIFICATION`.
    unsafe {
        set_int(
            LibraryOption::EnableStrictHashVerification,
            c_int::from(enabled),
        )
    }
}

/// Makes `allocator` libgit2's memory allocator for every later allocation, or with
/// `None` restores the system allocator (`malloc` and friends).
///
/// libgit2 copies the three members into a global (`git__allocator`), so `allocator`
/// is borrowed for the call only. Does not initialise libgit2: initialisation keeps an
/// allocator installed before it, and itself allocates.
///
/// # Safety
///
/// - `allocator`'s members must form one allocator family meeting [`Alloc`]'s
///   contract, as an [`Allocator::new`] value does; an [`Allocator::from_fns`] value
///   does only if its functions do.
///
/// [`Alloc`]: crate::libgit2::util::sys::alloc::Alloc
/// [`Allocator::new`]: crate::libgit2::util::sys::alloc::Allocator::new
/// [`Allocator::from_fns`]: crate::libgit2::util::sys::alloc::Allocator::from_fns
/// - Every allocation libgit2 holds when this runs must be one the new family can
///   resize and release, since libgit2 releases it through whichever allocator is
///   current then: install before libgit2 is initialised (before any call into this
///   crate that reaches libgit2 — libgit2's global state holds allocations from its
///   initialisation on), or install a family that shares storage with the one it
///   replaces (two over libc's `malloc`, say). The same holds for every allocation a
///   live owner of this crate holds (an `OdbDataBox`, say).
/// - No other thread may call into libgit2 — through this crate or otherwise — while
///   this runs: every allocation reads the global without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_allocator(allocator: Option<AllocatorRef<'_>>) {
    let allocator = allocator.map_or(ptr::null(), |a| a.as_ptr());
    // SAFETY: `GIT_OPT_SET_ALLOCATOR` reads one `git_allocator *`, NULL or a live
    // value it `memcpy`s and does not retain. The family is coherent and every live
    // allocation is the new family's, and nothing else reads the global, per this
    // function's contract.
    let rc = unsafe { ffi::git_libgit2_opts(LibraryOption::SetAllocator.key(), allocator) };
    debug_assert_eq!(rc, 0);
}

/// Enables or disables refusing to reload the index from disk (as checkout does)
/// over unsaved changes in it; a forced checkout still overwrites them. Enabled by
/// default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_unsaved_index_safety(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_UNSAVED_INDEX_SAFETY`.
    unsafe {
        set_int(
            LibraryOption::EnableUnsavedIndexSafety,
            c_int::from(enabled),
        )
    }
}

/// The most objects libgit2 accepts in a pack file downloaded from a remote.
///
/// Wraps: git_libgit2_opts
pub fn pack_max_objects() -> usize {
    // SAFETY: as in `mwindow_size`, for `GIT_OPT_GET_PACK_MAX_OBJECTS`.
    unsafe { get_size(LibraryOption::GetPackMaxObjects) }
}

/// Sets the most objects libgit2 accepts in a pack file downloaded from a remote, to
/// bound the memory an untrusted remote can make it use.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_pack_max_objects(objects: usize) {
    // SAFETY: as in `set_mwindow_size`, for `GIT_OPT_SET_PACK_MAX_OBJECTS`.
    unsafe { set_size(LibraryOption::SetPackMaxObjects, objects) }
}

/// Enables or disables skipping the check for a `.keep` file next to each pack file,
/// which can speed up remote filesystems. Disabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn disable_pack_keep_file_checks(disabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_DISABLE_PACK_KEEP_FILE_CHECKS`.
    unsafe {
        set_int(
            LibraryOption::DisablePackKeepFileChecks,
            c_int::from(disabled),
        )
    }
}

/// Enables or disables `Expect: 100-continue` when POSTing to a server that
/// authenticates with NTLM or Negotiate. Not available on Windows.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn enable_http_expect_continue(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_ENABLE_HTTP_EXPECT_CONTINUE`.
    unsafe {
        set_int(
            LibraryOption::EnableHttpExpectContinue,
            c_int::from(enabled),
        )
    }
}

/// Sets the priority of the packed object-database backend libgit2 adds to a
/// repository with its default backends.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_odb_packed_priority(priority: c_int) {
    // SAFETY: `GIT_OPT_SET_ODB_PACKED_PRIORITY` reads one `int`; the caller excludes
    // concurrent access to its global.
    unsafe { set_int(LibraryOption::SetOdbPackedPriority, priority) }
}

/// Sets the priority of the loose object-database backend libgit2 adds to a
/// repository with its default backends.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_odb_loose_priority(priority: c_int) {
    // SAFETY: as in `set_odb_packed_priority`, for `GIT_OPT_SET_ODB_LOOSE_PRIORITY`.
    unsafe { set_int(LibraryOption::SetOdbLoosePriority, priority) }
}

/// The repository extensions libgit2 supports, sorted: its built-in ones (`noop`,
/// `objectformat`, ...) less those negated with [`set_extensions`], and those added
/// with it.
///
/// Pins the runtime first. Fails, with libgit2's error, when the copy cannot be
/// allocated.
///
/// Wraps: git_libgit2_opts
pub fn extensions() -> Result<StrArrayVal, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = StrArray::new();
    // SAFETY: `GIT_OPT_GET_EXTENSIONS` reads one `git_strarray *`, an empty array of
    // this function's own, and overwrites `strings` / `count` with a fresh array of
    // strings in libgit2's allocator, which `git_strarray_dispose` releases; it keeps
    // no pointer to `out`. The previous contents are empty, so nothing leaks. The
    // runtime is pinned; the extension list is written only by `set_extensions`,
    // whose caller excludes this.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            LibraryOption::GetExtensions.key(),
            out.as_mut().as_mut_ptr(),
        )
    };
    status(rc).map(|()| out)
}

/// Sets the repository extensions the caller supports, beyond libgit2's built-in
/// ones; a built-in one prefixed with `!` (`"!noop"`) is negated instead, so that
/// libgit2 refuses repositories needing it.
///
/// Replaces every extension set before. libgit2 copies each string, keeping one copy
/// of duplicates and none of the built-in names. Pins the runtime first. Fails, with
/// libgit2's error, when a copy cannot be allocated; the extensions copied until then
/// are set.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_extensions<S: AsRef<CStr>>(extensions: &[S]) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let ptrs: Vec<*const c_char> = extensions.iter().map(|s| s.as_ref().as_ptr()).collect();
    // SAFETY: `GIT_OPT_SET_EXTENSIONS` reads a `const char **` and a `size_t`: `ptrs`
    // holds that many NUL-terminated strings, borrowed from `extensions` for the
    // call, which libgit2 copies and does not retain (an empty `ptrs` is read not at
    // all). The runtime is pinned; the caller excludes concurrent access to the list.
    let rc = unsafe {
        ffi::git_libgit2_opts(
            LibraryOption::SetExtensions.key(),
            ptrs.as_ptr(),
            ptrs.len(),
        )
    };
    status(rc)
}

/// Whether libgit2 refuses to open a repository whose directory the current user does
/// not own.
///
/// Wraps: git_libgit2_opts
pub fn owner_validation() -> bool {
    // SAFETY: `GIT_OPT_GET_OWNER_VALIDATION` writes one `int` through one `int *`. Its
    // global is written only by `set_owner_validation`, whose caller excludes this.
    unsafe { get_int(LibraryOption::GetOwnerValidation) != 0 }
}

/// Sets whether libgit2 refuses to open a repository whose directory the current user
/// does not own. Enabled by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_owner_validation(enabled: bool) {
    // SAFETY: as in `enable_caching`, for `GIT_OPT_SET_OWNER_VALIDATION`.
    unsafe { set_int(LibraryOption::SetOwnerValidation, c_int::from(enabled)) }
}

/// The directory libgit2 takes as the current user's home for file lookups.
///
/// Pins the runtime first. Fails, with libgit2's error, when the copy cannot be
/// allocated.
///
/// Wraps: git_libgit2_opts
pub fn homedir() -> Result<CString, ErrorBuf> {
    buf_string(|buf| {
        // SAFETY: `GIT_OPT_GET_HOMEDIR` reads a `git_buf *`, the zeroed local
        // `buf_string` passes, which it fills and does not retain. The runtime is
        // pinned; the path is written only by `set_homedir`, whose caller excludes
        // this.
        unsafe { ffi::git_libgit2_opts(LibraryOption::GetHomeDir.key(), buf) }
    })
}

/// Sets the directory libgit2 takes as the current user's home for file lookups, or
/// with `None` resets it to the default libgit2 derives from the environment.
///
/// libgit2 copies `path`. Pins the runtime first. Fails, with libgit2's error, when the
/// copy cannot be allocated.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_homedir(path: Option<&CStr>) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let path = path.map_or(ptr::null(), CStr::as_ptr);
    // SAFETY: as in `set_template_path`, for `GIT_OPT_SET_HOMEDIR`.
    status(unsafe { ffi::git_libgit2_opts(LibraryOption::SetHomeDir.key(), path) })
}

/// The timeout for connecting to a remote server, in milliseconds; 0 is the system's.
///
/// Wraps: git_libgit2_opts
pub fn server_connect_timeout() -> c_int {
    // SAFETY: `GIT_OPT_GET_SERVER_CONNECT_TIMEOUT` writes one `int` through one
    // `int *`. Its global is written only by `set_server_connect_timeout`, whose
    // caller excludes this.
    unsafe { get_int(LibraryOption::GetServerConnectTimeout) }
}

/// Sets the timeout for connecting to a remote server, in milliseconds; 0 (the
/// default) is the system's, typically 75 seconds, which a longer timeout may not
/// exceed.
///
/// Pins the runtime first. Fails, with libgit2's error (`GIT_ERROR_INVALID`, "invalid
/// connect timeout"), for a negative timeout, leaving the setting alone.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_server_connect_timeout(timeout_ms: c_int) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    // SAFETY: `GIT_OPT_SET_SERVER_CONNECT_TIMEOUT` reads one `int`, which it
    // range-checks. The runtime is pinned, so a rejection is reported on live thread
    // state; the caller excludes concurrent access to the global.
    status(unsafe {
        ffi::git_libgit2_opts(LibraryOption::SetServerConnectTimeout.key(), timeout_ms)
    })
}

/// The timeout for reading from and writing to a remote server, in milliseconds; 0 is
/// the system's.
///
/// Wraps: git_libgit2_opts
pub fn server_timeout() -> c_int {
    // SAFETY: as in `server_connect_timeout`, for `GIT_OPT_GET_SERVER_TIMEOUT`, written
    // only by `set_server_timeout`.
    unsafe { get_int(LibraryOption::GetServerTimeout) }
}

/// Sets the timeout for reading from and writing to a remote server, in milliseconds;
/// 0 (the default) is the system's.
///
/// Pins the runtime first. Fails, with libgit2's error (`GIT_ERROR_INVALID`, "invalid
/// timeout"), for a negative timeout, leaving the setting alone.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_server_timeout(timeout_ms: c_int) -> Result<(), ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    // SAFETY: as in `set_server_connect_timeout`, for `GIT_OPT_SET_SERVER_TIMEOUT`.
    status(unsafe { ffi::git_libgit2_opts(LibraryOption::SetServerTimeout.key(), timeout_ms) })
}

/// The largest object, in bytes, libgit2 accepts in a pack file downloaded from a
/// remote.
///
/// Wraps: git_libgit2_opts
pub fn pack_max_object_size() -> usize {
    // SAFETY: as in `mwindow_size`, for `GIT_OPT_GET_PACK_MAX_OBJECT_SIZE`.
    unsafe { get_size(LibraryOption::GetPackMaxObjectSize) }
}

/// Sets the largest object, in bytes, libgit2 accepts in a pack file downloaded from a
/// remote; 2 GiB by default.
///
/// # Safety
///
/// No other thread may call into libgit2 — through this crate or otherwise — while
/// this runs: libgit2 keeps the setting in a global that its other routines read
/// without synchronisation.
///
/// Wraps: git_libgit2_opts
pub unsafe fn set_pack_max_object_size(size: usize) {
    // SAFETY: as in `set_mwindow_size`, for `GIT_OPT_SET_PACK_MAX_OBJECT_SIZE`.
    unsafe { set_size(LibraryOption::SetPackMaxObjectSize, size) }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::errors::ErrorClass;
    use crate::libgit2::util::sys::errors::set_str;

    #[test]
    fn feature_bits_are_distinct_single_bits() {
        let mut seen = 0;
        for (i, feature) in Feature::ALL.into_iter().enumerate() {
            assert_eq!(feature.raw(), 1 << i, "{feature:?}");
            assert_eq!(feature.bit() & seen, 0);
            seen |= feature.bit();
        }
        assert_eq!(Features::NAMED, seen);
        assert_eq!(Features::NAMED, (1 << 13) - 1);
    }

    #[test]
    fn feature_conversions_round_trip() {
        for feature in Feature::ALL {
            assert_eq!(Feature::from_raw(feature.raw()), Some(feature));
            assert_eq!(Feature::try_from(feature.raw()), Ok(feature));
            assert_eq!(ffi::git_feature_t::from(feature), feature.raw());
        }
        for raw in [0, 3, 1 << 13, 1 << 31, u32::MAX] {
            assert_eq!(Feature::from_raw(raw), None);
            let err = Feature::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 feature {raw} is not named by git2/common.h")
            );
        }
    }

    #[test]
    fn build_info_conversions_round_trip() {
        for info in [BuildInfo::Cpu, BuildInfo::Commit] {
            assert_eq!(BuildInfo::from_raw(info.raw()), Some(info));
            assert_eq!(BuildInfo::try_from(info.raw()), Ok(info));
            assert_eq!(ffi::git_buildinfo_t::from(info), info.raw());
        }
        assert_eq!(BuildInfo::Cpu.raw(), 1);
        assert_eq!(BuildInfo::Commit.raw(), 2);
        for raw in [0, 3, u32::MAX] {
            let err = BuildInfo::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 build information item {raw} is not named by git2/common.h")
            );
        }
    }

    #[test]
    fn features_set_operations() {
        let mut set = Features::empty();
        assert!(set.is_empty());
        assert_eq!(set, Features::default());
        assert_eq!(set.iter().next(), None);

        set.insert(Feature::Sha256);
        set |= Feature::Threads;
        assert_eq!(
            set.bits(),
            (ffi::GIT_FEATURE_SHA256 | ffi::GIT_FEATURE_THREADS) as c_int
        );
        assert!(set.contains(Feature::Threads) && set.contains(Feature::Sha256));
        assert!(!set.contains(Feature::Ssh));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [Feature::Threads, Feature::Sha256]
        );

        // Inserting twice and removing an absent feature change nothing.
        set.insert(Feature::Threads);
        set.remove(Feature::Ssh);
        assert_eq!(set, Feature::Threads | Feature::Sha256);
        set.remove(Feature::Threads);
        assert_eq!(set, Features::from(Feature::Sha256));

        let mut other = Features::from_iter([Feature::Http, Feature::Http]);
        other |= set;
        other.extend([Feature::Regex]);
        assert_eq!(
            other.into_iter().collect::<Vec<_>>(),
            [Feature::Regex, Feature::Sha256, Feature::Http]
        );
        assert_eq!(other | Feature::Nsec, other | Features::from(Feature::Nsec));
        assert_eq!(
            Feature::ALL.into_iter().collect::<Features>().bits(),
            Features::NAMED
        );
    }

    #[test]
    fn features_retain_unnamed_bits() {
        for bits in [-1, c_int::MIN, 1 << 13, 1 << 20 | 1, c_int::MAX] {
            let mut set = Features::from_bits_retain(bits);
            assert_eq!(set.bits(), bits);
            assert_eq!(set.unnamed_bits(), bits & !Features::NAMED);
            let named: Features = set.iter().collect();
            assert_eq!(named.bits(), bits & Features::NAMED);
            // Removing every named feature leaves the unnamed bits alone.
            for feature in Feature::ALL {
                set.remove(feature);
            }
            assert_eq!(set.bits(), set.unnamed_bits());
            assert_eq!(set.is_empty(), bits & !Features::NAMED == 0);
        }
    }

    #[test]
    fn features_iter_is_fused_and_bounded() {
        let mut iter = Features::from_bits_retain(-1).iter();
        assert_eq!(iter.size_hint(), (0, Some(13)));
        assert_eq!(iter.by_ref().count(), 13);
        assert_eq!(iter.size_hint(), (0, Some(0)));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn features_debug_lists_names_then_unnamed_bits() {
        assert_eq!(format!("{:?}", Features::empty()), "Features {}");
        assert_eq!(
            format!("{:?}", Feature::Threads | Feature::Sha1),
            "Features {Threads | Sha1}"
        );
        assert_eq!(
            format!("{:?}", Features::from_bits_retain(1 << 14 | 1)),
            "Features {Threads, 0x4000}"
        );
        assert_eq!(
            format!("{:?}", Features::from_bits_retain(1 << 14)),
            "Features {0x4000}"
        );
    }

    /// One past the last key `git2/common.h` declares.
    const END: ffi::git_libgit2_opt_t = ffi::GIT_OPT_SET_PACK_MAX_OBJECT_SIZE + 1;

    #[test]
    fn keys_are_dense_and_round_trip() {
        for raw in 0..END {
            let opt = LibraryOption::from_raw(raw).unwrap();
            assert_eq!(opt.raw(), raw);
            assert_eq!(ffi::git_libgit2_opt_t::from(opt), raw);
            assert_eq!(opt.key(), raw as c_int);
            assert_eq!(LibraryOption::from_key(opt.key()), Some(opt));
            assert_eq!(LibraryOption::try_from(raw), Ok(opt));
        }
    }

    #[test]
    fn keys_past_the_header_are_unnamed() {
        for raw in [END, END + 1, 1000, u32::MAX - 1, u32::MAX] {
            assert_eq!(LibraryOption::from_raw(raw), None);
            assert_eq!(LibraryOption::try_from(raw), Err(UnnamedLibraryOption(raw)));
        }
    }

    #[test]
    fn negative_keys_are_refused_before_conversion() {
        // `-1 as u32` is `u32::MAX`, unnamed too, but `from_key` must not rely on it.
        for key in [-1, -2, c_int::MIN] {
            assert_eq!(LibraryOption::from_key(key), None);
        }
        assert_eq!(LibraryOption::from_key(END as c_int), None);
        assert_eq!(LibraryOption::from_key(c_int::MAX), None);
    }

    #[test]
    fn unnamed_key_displays_its_value() {
        let err = LibraryOption::try_from(END).unwrap_err();
        assert_eq!(err.raw(), END);
        assert_eq!(
            err.to_string(),
            format!("libgit2 option key {END} is not named by git2/common.h")
        );
        let boxed: Box<dyn std::error::Error> = Box::new(err);
        assert!(boxed.source().is_none());
    }

    #[test]
    fn variants_are_distinct() {
        use std::collections::HashSet;
        let set: HashSet<LibraryOption> = (0..END).filter_map(LibraryOption::from_raw).collect();
        assert_eq!(set.len(), END as usize);
        assert_eq!(format!("{:?}", LibraryOption::SetAllocator), "SetAllocator");
    }

    #[test]
    fn version_displays_dotted() {
        let v = Version {
            major: 1,
            minor: 9,
            revision: 0,
        };
        assert_eq!(v.to_string(), "1.9.0");
        assert!(
            v < Version {
                major: 1,
                minor: 10,
                revision: 0
            }
        );
        assert!(
            v > Version {
                major: 0,
                minor: 99,
                revision: 99
            }
        );
    }

    #[test]
    fn search_path_levels_are_the_config_levels() {
        assert_eq!(SearchPathLevel::System.raw(), ffi::GIT_CONFIG_LEVEL_SYSTEM);
        assert_eq!(SearchPathLevel::Xdg.raw(), ffi::GIT_CONFIG_LEVEL_XDG);
        assert_eq!(SearchPathLevel::Global.raw(), ffi::GIT_CONFIG_LEVEL_GLOBAL);
    }

    #[test]
    fn cached_memory_defaults_to_zero() {
        assert_eq!(
            CachedMemory::default(),
            CachedMemory {
                current: 0,
                allowed: 0
            }
        );
    }

    /// A key that fails leaves its error; a key that leaves the buffer untouched (NULL)
    /// reads as empty; a filled buffer is copied and disposed (LSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn buf_string_covers_every_outcome() {
        let failed = buf_string(|_| {
            set_str(ErrorClass::Invalid, c"refused by the test").unwrap();
            -1
        })
        .unwrap_err();
        assert_eq!(failed.class(), Some(ErrorClass::Invalid));
        assert_eq!(failed.message(), c"refused by the test");
        assert_eq!(buf_string(|_| 0).unwrap(), CString::default());
        let agent = buf_string(|buf| {
            // SAFETY: `GIT_OPT_GET_USER_AGENT` fills the zeroed `git_buf *` it is given.
            unsafe { ffi::git_libgit2_opts(LibraryOption::GetUserAgent.key(), buf) }
        })
        .unwrap();
        assert!(agent.to_bytes().starts_with(b"libgit2 "));
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn status_reads_the_thread_error() {
        assert!(runtime_pinned());
        assert_eq!(status(0), Ok(()));
        assert_eq!(status(1), Ok(()));
        set_str(ErrorClass::Os, c"from the test").unwrap();
        let e = status(-1).unwrap_err();
        assert_eq!(e.class(), Some(ErrorClass::Os));
        assert_eq!(e.message(), c"from the test");
    }
}
