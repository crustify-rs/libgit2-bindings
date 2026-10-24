//! Safe wrappers over `include/git2/global.h`: initialising and shutting down
//! libgit2's global state.
//!
//! libgit2 counts initialisations (`git_runtime_init`, under its own mutex): only the
//! first, the 0 -> 1 transition, sets the subsystems up (the allocator, the per-thread
//! error state's TLS key, the system directories, OpenSSL, ...), and only the shutdown
//! that brings the count back to 0 tears them down again. A [`Runtime`] is one counted
//! reference: [`init`] takes it, and dropping it (or [`Runtime::shutdown`]) releases
//! exactly that one, so the count can never be released more often than it was taken.
//! A C caller can call `git_libgit2_shutdown` unbalanced, driving the count negative;
//! libgit2 then sets itself up again only once as many initialisations have brought
//! the count back through 1, and [`init`] reports the 0 it returns on the way as a
//! failure.
//!
//! Every safe wrapper that needs libgit2's global state holds a reference of its own,
//! taken on first use and kept for the rest of the process (`runtime_pinned`, in
//! `util::sys::errors`, a [`Runtime`] that is never dropped). So releasing a
//! [`Runtime`] never tears down state a live object relies on: the count reaches 0
//! only while no wrapper has needed libgit2 yet. A [`Runtime`] is for code that wants
//! the state set up eagerly, to see an initialisation failure where it happens, or to
//! tear libgit2 down again before anything pinned it.
//!
//! When the first initialisation fails part-way, libgit2 leaves its count at 1 with
//! some subsystems set up and others not, the per-thread error state possibly without
//! a TLS key, and answers every later `git_libgit2_init` with success. [`init`] does not
//! trust that: once an initialisation through it has failed, it refuses every later one
//! ([`InitError::Poisoned`]) without calling libgit2.

use core::ffi::{CStr, c_int};
use core::fmt;
use core::num::NonZeroUsize;
use core::ops::{BitOr, BitOrAssign};
use std::sync::{Mutex, PoisonError};

use crate::ffi;
use crate::libgit2::src::libgit2::settings::Version;

/// One configurable feature of libgit2: an optional setting (threading, say), or a
/// capability that one of several backend providers supplies (HTTPS, through
/// OpenSSL, mbedTLS or a system library). Each is a single bit of a [`Features`] set.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: `git_feature_t`
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
    #[must_use]
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
    #[must_use]
    pub const fn raw(self) -> ffi::git_feature_t {
        self as ffi::git_feature_t
    }

    /// The feature's bit in the `int` `git_libgit2_features` returns. Every bit is
    /// below `1 << 13`, so the value is unchanged.
    pub(crate) const fn bit(self) -> c_int {
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
    #[must_use]
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
/// Wraps: `git_feature_t`
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Features(c_int);

impl Features {
    /// The bits of every [`Feature`].
    pub(crate) const NAMED: c_int = {
        let mut bits = 0;
        let mut i = 0;
        while i < Feature::ALL.len() {
            bits |= Feature::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The set with no feature.
    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as `git_libgit2_features` returns it; bits
    /// that no [`Feature`] names are kept.
    #[must_use]
    pub const fn from_bits_retain(bits: c_int) -> Self {
        Self(bits)
    }

    /// The bits of the set, as `git_libgit2_features` returns them.
    #[must_use]
    pub const fn bits(self) -> c_int {
        self.0
    }

    /// The bits of the set that no [`Feature`] names.
    #[must_use]
    pub const fn unnamed_bits(self) -> c_int {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `feature` is in the set.
    #[must_use]
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
    #[must_use]
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
    type IntoIter = FeaturesIter;
    type Item = Feature;

    fn into_iter(self) -> FeaturesIter {
        self.iter()
    }
}

/// Lists the named features, then any unnamed bits in hex: `{Threads | Sha1, 0x4000}`.
impl fmt::Debug for Features {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Features {")?;
        let mut sep = "";
        for feature in *self {
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

/// The version of the libgit2 library in use.
///
/// libgit2 answers from constants compiled into it, so this needs no initialisation
/// and cannot fail.
///
/// Wraps: `git_libgit2_version`
#[must_use]
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
/// Wraps: `git_libgit2_prerelease`
#[must_use]
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
/// Wraps: `git_libgit2_features`
#[must_use]
pub fn features() -> Features {
    // SAFETY: no arguments; the routine ORs compile-time constants.
    Features::from_bits_retain(unsafe { ffi::git_libgit2_features() })
}

/// The backend that provides `feature` in the libgit2 library in use: `"openssl"`
/// for [`Feature::Https`] over OpenSSL, `"pcre2"` for [`Feature::Regex`], and so on.
/// `None` when the feature is not compiled in, or has no backend to choose:
/// [`Feature::Http`] never has one, its presence is all [`features`] reports.
///
/// libgit2 returns a string literal of its own, as [`prerelease`] does, chosen by the
/// build's defines. Needs no initialisation: the routine's only other path, an
/// assertion for a compiled-in feature whose backend no define names, cannot be
/// reached by a build that `CMake` configured, which always names one.
///
/// Wraps: `git_libgit2_feature_backend`
#[must_use]
pub fn feature_backend(feature: Feature) -> Option<&'static CStr> {
    // SAFETY: a named `git_feature_t`, by value; the routine reads no state.
    let s = unsafe { ffi::git_libgit2_feature_backend(feature.raw()) };
    // SAFETY: a non-NULL result is one of the routine's NUL-terminated literals, in
    // libgit2.so's read-only data, never written or freed while the library is loaded,
    // which it is for the rest of the process.
    (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) })
}

/// The most references [`init`] lets libgit2's count reach: half of `c_int::MAX`,
/// well short of the wrap-around that would make libgit2 set itself up a second time
/// under live users (as `Arc` stops short of `isize::MAX`).
///
/// The unit tests lower it, so they can reach it.
const MAX_REFERENCES: c_int = if cfg!(test) { 64 } else { c_int::MAX / 2 };

/// Serialises [`init`]'s calls and records whether one failed: a failure is recorded
/// before any later caller can reach `git_libgit2_init`, which would answer it with a
/// success.
static INIT: Mutex<bool> = Mutex::new(false);

/// One `git_libgit2_init` reference: libgit2's global state stays initialised while
/// it lives, and dropping it releases the reference with `git_libgit2_shutdown`.
///
/// It holds no pointer and no thread affinity — libgit2 counts references under its
/// own mutex — so it is `Send` and `Sync`, and may be dropped on any thread. It is not
/// `Clone`: another reference is another [`init`], which can fail.
///
/// Handwritten
#[derive(Debug)]
#[must_use = "dropping the reference releases it at once"]
pub struct Runtime {
    count: NonZeroUsize,
}

/// Initialises libgit2's global state, or takes one more reference to it.
///
/// Returns the reference, which knows how many references were live once it was
/// taken, itself included ([`Runtime::count`]): 1 when this call set libgit2 up.
///
/// No libgit2 error is set or reported: after a failed initialisation, the per-thread
/// error state may have no TLS key to be looked up under.
///
/// Wraps: `git_libgit2_init`
///
/// # Errors
///
/// Fails when libgit2 cannot be initialised ([`InitError::Failed`]), or when an
/// unbalanced C `git_libgit2_shutdown` had driven the count negative, so that this
/// call only brought it to 0 and set nothing up; from then on every call fails
/// ([`InitError::Poisoned`]), where C's `git_libgit2_init` would report success over
/// half-initialised state. It also refuses to let the count grow past `c_int::MAX / 2`
/// ([`InitError::TooManyReferences`]), so a program leaking references cannot wrap it
/// around.
pub fn init() -> Result<Runtime, InitError> {
    let mut failed = INIT.lock().unwrap_or_else(PoisonError::into_inner);
    if *failed {
        return Err(InitError::Poisoned);
    }
    // SAFETY: no arguments; libgit2 counts initialisations under its own mutex, so it
    // is callable from any thread at any time.
    let count = unsafe { ffi::git_libgit2_init() };
    if count > MAX_REFERENCES {
        // SAFETY: releases the reference this call just took, which is not the last:
        // the count is far above 1.
        unsafe { ffi::git_libgit2_shutdown() };
        return Err(InitError::TooManyReferences);
    }
    if let Some(count) = usize::try_from(count).ok().and_then(NonZeroUsize::new) {
        Ok(Runtime { count })
    } else {
        *failed = true;
        Err(InitError::Failed)
    }
}

impl Runtime {
    /// How many references were live once this one was taken, itself included, as
    /// `git_libgit2_init` returned it: 1 when taking it set libgit2 up.
    ///
    /// A snapshot: other references come and go independently.
    #[must_use]
    pub fn count(&self) -> NonZeroUsize {
        self.count
    }

    /// Releases the reference, as dropping it does, and reports how many references
    /// remain, as `git_libgit2_shutdown` returns it: 0 when this one was the last and
    /// libgit2 tore its global state down.
    ///
    /// Wraps: `git_libgit2_shutdown`
    ///
    /// # Errors
    ///
    /// Fails when libgit2 cannot take or release its initialisation mutex, in which
    /// case the reference may stay counted, which keeps the state alive rather than
    /// tearing it down under another holder; or when an unbalanced C
    /// `git_libgit2_shutdown` had already released this reference's count, so that
    /// libgit2 reports a negative one.
    pub fn shutdown(self) -> Result<usize, ShutdownError> {
        // The reference is released here, not again by `Drop`.
        let _ = core::mem::ManuallyDrop::new(self);
        release()
    }
}

/// Releases the reference with `git_libgit2_shutdown`, ignoring a mutex failure as
/// [`Runtime::shutdown`] would report it.
///
/// Wraps: `git_libgit2_shutdown`
impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = release();
    }
}

/// `git_libgit2_shutdown` for a reference a [`Runtime`] held.
fn release() -> Result<usize, ShutdownError> {
    // SAFETY: no arguments; the caller gives up a reference `init` took, so the count
    // is released no more often than it was taken. libgit2 counts under its own mutex.
    let remaining = unsafe { ffi::git_libgit2_shutdown() };
    usize::try_from(remaining).map_err(|_| ShutdownError)
}

/// [`init`] could not take a reference to libgit2's global state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InitError {
    /// `git_libgit2_init` failed: libgit2 could not take its initialisation mutex, or
    /// a subsystem could not be set up (an allocator refusing requests, say); or it
    /// answered 0, having set nothing up, because an unbalanced C
    /// `git_libgit2_shutdown` had driven the count negative.
    Failed,
    /// An earlier initialisation through [`init`] failed, leaving libgit2
    /// half-initialised for the rest of the process.
    Poisoned,
    /// libgit2's count of references reached [`init`]'s ceiling, `c_int::MAX / 2`.
    TooManyReferences,
}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Failed => "libgit2 could not be initialised",
            Self::Poisoned => "an earlier libgit2 initialisation failed",
            Self::TooManyReferences => "too many libgit2 initialisations are live",
        })
    }
}

impl std::error::Error for InitError {}

/// [`Runtime::shutdown`] failed: libgit2 could not take or release its
/// initialisation mutex, or reported a negative count, an unbalanced C
/// `git_libgit2_shutdown` having released the reference's count before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShutdownError;

impl fmt::Display for ShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("libgit2 could not take its initialisation lock to shut down")
    }
}

impl std::error::Error for ShutdownError {}

#[cfg(test)]
mod unit_tests {
    use core::ffi::CStr;
    use core::ptr;
    use core::ptr::NonNull;
    use std::process::Command;

    use super::*;
    use crate::libgit2::src::libgit2::settings::set_allocator;
    use crate::libgit2::src::util::alloc::{Alloc, Allocator};

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
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

    /// An allocator refusing every request, so the first initialisation fails.
    struct Refusing;

    // SAFETY: it never hands out an allocation, which `Alloc` allows; `free` is never
    // reached, since nothing was allocated.
    unsafe impl Alloc for Refusing {
        fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            None
        }

        unsafe fn realloc(
            _: Option<NonNull<u8>>,
            _: usize,
            _: Option<&CStr>,
            _: i32,
        ) -> Option<NonNull<u8>> {
            None
        }

        unsafe fn free(_: NonNull<u8>) {}
    }

    static REFUSING: Allocator = Allocator::new::<Refusing>();

    #[test]
    fn errors_report_themselves() {
        assert_eq!(
            InitError::Failed.to_string(),
            "libgit2 could not be initialised"
        );
        assert_eq!(
            InitError::Poisoned.to_string(),
            "an earlier libgit2 initialisation failed"
        );
        assert_eq!(
            InitError::TooManyReferences.to_string(),
            "too many libgit2 initialisations are live"
        );
        assert_eq!(
            ShutdownError.to_string(),
            "libgit2 could not take its initialisation lock to shut down"
        );
        let boxed: Box<dyn std::error::Error> = Box::new(InitError::Poisoned);
        assert!(boxed.source().is_none());
        assert_eq!(format!("{:?}", InitError::Failed), "Failed");
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn runtime_debug_shows_its_count() {
        let r = init().unwrap();
        assert!(format!("{r:?}").starts_with("Runtime { count: "));
    }

    /// Corrects C: after a failed first initialisation, `git_libgit2_init` answers
    /// every later call with success over half-initialised state; `init` refuses.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn a_failed_initialisation_poisons_later_ones() {
        isolated(
            concat!(
                module_path!(),
                "::a_failed_initialisation_poisons_later_ones"
            ),
            || {
                // SAFETY: nothing has initialised libgit2 or allocated through it in
                // this child, and no other thread runs.
                unsafe { set_allocator(Some(REFUSING.as_ref())) };
                assert_eq!(init().unwrap_err(), InitError::Failed);
                // SAFETY: the system allocator, which refused nothing and so owns
                // every block libgit2 holds (none); no other thread runs.
                unsafe { set_allocator(None) };
                assert_eq!(init().unwrap_err(), InitError::Poisoned);
                assert_eq!(init().unwrap_err(), InitError::Poisoned, "for good");
                // C's own answer: success, over the count the failure left at 1.
                // SAFETY: no arguments; the reference dies with the child.
                assert_eq!(unsafe { ffi::git_libgit2_init() }, 2);
                assert_eq!(init().unwrap_err(), InitError::Poisoned);
            },
        );
    }

    /// The count stops at the ceiling (64 in unit tests): the reference that would
    /// pass it is released again and refused.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn the_count_stops_at_the_ceiling() {
        isolated(
            concat!(module_path!(), "::the_count_stops_at_the_ceiling"),
            || {
                let held: Vec<Runtime> = (1..=MAX_REFERENCES)
                    .map(|n| {
                        let r = init().unwrap();
                        assert_eq!(r.count().get(), usize::try_from(n).unwrap());
                        r
                    })
                    .collect();
                assert_eq!(init().unwrap_err(), InitError::TooManyReferences);
                assert_eq!(init().unwrap_err(), InitError::TooManyReferences);
                let mut held = held;
                assert_eq!(held.pop().unwrap().shutdown(), Ok(63));
                assert_eq!(init().unwrap().count().get(), 64, "not poisoned");
                drop(held);
                assert_eq!(init().unwrap().count().get(), 1);
            },
        );
    }

    /// An unbalanced C shutdown drives the count negative: the `init` that brings it
    /// to 0 sets nothing up and fails, poisoning later ones, while C's next
    /// initialisation sets libgit2 up again.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn an_unbalanced_c_shutdown_fails_the_next_init() {
        isolated(
            concat!(
                module_path!(),
                "::an_unbalanced_c_shutdown_fails_the_next_init"
            ),
            || {
                // SAFETY: no arguments; releases a reference never taken, which is
                // this test's subject, in a child of its own.
                assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, -1);
                assert_eq!(init().unwrap_err(), InitError::Failed, "answered 0");
                assert_eq!(init().unwrap_err(), InitError::Poisoned);
                // SAFETY: no arguments; balanced by the shutdown below.
                assert_eq!(unsafe { ffi::git_libgit2_init() }, 1, "C sets up again");
                let mut buf = ffi::git_buf {
                    ptr: ptr::null_mut(),
                    reserved: 0,
                    size: 0,
                };
                // SAFETY: `GIT_OPT_GET_USER_AGENT` fills the zeroed `git_buf *` it is
                // given; libgit2 is initialised; disposed below.
                let rc = unsafe {
                    ffi::git_libgit2_opts(ffi::GIT_OPT_GET_USER_AGENT.cast_signed(), &raw mut buf)
                };
                assert_eq!(rc, 0);
                // SAFETY: libgit2 filled the buffer with a NUL-terminated string.
                let agent = unsafe { CStr::from_ptr(buf.ptr) }.to_owned();
                // SAFETY: as libgit2 left it.
                unsafe { ffi::git_buf_dispose(&raw mut buf) };
                assert!(agent.to_bytes().starts_with(b"libgit2 "));
                // SAFETY: releases the reference just taken.
                assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, 0);
            },
        );
    }

    /// An unbalanced C shutdown that released a `Runtime`'s count before it makes
    /// the `Runtime`'s own release fail.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn an_unbalanced_c_shutdown_fails_the_runtimes_release() {
        isolated(
            concat!(
                module_path!(),
                "::an_unbalanced_c_shutdown_fails_the_runtimes_release"
            ),
            || {
                let r = init().unwrap();
                assert_eq!(r.count().get(), 1);
                // SAFETY: no arguments; releases the Rust reference's count from C,
                // which is this test's subject, in a child of its own.
                assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, 0);
                assert_eq!(r.shutdown(), Err(ShutdownError), "the count went to -1");
                // SAFETY: no arguments; brings the count back to 0, setting nothing up.
                assert_eq!(unsafe { ffi::git_libgit2_init() }, 0);
            },
        );
    }

    /// `shutdown` and `drop` release exactly one reference each.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shutdown_and_drop_release_one_each() {
        isolated(
            concat!(module_path!(), "::shutdown_and_drop_release_one_each"),
            || {
                let a = init().unwrap();
                let b = init().unwrap();
                let c = init().unwrap();
                assert_eq!(c.count().get(), 3);
                drop(b);
                assert_eq!(a.shutdown(), Ok(1));
                assert_eq!(c.shutdown(), Ok(0));
                // SAFETY: no arguments; balanced by the shutdown below.
                assert_eq!(unsafe { ffi::git_libgit2_init() }, 1, "back at 0 before");
                // SAFETY: releases the reference just taken.
                assert_eq!(unsafe { ffi::git_libgit2_shutdown() }, 0);
            },
        );
    }
}
