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

use core::ffi::c_int;
use core::fmt;
use core::ops::{BitOr, BitOrAssign};

use crate::ffi;

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

#[cfg(test)]
mod unit_tests {
    use super::*;

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
}
