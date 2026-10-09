//! Safe wrappers over `include/git2/oid.h`.
//!
//! libgit2 passes an object id's type (`git_oid_t`) by value: it names the hash
//! function behind the id, which fixes the id's size. Values the header does not name
//! travel through it too: a zeroed `oid_type` field of an options struct means "the
//! default type" ([`OidType::default`]), and `git_oid.type` stores whatever byte it was
//! given. So a raw value is converted fallibly ([`OidType::from_raw`]) and never
//! transmuted.

use core::fmt;

use crate::ffi;

/// The type of an object id: the hash function that produced it, which fixes how
/// many bytes of a `git_oid` hold the id ([`size`](Self::size)) and how many hex
/// digits format it ([`hex_size`](Self::hex_size)).
///
/// libgit2's parsers reject any value without a variant as an "unknown type", and its
/// options structs read `0` as [`OidType::default`];
/// [`from_raw`](Self::from_raw) gives `None` for all of them.
///
/// Wraps: git_oid_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum OidType {
    /// SHA-1: 20 bytes, 40 hex digits.
    Sha1 = ffi::GIT_OID_SHA1,
    /// SHA-256: 32 bytes, 64 hex digits.
    Sha256 = ffi::GIT_OID_SHA256,
}

impl OidType {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_oid_t) -> Option<Self> {
        match raw {
            ffi::GIT_OID_SHA1 => Some(Self::Sha1),
            ffi::GIT_OID_SHA256 => Some(Self::Sha256),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    pub const fn raw(self) -> ffi::git_oid_t {
        self as ffi::git_oid_t
    }

    /// The size in bytes of an id of this type (`GIT_OID_SHA1_SIZE`,
    /// `GIT_OID_SHA256_SIZE`).
    pub const fn size(self) -> usize {
        match self {
            Self::Sha1 => ffi::GIT_OID_SHA1_SIZE as usize,
            Self::Sha256 => ffi::GIT_OID_SHA256_SIZE as usize,
        }
    }

    /// The length in hex digits of an id of this type (`GIT_OID_SHA1_HEXSIZE`,
    /// `GIT_OID_SHA256_HEXSIZE`).
    pub const fn hex_size(self) -> usize {
        match self {
            Self::Sha1 => ffi::GIT_OID_SHA1_HEXSIZE as usize,
            Self::Sha256 => ffi::GIT_OID_SHA256_HEXSIZE as usize,
        }
    }
}

/// libgit2's default type, `GIT_OID_DEFAULT`: the one a zeroed `oid_type` selects.
impl Default for OidType {
    fn default() -> Self {
        Self::Sha1
    }
}

impl TryFrom<ffi::git_oid_t> for OidType {
    type Error = UnnamedOidType;

    fn try_from(raw: ffi::git_oid_t) -> Result<Self, UnnamedOidType> {
        Self::from_raw(raw).ok_or(UnnamedOidType(raw))
    }
}

impl From<OidType> for ffi::git_oid_t {
    fn from(value: OidType) -> ffi::git_oid_t {
        value.raw()
    }
}

/// An object id type that this version of libgit2's header does not name: `0`, which
/// an options struct reads as the default, or a value libgit2 rejects.
/// [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedOidType(ffi::git_oid_t);

impl UnnamedOidType {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_oid_t {
        self.0
    }
}

impl fmt::Display for UnnamedOidType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 object id type {} is not named by git2/oid.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedOidType {}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn from_raw_round_trips_every_variant() {
        for ty in [OidType::Sha1, OidType::Sha256] {
            assert_eq!(OidType::from_raw(ty.raw()), Some(ty));
            assert_eq!(OidType::try_from(ty.raw()), Ok(ty));
            assert_eq!(ffi::git_oid_t::from(ty), ty.raw());
        }
    }

    #[test]
    fn unnamed_values_are_rejected_with_their_value() {
        for raw in [0, 3, 255, 256, ffi::git_oid_t::MAX] {
            assert_eq!(OidType::from_raw(raw), None);
            let err = OidType::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 object id type {raw} is not named by git2/oid.h")
            );
            let source: &dyn std::error::Error = &err;
            assert!(source.source().is_none());
        }
    }

    #[test]
    fn sizes_follow_the_header() {
        assert_eq!(OidType::Sha1.size(), 20);
        assert_eq!(OidType::Sha1.hex_size(), 40);
        assert_eq!(OidType::Sha256.size(), 32);
        assert_eq!(OidType::Sha256.hex_size(), 64);
        for ty in [OidType::Sha1, OidType::Sha256] {
            assert_eq!(ty.hex_size(), 2 * ty.size());
        }
    }

    #[test]
    fn default_is_sha1_and_debug_names_the_variant() {
        assert_eq!(OidType::default(), OidType::Sha1);
        assert_eq!(format!("{:?}", OidType::Sha256), "Sha256");
        assert_eq!(
            format!("{:?}", OidType::try_from(0)),
            "Err(UnnamedOidType(0))"
        );
    }

    #[test]
    fn option_uses_a_niche() {
        assert_eq!(size_of::<Option<OidType>>(), size_of::<OidType>());
        assert_eq!(size_of::<OidType>(), size_of::<ffi::git_oid_t>());
    }
}
