//! Safe wrappers over `include/git2/sys/repository.h`.
//!
//! # Options for a repository without backends
//!
//! `git_repository_new_ext` takes a `git_repository_new_options` ([`NewOptions`]): a
//! structure version and the `git_oid_t` of the objects the new repository holds. It
//! only reads them, for the call: it refuses a version of 0 or above
//! [`NewOptions::VERSION`] with `GIT_ERROR_INVALID`, reads an `oid_type` of `0` as
//! `GIT_OID_DEFAULT`, and asserts that any other value is a type it names, which
//! aborts a debug build of libgit2 and fails with `-1` otherwise. So the version keeps
//! any value, and the object id type is an `Option<OidType>`: `None` is the zero
//! default, and no safe path stores a value [`OidType`] does not name.
//!
//! The options are a plain value, never allocated or released by libgit2.
//! [`NewOptions::new`] fills one through `git_repository_new_options_init`, as
//! `GIT_REPOSITORY_NEW_OPTIONS_INIT` does: the current version and the default type.
//! All-zero is not that default (libgit2 refuses version 0), so there is no `zeroed`.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::ptr::{addr_of, addr_of_mut};

use ffibox::define_ctype;

use crate::ffi;
use crate::libgit2::oid::oid::{OidType, UnnamedOidType};

define_ctype!(
    /// Options for a repository without backends (`git_repository_new_options`): the
    /// structure version, and the type of the object ids the repository holds.
    ///
    /// A plain value of 8 bytes, resource-free: `Copy`, held inline, never released.
    /// Build one with [`new`](Self::new) (or `Default`), then read and write its fields
    /// through [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut):
    ///
    /// ```
    /// use libgit2::libgit2::oid::oid::OidType;
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// # if cfg!(miri) { return; }
    /// let mut opts = NewOptions::new();
    /// assert_eq!(opts.as_ref().oid_type(), None);
    /// opts.as_mut().set_oid_type(Some(OidType::Sha256));
    /// assert_eq!(opts.as_ref().version(), NewOptions::VERSION);
    /// assert_eq!(opts.as_ref().oid_type(), Some(OidType::Sha256));
    /// ```
    ///
    /// A handle borrows the value, so it cannot outlive it:
    ///
    /// ```compile_fail,E0597
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// let handle = {
    ///     let opts = NewOptions::new();
    ///     opts.as_ref()
    /// };
    /// let _ = handle.oid_type();
    /// ```
    ///
    /// nor coexist with an exclusive one:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// let mut opts = NewOptions::new();
    /// let mut m = opts.as_mut();
    /// let r = opts.as_ref();
    /// m.set_oid_type(None);
    /// let _ = r.oid_type();
    /// ```
    ///
    /// and the shared handle only reads:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::sys::repository::NewOptions;
    ///
    /// let opts = NewOptions::new();
    /// opts.as_ref().set_oid_type(None);
    /// ```
    ///
    /// Wraps: git_repository_new_options
    #[derive(Clone, Copy)]
    NewOptions, NewOptionsRef, NewOptionsMut, ffi::git_repository_new_options
);

// SAFETY: a `NewOptions` is 8 plain bytes: no pointer, no interior mutability, no
// thread affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for NewOptions {}
// SAFETY: `NewOptionsMut` is an exclusive borrow of such a value, as `&mut NewOptions`.
unsafe impl Send for NewOptionsMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through
// `NewOptionsMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for NewOptions {}
// SAFETY: as above; `NewOptionsRef` is a shared borrow, as `&NewOptions`.
unsafe impl Send for NewOptionsRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for NewOptionsRef<'_> {}
// SAFETY: `&NewOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for NewOptionsMut<'_> {}

impl NewOptions {
    /// The structure version this crate's header declares
    /// (`GIT_REPOSITORY_NEW_OPTIONS_VERSION`), the only one libgit2 accepts.
    pub const VERSION: u32 = ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION;

    /// The default options, as `git_repository_new_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_REPOSITORY_NEW_OPTIONS_INIT` spells them:
    /// the current version and the default object id type ([`OidType::default`]).
    /// Needs no initialised libgit2.
    ///
    /// Wraps: git_repository_new_options_init
    pub fn new() -> NewOptions {
        let mut raw = ffi::git_repository_new_options::default();
        // SAFETY: `raw` is a live `git_repository_new_options` the call may write, so
        // the routine's non-NULL assertion holds. With the header's own version its
        // version check passes, so it only copies the 8-byte
        // `GIT_REPOSITORY_NEW_OPTIONS_INIT` template into `raw`: no allocation and no
        // error state, so no initialised runtime is needed.
        let rc = unsafe { ffi::git_repository_new_options_init(&mut raw, Self::VERSION) };
        assert_eq!(
            rc, 0,
            "git_repository_new_options_init refused its own version"
        );
        NewOptions(raw, PhantomData)
    }
}

impl Default for NewOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The `git_repository_new_options` value, for code that already holds one by value.
/// Any version is kept; an `oid_type` that is neither `0` nor a type [`OidType`] names
/// is refused, as libgit2 would assert on it.
impl TryFrom<ffi::git_repository_new_options> for NewOptions {
    type Error = UnnamedOidType;

    fn try_from(raw: ffi::git_repository_new_options) -> Result<NewOptions, UnnamedOidType> {
        if raw.oid_type != 0 {
            OidType::try_from(raw.oid_type)?;
        }
        Ok(NewOptions(raw, PhantomData))
    }
}

/// The `git_repository_new_options` value, for code that passes one by value.
impl From<NewOptions> for ffi::git_repository_new_options {
    fn from(opts: NewOptions) -> ffi::git_repository_new_options {
        opts.0
    }
}

impl NewOptionsRef<'_> {
    /// The structure version. [`NewOptions::new`] sets [`NewOptions::VERSION`];
    /// libgit2 refuses the options with `GIT_ERROR_INVALID` if it is 0 or above that.
    ///
    /// Field: git_repository_new_options.version
    pub fn version(&self) -> u32 {
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_repository_new_options`; any value is a valid `u32`.
        unsafe { addr_of!((*self.as_ptr()).version).read() }
    }

    /// The type of the repository's object ids; `None`, the default, means
    /// [`OidType::default`]. Every safe path stores `0` or a named type, so `None` is
    /// always the zero default.
    ///
    /// Field: git_repository_new_options.oid_type
    pub fn oid_type(&self) -> Option<OidType> {
        // SAFETY: a read through the shared handle's pointer to a live
        // `git_repository_new_options`; the field is a plain `u32`, any value valid.
        let raw = unsafe { addr_of!((*self.as_ptr()).oid_type).read() };
        OidType::from_raw(raw)
    }

    /// A copy of the options, as a value of its own.
    pub fn to_options(&self) -> NewOptions {
        // SAFETY: a read of the whole live `git_repository_new_options` through the
        // shared handle's pointer; it is `Copy` plain data.
        NewOptions(unsafe { self.as_ptr().read() }, PhantomData)
    }
}

impl NewOptionsMut<'_> {
    /// Replaces the structure version. Any value is accepted; libgit2 refuses options
    /// whose version is 0 or above [`NewOptions::VERSION`].
    ///
    /// Field: git_repository_new_options.version
    pub fn set_version(&mut self, version: u32) {
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_repository_new_options`; the field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).version).write(version) }
    }

    /// Replaces the type of the repository's object ids; `None` stores `0`, which
    /// libgit2 reads as [`OidType::default`].
    ///
    /// Field: git_repository_new_options.oid_type
    pub fn set_oid_type(&mut self, oid_type: Option<OidType>) {
        let raw = oid_type.map_or(0, OidType::raw);
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_repository_new_options`; the field is a plain integer, and `raw` is 0 or
        // a type libgit2 names.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).oid_type).write(raw) }
    }
}

impl fmt::Debug for NewOptionsRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewOptions")
            .field("version", &self.version())
            .field("oid_type", &self.oid_type())
            .finish()
    }
}

impl fmt::Debug for NewOptionsMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for NewOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<NewOptionsRef<'b>> for NewOptionsRef<'_> {
    /// Field by field.
    fn eq(&self, other: &NewOptionsRef<'b>) -> bool {
        self.version() == other.version() && self.oid_type() == other.oid_type()
    }
}

impl Eq for NewOptionsRef<'_> {}

impl Hash for NewOptionsRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.version().hash(state);
        self.oid_type().hash(state);
    }
}

impl PartialEq for NewOptions {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for NewOptions {}

impl Hash for NewOptions {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// A `NewOptions` is a `git_repository_new_options`, so C reads a value Rust holds in
// place.
const _: () = assert!(
    size_of::<NewOptions>() == size_of::<ffi::git_repository_new_options>()
        && align_of::<NewOptions>() == align_of::<ffi::git_repository_new_options>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// `GIT_REPOSITORY_NEW_OPTIONS_INIT`, built in Rust so Miri can run the accessors;
    /// `new_matches_the_template` ties it to `git_repository_new_options_init`.
    fn template() -> NewOptions {
        NewOptions(
            ffi::git_repository_new_options {
                version: ffi::GIT_REPOSITORY_NEW_OPTIONS_VERSION,
                oid_type: 0,
            },
            PhantomData,
        )
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn new_matches_the_template() {
        let options = NewOptions::new();
        assert_eq!((options.0.version, options.0.oid_type), (1, 0));
        assert_eq!(options, template());
        assert_eq!(NewOptions::default(), options);
    }

    #[test]
    fn defaults_read_as_documented() {
        let options = template();
        assert_eq!(options.as_ref().version(), NewOptions::VERSION);
        assert_eq!(options.as_ref().oid_type(), None);
        assert_eq!(
            format!("{options:?}"),
            "NewOptions { version: 1, oid_type: None }"
        );
    }

    #[test]
    fn each_setter_writes_its_field_only() {
        let mut options = template();
        options.as_mut().set_oid_type(Some(OidType::Sha256));
        assert_eq!(
            (options.0.version, options.0.oid_type),
            (1, ffi::GIT_OID_SHA256)
        );
        options.as_mut().set_version(0);
        assert_eq!(
            (options.0.version, options.0.oid_type),
            (0, ffi::GIT_OID_SHA256)
        );
        options.as_mut().set_oid_type(None);
        assert_eq!((options.0.version, options.0.oid_type), (0, 0));
    }

    #[test]
    fn try_from_keeps_named_types_and_refuses_the_rest() {
        for (raw, ok) in [
            (0, true),
            (ffi::GIT_OID_SHA1, true),
            (ffi::GIT_OID_SHA256, true),
            (3, false),
            (u32::MAX, false),
        ] {
            let value = ffi::git_repository_new_options {
                version: 9,
                oid_type: raw,
            };
            match NewOptions::try_from(value) {
                Ok(o) => {
                    assert!(ok, "{raw}");
                    assert_eq!((o.0.version, o.0.oid_type), (9, raw));
                    let back = ffi::git_repository_new_options::from(o);
                    assert_eq!((back.version, back.oid_type), (9, raw));
                }
                Err(e) => {
                    assert!(!ok, "{raw}");
                    assert_eq!(e.raw(), raw);
                }
            }
        }
    }

    #[test]
    fn getters_read_what_c_may_store() {
        // A struct C filled may hold an unnamed type, which reads as `None`.
        let mut options = template();
        options.0.oid_type = 3;
        assert_eq!(options.as_ref().oid_type(), None);
        assert_eq!(options.as_ref().to_options().0.oid_type, 3);
    }

    #[test]
    fn handles_reborrow_and_compare() {
        let mut options = template();
        let mut m = options.as_mut();
        m.as_mut().set_oid_type(Some(OidType::Sha1));
        assert_eq!(format!("{m:?}"), format!("{:?}", m.as_ref()));
        let other = template();
        assert_ne!(m.as_ref(), other.as_ref());
        assert_eq!(m.as_ref(), m.as_ref().to_options().as_ref());
        assert_eq!(options.as_ref().oid_type(), Some(OidType::Sha1));
    }
}
