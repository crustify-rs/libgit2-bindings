//! The patch parser of `src/libgit2/diff_parse.c`: [`Diff`]'s
//! constructors from the text of a Git patch, for SHA-1 or a given object id type, and the
//! parse options ([`DiffParseOptions`]) that choose it.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::{align_of, size_of};
use core::ptr::{self};

use ffibox::{CMutSeam, CRefSeam, define_ctype};

use crate::ffi;
use crate::libgit2::src::libgit2::diff::{Diff, DiffBox, DiffError, constructed};
use crate::libgit2::src::libgit2::oid::OidType;
use crate::libgit2::src::util::errors::runtime_pinned;

impl Diff<'static> {
    /// Parse Git patch bytes; paths, hunk lines and binary data are copied by C. Empty input is an
    /// empty diff.
    ///
    /// # Errors
    ///
    /// Non-Git unified diffs are rejected by C.
    ///
    /// Fails, as well, with libgit2's error when it cannot be initialised.
    ///
    /// Wraps `fn::git_diff_from_buffer`
    #[doc(alias = "git_diff_from_buffer")]
    pub fn from_buffer(content: &[u8]) -> Result<DiffBox<'static>, DiffError> {
        if !runtime_pinned() {
            return Err(DiffError::from_libgit2(-1));
        }
        let (mut out, data) = (ptr::null_mut(), content.as_ptr().cast());
        // SAFETY: content is readable for its length, including the non-null empty
        // slice; out is writable. C copies the data used after parsing; retained
        // input cursors are never dereferenced again (see the module docs).
        let rc = unsafe { ffi::git_diff_from_buffer(&raw mut out, data, content.len()) };
        // SAFETY: successful parsing hands out a fresh sole reference with no repository,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }

    /// Parse with an explicit object-id type.
    ///
    /// # Errors
    ///
    /// Fails as [`from_buffer`](Self::from_buffer) does.
    ///
    /// Wraps `fn::git_diff_from_buffer_ext`
    #[doc(alias = "git_diff_from_buffer_ext")]
    pub fn from_buffer_with_oid_type(
        content: &[u8],
        kind: OidType,
    ) -> Result<DiffBox<'static>, DiffError> {
        Self::from_buffer_with_options(content, DiffParseOptions::with_oid_type(kind))
    }

    /// Parse with the given options: the entire parse-options surface of
    /// `git_diff_from_buffer_ext`. An unset object id type parses SHA-1, as
    /// [`from_buffer`](Self::from_buffer) does; the structure version is not checked.
    ///
    /// # Errors
    ///
    /// Fails as [`from_buffer`](Self::from_buffer) does.
    pub fn from_buffer_with_options(
        content: &[u8],
        opts: DiffParseOptions,
    ) -> Result<DiffBox<'static>, DiffError> {
        if !runtime_pinned() {
            return Err(DiffError::from_libgit2(-1));
        }
        let mut opts = ffi::git_diff_parse_options::from(opts);
        let mut out = ptr::null_mut();
        // SAFETY: options and bytes live through parsing; C only reads the options' two
        // scalars, and the oid type is 0 or a named type (`DiffParseOptions` holds no other).
        // C copies paths/lines/data; the retained parse cursor is never dereferenced after
        // parsing finishes.
        let rc = unsafe {
            ffi::git_diff_from_buffer_ext(
                &raw mut out,
                content.as_ptr().cast(),
                content.len(),
                &raw mut opts,
            )
        };
        // SAFETY: successful parsing hands out a fresh sole reference with no repository,
        // released by DiffFree; every failure leaves `out` NULL (adopted as None).
        constructed(rc, unsafe { DiffBox::from_c(out) })
    }
}

define_ctype!(
    /// The options of the patch parser (`git_diff_parse_options`): the structure version, and
    /// the object id type the patch's ids are parsed as.
    ///
    /// A plain value of 8 bytes, resource-free: `Copy`, held inline, never released. Build
    /// one with [`new`](Self::new) (or `Default`), read and write its fields through
    /// [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut), and parse with it through
    /// [`Diff::from_buffer_with_options`]:
    ///
    /// ```
    /// use libgit2::libgit2::src::libgit2::diff_parse::DiffParseOptions;
    /// use libgit2::libgit2::src::libgit2::oid::OidType;
    ///
    /// let mut opts = DiffParseOptions::new();
    /// assert_eq!(opts.as_ref().oid_type(), None);
    /// opts.as_mut().set_oid_type(Some(OidType::Sha256));
    /// assert_eq!(opts.as_ref().oid_type(), Some(OidType::Sha256));
    /// assert_eq!(opts.as_ref().version(), DiffParseOptions::VERSION);
    /// ```
    ///
    /// libgit2 does not check `oid_type`: a type it does not name sizes the patch's ids as no
    /// type at all. So the value only ever holds 0, which libgit2 reads as its default,
    /// SHA-1, or a named [`OidType`]; there is no conversion from a raw
    /// `git_diff_parse_options`, and the setter takes an `Option<OidType>`. libgit2 never
    /// reads the version, so any version may be stored.
    ///
    /// A handle borrows the value, so it cannot outlive it:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::src::libgit2::diff_parse::DiffParseOptions;
    ///
    /// let handle = {
    ///     let opts = DiffParseOptions::new();
    ///     opts.as_ref()
    /// };
    /// let _ = handle.version();
    /// ```
    ///
    /// and the shared handle only reads:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::src::libgit2::diff_parse::DiffParseOptions;
    ///
    /// let opts = DiffParseOptions::new();
    /// opts.as_ref().set_oid_type(None);
    /// ```
    ///
    /// Wraps `ty::git_diff_parse_options`
    #[doc(alias = "git_diff_parse_options")]
    #[derive(Clone, Copy)]
    DiffParseOptions,
    DiffParseOptionsRef,
    DiffParseOptionsMut,
    ffi::git_diff_parse_options
);

// SAFETY: a `DiffParseOptions` is 8 plain bytes: no pointer, no interior mutability, no
// thread affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for DiffParseOptions {}

// SAFETY: `DiffParseOptionsMut` is an exclusive borrow of such a value, as
// `&mut DiffParseOptions`.
unsafe impl Send for DiffParseOptionsMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through
// `DiffParseOptionsMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for DiffParseOptions {}

// SAFETY: as above; `DiffParseOptionsRef` is a shared borrow, as `&DiffParseOptions`.
unsafe impl Send for DiffParseOptionsRef<'_> {}

// SAFETY: as above.
unsafe impl Sync for DiffParseOptionsRef<'_> {}

// SAFETY: `&DiffParseOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for DiffParseOptionsMut<'_> {}

impl DiffParseOptions {
    /// The structure version this crate's header declares
    /// (`GIT_DIFF_PARSE_OPTIONS_VERSION`).
    pub const VERSION: u32 = ffi::GIT_DIFF_PARSE_OPTIONS_VERSION;

    /// The default options, as `GIT_DIFF_PARSE_OPTIONS_INIT` spells them: the current
    /// version, and `oid_type` 0, which libgit2 reads as SHA-1. libgit2 has no init routine
    /// for these options, so this calls no C and needs no initialised libgit2.
    #[must_use]
    pub const fn new() -> DiffParseOptions {
        DiffParseOptions(
            ffi::git_diff_parse_options {
                version: Self::VERSION,
                oid_type: 0,
            },
            PhantomData,
        )
    }

    /// The default options with `oid_type` set: what
    /// [`Diff::from_buffer_with_oid_type`] passes libgit2.
    #[must_use]
    pub fn with_oid_type(oid_type: OidType) -> DiffParseOptions {
        let mut opts = DiffParseOptions::new();
        opts.as_mut().set_oid_type(Some(oid_type));
        opts
    }
}

impl Default for DiffParseOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The `git_diff_parse_options` value, for code that passes one by value.
impl From<DiffParseOptions> for ffi::git_diff_parse_options {
    fn from(opts: DiffParseOptions) -> ffi::git_diff_parse_options {
        opts.0
    }
}

impl DiffParseOptionsRef<'_> {
    /// The structure version. [`DiffParseOptions::new`] sets [`DiffParseOptions::VERSION`];
    /// libgit2 does not read it.
    ///
    /// Wraps `fld::git_diff_parse_options.version`
    #[doc(alias = "git_diff_parse_options.version")]
    #[must_use]
    pub fn version(&self) -> u32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let version_ptr = unsafe { &raw const (*self.as_ptr()).version };
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_diff_parse_options`; any value is a valid `u32`.
        unsafe { version_ptr.read() }
    }

    /// The object id type the patch's ids are parsed as; `None` for 0, which libgit2 reads
    /// as its default, [`OidType::Sha1`].
    ///
    /// Wraps `fld::git_diff_parse_options.oid_type`
    #[doc(alias = "git_diff_parse_options.oid_type")]
    #[must_use]
    pub fn oid_type(&self) -> Option<OidType> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let oid_type_ptr = unsafe { &raw const (*self.as_ptr()).oid_type };
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_diff_parse_options`; any value is a valid `git_oid_t`.
        let raw = unsafe { oid_type_ptr.read() };
        OidType::from_raw(raw)
    }

    /// A copy of the options, as a value of its own.
    #[must_use]
    pub fn to_options(&self) -> DiffParseOptions {
        let mut opts = DiffParseOptions::new();
        let mut m = opts.as_mut();
        m.set_version(self.version());
        m.set_oid_type(self.oid_type());
        opts
    }
}

impl DiffParseOptionsMut<'_> {
    /// Replaces the structure version. Any value is accepted, by this setter and by libgit2,
    /// which does not read it.
    ///
    /// Wraps `fld::git_diff_parse_options.version`
    #[doc(alias = "git_diff_parse_options.version")]
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let version_ptr = unsafe { &raw mut (*self.as_mut_ptr()).version };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_diff_parse_options`; the field is a plain integer.
        unsafe { version_ptr.write(version) }
    }

    /// Replaces the object id type; `None` stores 0, libgit2's default (SHA-1).
    ///
    /// Wraps `fld::git_diff_parse_options.oid_type`
    #[doc(alias = "git_diff_parse_options.oid_type")]
    pub fn set_oid_type(&mut self, oid_type: Option<OidType>) {
        let raw = oid_type.map_or(0, OidType::raw);
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the
        // projection forms no reference to it.
        let oid_type_ptr = unsafe { &raw mut (*self.as_mut_ptr()).oid_type };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_diff_parse_options`; the field is a plain integer, and `raw` is 0 or a named
        // type.
        unsafe { oid_type_ptr.write(raw) }
    }
}

impl fmt::Debug for DiffParseOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiffParseOptions")
            .field("version", &self.version())
            .field("oid_type", &self.oid_type())
            .finish()
    }
}

impl fmt::Debug for DiffParseOptionsMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for DiffParseOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<DiffParseOptionsRef<'b>> for DiffParseOptionsRef<'_> {
    /// Field by field.
    fn eq(&self, other: &DiffParseOptionsRef<'b>) -> bool {
        self.version() == other.version() && self.oid_type() == other.oid_type()
    }
}

impl Eq for DiffParseOptionsRef<'_> {}

impl Hash for DiffParseOptionsRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version().hash(state);
        self.oid_type().hash(state);
    }
}

impl PartialEq for DiffParseOptions {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for DiffParseOptions {}

impl Hash for DiffParseOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// A `DiffParseOptions` is a `git_diff_parse_options`, so C reads a value Rust holds in place.
const _: () = assert!(
    size_of::<DiffParseOptions>() == size_of::<ffi::git_diff_parse_options>()
        && align_of::<DiffParseOptions>() == align_of::<ffi::git_diff_parse_options>()
);

#[cfg(test)]
mod unit_tests {
    use std::collections::hash_map::DefaultHasher;

    use super::*;

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut h = DefaultHasher::new();
        value.hash(&mut h);
        h.finish()
    }

    #[test]
    fn new_spells_the_header_initializer() {
        const OPTS: DiffParseOptions = DiffParseOptions::new();
        let raw = ffi::git_diff_parse_options::from(OPTS);
        assert_eq!(raw.version, ffi::GIT_DIFF_PARSE_OPTIONS_VERSION);
        assert_eq!(raw.oid_type, 0);
        assert_eq!(OPTS, DiffParseOptions::default());
        assert_eq!(OPTS.as_ref().oid_type(), None);
        assert_eq!(OPTS.as_ref().version(), DiffParseOptions::VERSION);
    }

    #[test]
    fn setters_round_trip_through_the_layout() {
        let mut opts = DiffParseOptions::new();
        for kind in [Some(OidType::Sha1), Some(OidType::Sha256), None] {
            opts.as_mut().set_oid_type(kind);
            assert_eq!(opts.as_ref().oid_type(), kind);
            assert_eq!(
                ffi::git_diff_parse_options::from(opts).oid_type,
                kind.map_or(0, OidType::raw)
            );
        }
        for version in [0, 1, 2, u32::MAX] {
            let mut m = opts.as_mut();
            m.set_version(version);
            assert_eq!(m.as_ref().version(), version);
            assert_eq!(ffi::git_diff_parse_options::from(opts).version, version);
        }
    }

    #[test]
    fn with_oid_type_sets_only_the_type() {
        for kind in [OidType::Sha1, OidType::Sha256] {
            let opts = DiffParseOptions::with_oid_type(kind);
            assert_eq!(opts.as_ref().oid_type(), Some(kind));
            assert_eq!(opts.as_ref().version(), DiffParseOptions::VERSION);
        }
    }

    #[test]
    fn copies_compare_and_hash_field_by_field() {
        let mut a = DiffParseOptions::with_oid_type(OidType::Sha256);
        a.as_mut().set_version(7);
        let b = a.as_ref().to_options();
        assert_eq!(a, b);
        assert_eq!(a.as_ref(), b.as_ref());
        assert_eq!(hash_of(&a), hash_of(&b));
        assert_eq!(hash_of(&a.as_ref()), hash_of(&b.as_ref()));
        let mut c = b;
        c.as_mut().set_oid_type(None);
        assert_ne!(a, c);
        c.as_mut().set_oid_type(Some(OidType::Sha256));
        c.as_mut().set_version(1);
        assert_ne!(a, c);
    }

    #[test]
    fn debug_names_the_fields() {
        let mut opts = DiffParseOptions::with_oid_type(OidType::Sha1);
        let shown = format!("{opts:?}");
        assert_eq!(
            shown,
            "DiffParseOptions { version: 1, oid_type: Some(Sha1) }"
        );
        assert_eq!(format!("{:?}", opts.as_ref()), shown);
        assert_eq!(format!("{:?}", opts.as_mut()), shown);
    }
}
