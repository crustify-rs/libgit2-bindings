//! Safe wrappers over `include/git2/oid.h`.
//!
//! libgit2 passes an object id's type (`git_oid_t`) by value: it names the hash
//! function behind the id, which fixes the id's size. Values the header does not name
//! travel through it too: a zeroed `oid_type` field of an options struct means "the
//! default type" ([`OidType::default`]), and `git_oid.type` stores whatever byte it was
//! given. So a raw value is converted fallibly ([`OidType::from_raw`]) and never
//! transmuted.
//!
//! An [`Oid`] is a `git_oid`: a one-byte object-id type (an [`OidType`] value, or 0
//! for none) and 32 bytes of raw id, of which SHA1 uses the first 20. It owns nothing, so
//! it is held by value, copied freely and never released; libgit2 hands out
//! `const git_oid *` into its own objects, which become [`OidRef`] handles. Every byte
//! pattern is a valid `git_oid` to libgit2: `oid.c` maps an unknown type to a size of
//! 0 and reports it ("unknown type"), and the all-zero oid is its own `GIT_OID_NONE`.
//! A safe wrapper taking an `Oid` must therefore accept any type byte.
//!
//! An [`OidShortener`] is a `git_oid_shorten`: an opaque C object that finds the
//! shortest prefix length telling apart a set of hex object ids. libgit2 allocates it
//! (`git_oid_shorten_new`) and frees it (`git_oid_shorten_free`), so Rust owns it
//! through an [`OidShortenerBox`]. Its trie keeps pointers into the text ids it is
//! given (`git_oid_shorten_add` stores the unread tail of each), which the header does
//! not say: the shortener carries the lifetime `'s` of those strings, and must not
//! outlive any of them.
//!
//! Oids compare as libgit2 compares them: `Eq` and `Ord` over all 33 bytes, type first
//! (`memcmp`), and [`OidRef::eq_prefix`], [`OidRef::cmp_hex`] and [`OidRef::eq_hex`]
//! against a prefix of another oid or a hex string. They format as lowercase hex into
//! a caller's buffer, whose type or length bounds what libgit2 writes. libgit2's own
//! string, `git_oid_tostr_s`, is a per-thread buffer that the next call on the thread
//! overwrites, so [`OidRef::to_hex_string`] copies it out. An oid whose type libgit2
//! does not know has no hex form: formatting one fails.
//!
//! The constructors, the formatters and [`OidShortenerMut::add`] can fail and set the
//! calling thread's libgit2 error, so they pin libgit2's runtime first
//! (`runtime_pinned`) and return the error, copied out, as an [`ErrorBuf`].

use core::cmp::Ordering;
use core::ffi::{CStr, c_int};
use core::fmt;
use core::marker::PhantomData;
use core::ptr::{NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop, CSlice, CSliceMut, CZeroable, define_ctype};

use crate::ffi;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set};

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

/// The length of `git_oid.id`, `GIT_OID_MAX_SIZE`.
const MAX_SIZE: usize = ffi::GIT_OID_MAX_SIZE as usize;

/// The length of the longest hex form of an id, `GIT_OID_MAX_HEXSIZE`.
const MAX_HEX_SIZE: usize = ffi::GIT_OID_MAX_HEXSIZE as usize;

define_ctype!(
    /// The id of a git object: a commit, tree, blob or tag (`git_oid`).
    ///
    /// A plain value of 33 bytes, resource-free: `Copy`, held inline, never released.
    /// Parse one with [`from_string`](Self::from_string),
    /// [`from_prefix`](Self::from_prefix) or [`from_raw`](Self::from_raw), or start
    /// from [`Oid::zeroed`](ffibox::CZeroable::zeroed), libgit2's `GIT_OID_NONE`. Read
    /// and write its fields through [`as_ref`](Self::as_ref) /
    /// [`as_mut`](Self::as_mut).
    ///
    /// Wraps: git_oid
    #[derive(Clone, Copy)]
    Oid, OidRef, OidMut, ffi::git_oid
);

// SAFETY: all zero is libgit2's `GIT_OID_NONE`: type 0, which every `oid.c` routine
// maps to size 0 and rejects or ignores, and an all-zero id. The getters read plain
// bytes, and there is no teardown.
unsafe impl CZeroable for Oid {}

// SAFETY: an `Oid` is 33 plain bytes: no pointer, no interior mutability, no thread
// affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for Oid {}
// SAFETY: `OidMut` is an exclusive borrow of such a value, as `&mut Oid`.
unsafe impl Send for OidMut<'_> {}

// SAFETY: shared access only reads the bytes (every write goes through `OidMut`, which
// excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for Oid {}
// SAFETY: as above; `OidRef` is a shared borrow, as `&Oid`.
unsafe impl Send for OidRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for OidRef<'_> {}
// SAFETY: `&OidMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for OidMut<'_> {}

impl Oid {
    /// Parses the hex id `s` of type `kind`: exactly 40 characters for SHA1, 64 for
    /// SHA256, in either case.
    ///
    /// Fails, with libgit2's error, for a non-hex character or a string of another
    /// length.
    ///
    /// Wraps: git_oid_from_string
    pub fn from_string(s: &CStr, kind: OidType) -> Result<Oid, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut oid = Oid::zeroed();
        // SAFETY: `out` addresses a live `git_oid` in `oid`, written in place; `s` is a
        // NUL-terminated string, read up to its first non-hex character (its NUL at
        // the latest) and, only when the first hexsize are all hex, at index hexsize,
        // which is then within it. Neither is retained; `kind` is a named type. The
        // runtime is pinned, so a failure is reported on live thread-local state.
        let rc =
            unsafe { ffi::git_oid_from_string(oid.as_mut().as_mut_ptr(), s.as_ptr(), kind.raw()) };
        parsed(rc, oid)
    }

    /// Parses the hex prefix `prefix` of an id of type `kind`, zeroing the rest; an
    /// odd-length prefix leaves the last byte's low nibble zero.
    ///
    /// `prefix` need not be NUL-terminated. Fails, with libgit2's error, for an empty
    /// prefix, one longer than the type's [`hex_size`](OidType::hex_size), or a non-hex
    /// character.
    ///
    /// Wraps: git_oid_from_prefix
    pub fn from_prefix(prefix: &[u8], kind: OidType) -> Result<Oid, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut oid = Oid::zeroed();
        // SAFETY: `out` addresses a live `git_oid` in `oid`, written in place; `str` is
        // non-NULL and valid for reads of `len` bytes, the slice's (libgit2 reads at
        // most `len`, and none when `len` is 0). Neither is retained; `kind` is a named
        // type. The runtime is pinned.
        let rc = unsafe {
            ffi::git_oid_from_prefix(
                oid.as_mut().as_mut_ptr(),
                prefix.as_ptr().cast(),
                prefix.len(),
                kind.raw(),
            )
        };
        parsed(rc, oid)
    }

    /// The id of type `kind` whose raw bytes are `raw`: exactly
    /// [`kind.size()`](OidType::size) of them, 20 for SHA1 and 32 for SHA256.
    ///
    /// C reads as many bytes as the type needs from a pointer with no length; a `raw` of
    /// another length is refused here before the call, with a `GIT_ERROR_INVALID`
    /// "unable to parse OID - invalid raw length" error set on the thread as libgit2
    /// sets its own.
    ///
    /// Wraps: git_oid_from_raw
    pub fn from_raw(raw: &[u8], kind: OidType) -> Result<Oid, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if raw.len() != kind.size() {
            set(
                ErrorClass::Invalid,
                c"unable to parse OID - invalid raw length",
            );
            return Err(last());
        }
        let mut oid = Oid::zeroed();
        // SAFETY: `out` addresses a live `git_oid` in `oid`, written in place; libgit2
        // reads `git_oid_size(kind)` bytes of `raw`, `kind.size()`, which is exactly the
        // slice's length (checked above). Nothing is retained. The runtime is pinned.
        let rc =
            unsafe { ffi::git_oid_from_raw(oid.as_mut().as_mut_ptr(), raw.as_ptr(), kind.raw()) };
        parsed(rc, oid)
    }
}

/// `oid` when the parser returned 0; libgit2's error otherwise.
fn parsed(rc: c_int, oid: Oid) -> Result<Oid, ErrorBuf> {
    if rc == 0 { Ok(oid) } else { Err(last()) }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a> OidRef<'a> {
    /// The type of the id; `None` for 0 (no type, as in a zeroed oid) or any other
    /// byte the header does not name, which [`raw_kind`](Self::raw_kind) has.
    ///
    /// Field: git_oid.type
    pub fn kind(&self) -> Option<OidType> {
        OidType::from_raw(self.raw_kind().into())
    }

    /// The type of the id as libgit2 stores it, a `git_oid_t` value in one byte:
    /// whatever byte C left there, named or not.
    ///
    /// Field: git_oid.type
    pub fn raw_kind(&self) -> u8 {
        // SAFETY: a read through the shared handle's pointer to a live `git_oid`; any
        // byte is a valid `u8`.
        unsafe { addr_of!((*self.as_ptr()).type_).read() }
    }

    /// The raw id bytes, copied out: all 32, of which a SHA1 id uses the first 20 and
    /// libgit2's parsers zero the rest.
    ///
    /// Field: git_oid.id
    pub fn id(&self) -> [u8; MAX_SIZE] {
        // SAFETY: a read through the shared handle's pointer to a live `git_oid`; every
        // byte array is valid.
        unsafe { addr_of!((*self.as_ptr()).id).read() }
    }

    /// The raw id bytes, borrowed in place as a run of 32 for as long as the oid is.
    ///
    /// Field: git_oid.id
    pub fn id_bytes(&self) -> CSlice<'a, u8> {
        // SAFETY: the field projected from the shared handle's pointer to a live
        // `git_oid` is 32 initialised bytes, non-NULL, which outlive `'a` with it; the
        // run reads them only, as the handle does.
        unsafe {
            let id = addr_of!((*self.as_ptr()).id).cast::<u8>().cast_mut();
            CSlice::from_raw_parts(NonNull::new_unchecked(id), MAX_SIZE)
        }
    }

    /// A copy of the oid, as a value of its own.
    ///
    /// Wraps: git_oid_cpy
    pub fn to_oid(&self) -> Oid {
        let mut oid = Oid::zeroed();
        oid.as_mut().copy_from(*self);
        oid
    }
}

impl fmt::Debug for OidRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        /// The named type, or the raw byte when the header does not name it.
        struct Kind(u8);
        impl fmt::Debug for Kind {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match OidType::from_raw(self.0.into()) {
                    Some(kind) => kind.fmt(f),
                    None => self.0.fmt(f),
                }
            }
        }
        struct Hex([u8; MAX_SIZE]);
        impl fmt::Debug for Hex {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
            }
        }
        f.debug_struct("Oid")
            .field("kind", &Kind(self.raw_kind()))
            .field("id", &Hex(self.id()))
            .finish()
    }
}

impl OidRef<'_> {
    /// Whether `other` has the same type and the same first `len` hex digits; an odd
    /// `len` compares the high nibble of the last byte. A `len` beyond
    /// `GIT_OID_MAX_HEXSIZE` compares all 32 id bytes, and 0 only the types.
    ///
    /// Wraps: git_oid_ncmp
    pub fn eq_prefix(&self, other: OidRef<'_>, len: usize) -> bool {
        // SAFETY: two shared handles to live `git_oid`s, possibly the same one, read for
        // the call only; libgit2 caps `len` at `GIT_OID_MAX_HEXSIZE` digits, so it reads
        // at most the 32 id bytes of each.
        unsafe { ffi::git_oid_ncmp(self.as_ptr(), other.as_ptr(), len) == 0 }
    }

    /// Whether all 32 id bytes are zero, whatever the type byte holds.
    ///
    /// Wraps: git_oid_is_zero
    pub fn is_zero(&self) -> bool {
        // SAFETY: a shared handle to a live `git_oid`, of which libgit2 reads the 32 id
        // bytes for the call only.
        unsafe { ffi::git_oid_is_zero(self.as_ptr()) != 0 }
    }

    /// How this id sorts against the hex digits of `hex`, compared byte by byte.
    ///
    /// libgit2 compares only as many digits as both have: `hex` may be a prefix (the
    /// empty string is `Equal` to every id), digits past the id's
    /// [`hex_size`](OidType::hex_size) are ignored, an odd last digit is compared as a
    /// byte whose low nibble is zero, and an oid of a type libgit2 does not know is
    /// `Equal` to everything. A non-hex digit it reaches reads as `Less`: C reports it as
    /// -1, which a byte difference of -1 also is. Check the string first, with
    /// [`Oid::from_prefix`], where that matters.
    ///
    /// Wraps: git_oid_strcmp
    pub fn cmp_hex(&self, hex: &CStr) -> Ordering {
        // SAFETY: a shared handle to a live `git_oid`, of which libgit2 reads at most the
        // id's size in bytes; `hex` is NUL-terminated, and libgit2 checks for the NUL
        // before each digit it reads. Neither is retained.
        let rc = unsafe { ffi::git_oid_strcmp(self.as_ptr(), hex.as_ptr()) };
        rc.cmp(&0)
    }

    /// Whether [`cmp_hex`](Self::cmp_hex) finds `hex` equal to this id: `hex` is a hex
    /// prefix of it (or longer, its extra digits ignored).
    ///
    /// Wraps: git_oid_streq
    pub fn eq_hex(&self, hex: &CStr) -> bool {
        // SAFETY: as in `cmp_hex`: libgit2 reads at most the id's bytes, and `hex` up to
        // its NUL; neither is retained.
        unsafe { ffi::git_oid_streq(self.as_ptr(), hex.as_ptr()) == 0 }
    }

    /// Formats the id as its [`hex_size`](OidType::hex_size) lowercase hex digits at the
    /// start of `out`, and returns them; the rest of `out` is left as it was.
    ///
    /// Fails, with libgit2's error, for an oid of a type libgit2 does not know, writing
    /// nothing.
    ///
    /// Wraps: git_oid_fmt
    pub fn format<'b>(&self, out: &'b mut [u8; MAX_HEX_SIZE]) -> Result<&'b str, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        // SAFETY: `out` is valid for writes of `GIT_OID_MAX_HEXSIZE` bytes, the most any
        // type's hex form takes, and libgit2 writes that form only (none for an unknown
        // type); the oid is a live `git_oid` read for the call. Neither is retained. The
        // runtime is pinned, so an unknown type is reported on live thread-local state.
        let rc = unsafe { ffi::git_oid_fmt(out.as_mut_ptr().cast(), self.as_ptr()) };
        if rc != 0 {
            return Err(last());
        }
        Ok(hex_str(&out[..self.hex_len()]))
    }

    /// Formats the first `out.len()` hex digits of the id into `out`, and returns them;
    /// past the id's [`hex_size`](OidType::hex_size), `out` is zero-filled instead, and
    /// the digits returned are all of them. Nothing is NUL-terminated otherwise.
    ///
    /// Fails, with libgit2's error, for an oid of a type libgit2 does not know, writing
    /// nothing.
    ///
    /// Wraps: git_oid_nfmt
    pub fn format_prefix<'b>(&self, out: &'b mut [u8]) -> Result<&'b str, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        // SAFETY: `out` is valid for writes of `out.len()` bytes, the count passed, and
        // libgit2 writes no further (none for an unknown type); the oid is a live
        // `git_oid` read for the call. Neither is retained. The runtime is pinned.
        let rc = unsafe { ffi::git_oid_nfmt(out.as_mut_ptr().cast(), out.len(), self.as_ptr()) };
        if rc != 0 {
            return Err(last());
        }
        let digits = out.len().min(self.hex_len());
        Ok(hex_str(&out[..digits]))
    }

    /// Formats the id as a loose-object path, `aa/bbbb…`, at the start of `out`: its
    /// first two hex digits, a `/`, then the rest, one byte longer than its
    /// [`hex_size`](OidType::hex_size). Returns it; the rest of `out` is left as it was.
    ///
    /// Fails, with libgit2's error, for an oid of a type libgit2 does not know, writing
    /// nothing.
    ///
    /// Wraps: git_oid_pathfmt
    pub fn format_path<'b>(
        &self,
        out: &'b mut [u8; MAX_HEX_SIZE + 1],
    ) -> Result<&'b str, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        // SAFETY: `out` is valid for writes of `GIT_OID_MAX_HEXSIZE + 1` bytes, the
        // longest path form, and libgit2 writes that form only (none for an unknown
        // type); the oid is a live `git_oid` read for the call. Neither is retained. The
        // runtime is pinned.
        let rc = unsafe { ffi::git_oid_pathfmt(out.as_mut_ptr().cast(), self.as_ptr()) };
        if rc != 0 {
            return Err(last());
        }
        Ok(hex_str(&out[..self.hex_len() + 1]))
    }

    /// Formats the id into `out` as a NUL-terminated string, truncated to
    /// `out.len() - 1` digits when `out` is shorter than its
    /// [`hex_size`](OidType::hex_size) plus the NUL, and returns it. An empty `out`
    /// gives the empty string, writing nothing.
    ///
    /// Fails, with libgit2's error, for an oid of a type libgit2 does not know, having
    /// written only a NUL at the start of `out`: C returns that empty string and leaves
    /// the error on the thread.
    ///
    /// Wraps: git_oid_tostr
    pub fn format_cstr<'b>(&self, out: &'b mut [u8]) -> Result<&'b CStr, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        // SAFETY: `out` is valid for writes of `out.len()` bytes, the count passed, and
        // libgit2 writes no further (nothing when it is 0, when it returns its static
        // empty string instead); the oid is a live `git_oid` read for the call. Neither
        // is retained. The runtime is pinned.
        let rc = unsafe { ffi::git_oid_tostr(out.as_mut_ptr().cast(), out.len(), self.as_ptr()) };
        if rc.cast_const() != out.as_ptr().cast() {
            return Ok(c"");
        }
        if self.kind().is_none() {
            return Err(last());
        }
        Ok(
            CStr::from_bytes_until_nul(out)
                .expect("git_oid_tostr NUL-terminates within its buffer"),
        )
    }

    /// The id's hex form, formatted by libgit2 into its per-thread string and copied out
    /// before that is overwritten.
    ///
    /// Fails, with libgit2's error, when libgit2 cannot allocate the string, and for an
    /// oid of a type libgit2 does not know: C formats nothing then, and returns the
    /// thread's string as the last call left it, unwritten and unterminated on the
    /// thread's first call. It is refused here before the call instead, with the
    /// `GIT_ERROR_INVALID` "unable to parse OID - unknown type" error C sets.
    ///
    /// Wraps: git_oid_tostr_s
    pub fn to_hex_string(&self) -> Result<String, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if self.kind().is_none() {
            set(ErrorClass::Invalid, c"unable to parse OID - unknown type");
            return Err(last());
        }
        // SAFETY: the oid is a live `git_oid` of a known type, read for the call, which
        // libgit2 formats with its NUL into the thread's string of
        // `GIT_OID_MAX_HEXSIZE + 1` bytes. The runtime is pinned, so that string's TLS
        // key is live and no shutdown frees it.
        let s = unsafe { ffi::git_oid_tostr_s(self.as_ptr()) };
        if s.is_null() {
            return Err(last());
        }
        // SAFETY: `s` is this thread's NUL-terminated string, which only a libgit2 call
        // on this thread, or its exit, changes or frees; it is copied out before either.
        let hex = unsafe { CStr::from_ptr(s) };
        Ok(hex_str(hex.to_bytes()).to_owned())
    }

    /// The number of hex digits libgit2 formats the id with: its type's
    /// [`hex_size`](OidType::hex_size), 0 for a type libgit2 does not know.
    fn hex_len(&self) -> usize {
        self.kind().map_or(0, OidType::hex_size)
    }
}

/// The hex digits libgit2 formatted, as text.
fn hex_str(digits: &[u8]) -> &str {
    core::str::from_utf8(digits).expect("libgit2 formats ids as ASCII hex digits")
}

impl<'b> PartialEq<OidRef<'b>> for OidRef<'_> {
    /// Whether the two oids are byte for byte the same, type included.
    ///
    /// Wraps: git_oid_equal
    fn eq(&self, other: &OidRef<'b>) -> bool {
        // SAFETY: two shared handles to live `git_oid`s, possibly the same one, compared
        // with `memcmp` for the call only.
        unsafe { ffi::git_oid_equal(self.as_ptr(), other.as_ptr()) != 0 }
    }
}

impl Eq for OidRef<'_> {}

impl PartialOrd for OidRef<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OidRef<'_> {
    /// Orders the two oids as libgit2 does: `memcmp` over all 33 bytes, so by type
    /// first, then by raw id.
    ///
    /// Wraps: git_oid_cmp
    fn cmp(&self, other: &Self) -> Ordering {
        // SAFETY: two shared handles to live `git_oid`s, possibly the same one, compared
        // with `memcmp` for the call only.
        let rc = unsafe { ffi::git_oid_cmp(self.as_ptr(), other.as_ptr()) };
        rc.cmp(&0)
    }
}

impl PartialEq for Oid {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for Oid {}

impl PartialOrd for Oid {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Oid {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(&other.as_ref())
    }
}

impl OidMut<'_> {
    /// Replaces the type.
    ///
    /// Field: git_oid.type
    pub fn set_kind(&mut self, kind: OidType) {
        // Every `git_oid_t` constant fits the byte libgit2 stores it in.
        self.set_raw_kind(kind.raw() as u8);
    }

    /// Replaces the type byte. Any value is accepted: libgit2 rejects or ignores a type
    /// it does not know.
    ///
    /// Field: git_oid.type
    pub fn set_raw_kind(&mut self, kind: u8) {
        // SAFETY: a write through the exclusive handle's pointer to a live `git_oid`;
        // the field is a plain byte.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).type_).write(kind) }
    }

    /// Replaces all 32 raw id bytes.
    ///
    /// Field: git_oid.id
    pub fn set_id(&mut self, id: [u8; MAX_SIZE]) {
        // SAFETY: a write through the exclusive handle's pointer to a live `git_oid`;
        // the field is plain bytes.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).id).write(id) }
    }

    /// The raw id bytes, borrowed exclusively in place as a run of 32, for editing
    /// some of them.
    ///
    /// Field: git_oid.id
    pub fn id_mut(&mut self) -> CSliceMut<'_, u8> {
        // SAFETY: the field projected from the exclusive handle's pointer to a live
        // `git_oid` is 32 initialised bytes, non-NULL; the run borrows this handle
        // mutably, so nothing else reaches them while it lives.
        unsafe {
            let id = addr_of_mut!((*self.as_mut_ptr()).id).cast::<u8>();
            CSliceMut::from_raw_parts(NonNull::new_unchecked(id), MAX_SIZE)
        }
    }

    /// Overwrites this oid with a copy of `src`, type and all 32 id bytes.
    ///
    /// `src` cannot be this oid, which C's `memcpy` would not allow either: it is
    /// borrowed exclusively here.
    ///
    /// ```compile_fail,E0502
    /// use ffibox::CZeroable;
    /// use libgit2::libgit2::oid::oid::Oid;
    ///
    /// let mut oid = Oid::zeroed();
    /// let src = oid.as_ref();
    /// oid.as_mut().copy_from(src);
    /// ```
    ///
    /// Wraps: git_oid_cpy
    pub fn copy_from(&mut self, src: OidRef<'_>) {
        // SAFETY: `out` is the exclusive handle's pointer to a live `git_oid` and `src`
        // a shared handle's to another live one: the exclusive borrow rules out their
        // being the same object, so the whole-struct `memcpy` does not overlap. Neither
        // is retained; the copy always succeeds.
        let rc = unsafe { ffi::git_oid_cpy(self.as_mut_ptr(), src.as_ptr()) };
        debug_assert_eq!(rc, 0);
    }
}

/// An OID shortener (`git_oid_shorten`): the shortest prefix length that tells apart
/// every hex id added to it, as `git log --abbrev` prints.
///
/// libgit2 allocates and frees it, so it is only ever reached through an
/// [`OidShortenerBox`], built by [`new`](Self::new), and its handles. It is opaque: no
/// field is published.
///
/// Its trie keeps pointers into the text ids added to it, read again by later
/// additions, so `'s` is the lifetime of those strings: a shortener lives no longer
/// than any of them. The exclusive handle is invariant in `'s`, so one cannot be
/// shrunk to accept a shorter-lived string than its owner was built for:
///
/// ```compile_fail
/// use libgit2::libgit2::oid::oid::OidShortenerMut;
///
/// fn shrink<'a, 's>(m: OidShortenerMut<'a, 'static>) -> OidShortenerMut<'a, 's> {
///     m
/// }
/// ```
///
/// Wraps: git_oid_shorten
/// Handwritten
#[repr(transparent)]
pub struct OidShortener<'s>(ffi::git_oid_shorten, PhantomData<(*const (), &'s [u8])>);

/// Shared borrow of an [`OidShortener`]. `Copy`, like `&T`.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct OidShortenerRef<'a, 's>(CBorrowedPtr<'a, OidShortener<'s>>);

/// Exclusive borrow of an [`OidShortener`]. Reaches the shared handle with
/// [`as_ref`](Self::as_ref).
///
/// Handwritten
#[repr(transparent)]
pub struct OidShortenerMut<'a, 's>(
    OidShortenerRef<'a, 's>,
    // Invariant in the shortener, and so in `'s`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut OidShortener<'s>>,
);

// SAFETY: `OidShortener` is `#[repr(transparent)]` over `git_oid_shorten` with a
// zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, OidShortener<'s>>` (`OidShortenerMut` through `OidShortenerRef`)
// with zero-sized markers and no `Drop`, hand out no reference to the shortener, and
// `OidShortenerRef` has no write operation. `OidShortenerMut` is invariant in the
// shortener through its `PhantomData<&'a mut OidShortener<'s>>`.
unsafe impl<'s> CCell for OidShortener<'s> {
    type C = ffi::git_oid_shorten;
    type Ref<'a>
        = OidShortenerRef<'a, 's>
    where
        Self: 'a;
    type Mut<'a>
        = OidShortenerMut<'a, 's>
    where
        Self: 'a;
}

// SAFETY: a shortener is a `git__calloc`'d struct and its trie buffer, both from
// libgit2's process-wide allocator, which frees on any thread, plus borrows of the
// text ids added to it, which are `&'s [u8]` and so may be sent. No C routine keeps
// thread-local state about it: the errors `git_oid_shorten_add` sets go to the calling
// thread.
unsafe impl Send for OidShortener<'_> {}
// SAFETY: no operation reaches a shortener through a shared handle — libgit2 has no
// routine taking a `const git_oid_shorten *` — so shared access reads nothing.
unsafe impl Sync for OidShortener<'_> {}

impl<'a, 's> OidShortenerRef<'a, 's> {
    /// Read-only pointer to the C object.
    #[allow(dead_code)]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_oid_shorten {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'a, 's> OidShortenerMut<'a, 's> {
    /// Writable pointer to the C object, for `git_oid_shorten_add`.
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_oid_shorten {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> OidShortenerRef<'_, 's> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> OidShortenerMut<'_, 's> {
        OidShortenerMut(self.0, PhantomData)
    }

    /// Adds the hex id `id` to the set, and returns the shortest prefix length, in hex
    /// digits, that tells apart every id added so far; never below the shortener's
    /// minimum length, and 41 once an id is added twice.
    ///
    /// libgit2 reads the first 40 digits of `id` (a SHA1 id's length) and no further: a
    /// longer id, such as a SHA256 one, is shortened on those. It keeps pointers into
    /// them for later additions to read, so `id` is borrowed for the shortener's `'s`.
    ///
    /// Fails, with libgit2's error:
    ///
    /// - when `id` does not start with 40 hex digits. C validates only the digits it
    ///   walks before finding a free slot, and keeps the rest for a later addition to
    ///   index its trie with, writing outside a node for a non-hex one; such an `id` is
    ///   refused here before the call instead, with the `GIT_ERROR_INVALID` "unable to
    ///   shorten OID - invalid hex value" error C sets for a non-hex digit it walks;
    /// - once the trie is full (`SHRT_MAX` nodes, some 20,000 to 30,000 ids), with
    ///   "unable to shorten OID - OID set full"; every later addition fails the same way.
    ///
    /// # Aborts
    ///
    /// When libgit2 cannot grow the trie, C keeps NULL in its place, which the next
    /// addition dereferences; only freeing the shortener stays safe, which a borrowed
    /// handle cannot enforce. The process is aborted instead, as Rust aborts on an
    /// allocation failure.
    ///
    /// Wraps: git_oid_shorten_add
    pub fn add(&mut self, id: &'s str) -> Result<usize, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if !shortenable(id) {
            set(
                ErrorClass::Invalid,
                c"unable to shorten OID - invalid hex value",
            );
            return Err(last());
        }
        // SAFETY: the exclusive handle's pointer to a live shortener, which nothing else
        // reaches during the call. libgit2 reads `id[..40]`, all hex digits (checked
        // above), and keeps pointers into `id[1..=40]` (the last one past the end, never
        // read), which outlive the shortener: `id` is borrowed for `'s`, which this
        // handle is invariant in. The runtime is pinned.
        let rc = unsafe { ffi::git_oid_shorten_add(self.as_mut_ptr(), id.as_ptr().cast()) };
        if let Ok(len) = usize::try_from(rc) {
            return Ok(len);
        }
        let error = last();
        // Only a full trie stays usable; any other failure is the lost trie, recorded as
        // libgit2's out-of-memory error or not at all.
        if error.class() == Some(ErrorClass::Invalid)
            && error.message() == c"unable to shorten OID - OID set full"
        {
            return Err(error);
        }
        eprintln!("git_oid_shorten_add lost its trie to an allocation failure; aborting");
        std::process::abort()
    }
}

/// The digits of an id that `git_oid_shorten_add` reads: a SHA1 id's.
const SHORTEN_DIGITS: usize = ffi::GIT_OID_SHA1_HEXSIZE as usize;

/// Whether `id` starts with the [`SHORTEN_DIGITS`] hex digits `git_oid_shorten_add`
/// reads, the alphabet of libgit2's `git__fromhex`.
fn shortenable(id: &str) -> bool {
    id.as_bytes()
        .get(..SHORTEN_DIGITS)
        .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
}

impl fmt::Debug for OidShortenerRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OidShortener").finish_non_exhaustive()
    }
}

impl fmt::Debug for OidShortenerMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a shortener through `git_oid_shorten_free`, which frees its trie and then
/// the struct, and never touches the text ids it borrowed.
///
/// Wraps: git_oid_shorten_free
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct OidShortenerFree;

// SAFETY: `git_oid_shorten_free` releases the shortener and its trie exactly once,
// whatever state `git_oid_shorten_add` left them in (a failed add leaves the trie
// NULL, which `git__free` accepts), and needs no state beyond the pointer.
unsafe impl<'s> CDrop<OidShortener<'s>> for OidShortenerFree {
    unsafe fn c_drop(&self, ptr: NonNull<OidShortener<'s>>) {
        // SAFETY: per `c_drop`'s contract, `ptr` addresses a live shortener whose
        // ownership the caller gives up; `OidShortener` is transparent over
        // `git_oid_shorten`.
        unsafe { ffi::git_oid_shorten_free(ptr.as_ptr().cast()) }
    }
}

/// An owned [`OidShortener`], freed by `git_oid_shorten_free` on drop.
pub type OidShortenerBox<'s> = CBox<OidShortener<'s>, OidShortenerFree>;

impl<'s> OidShortener<'s> {
    /// A new, empty shortener whose results are never below `min_length`, even where a
    /// shorter prefix would be unique.
    ///
    /// Fails, with libgit2's error, when libgit2 runs out of memory or `min_length` does
    /// not survive a round trip through a C `int`, as libgit2 requires: it must be at
    /// most `i32::MAX`, or the sign extension of a negative `int` (`usize::MAX` reads
    /// as -1, which C stores and later reports as the minimum). C rejects any other
    /// length with an assertion, which aborts the process in a debug build of libgit2;
    /// it is refused here before the call instead, with the `GIT_ERROR_INVALID` error a
    /// release build of libgit2 sets.
    ///
    /// Wraps: git_oid_shorten_new
    pub fn new(min_length: usize) -> Result<OidShortenerBox<'s>, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        // C's `(size_t)((int)min_length) == min_length`: `as` truncates, then
        // sign-extends back.
        if min_length as c_int as usize != min_length {
            set(
                ErrorClass::Invalid,
                c"invalid argument: '(size_t)((int)min_length) == min_length'",
            );
            return Err(last());
        }
        // SAFETY: `min_length` round-trips through an `int`, as libgit2 asserts; the
        // result is NULL or a fresh shortener the caller owns. The runtime is pinned, so a failure is
        // reported on live thread-local state.
        let shortener = unsafe { ffi::git_oid_shorten_new(min_length) };
        // SAFETY: NULL, or a fresh shortener nothing else references, released by
        // `git_oid_shorten_free` as `OidShortenerFree` does. It holds no text id yet,
        // so any `'s` is met.
        unsafe { OidShortenerBox::from_c(shortener) }.ok_or_else(last)
    }
}

// An `Oid` is a `git_oid`, so C reads and writes a value Rust holds in place.
const _: () = assert!(
    size_of::<Oid>() == size_of::<ffi::git_oid>()
        && align_of::<Oid>() == align_of::<ffi::git_oid>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use core::mem::size_of;
    use std::ffi::CString;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

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

    static REFUSING: AtomicBool = AtomicBool::new(false);
    static REFUSING_REALLOC: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut core::ffi::c_void;
        fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
        fn free(p: *mut core::ffi::c_void);
    }

    /// libc's allocator, refusing everything while `REFUSING` is set, and resizes
    /// alone while `REFUSING_REALLOC` is.
    struct Refusing;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if REFUSING.load(SeqCst) {
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
            if REFUSING.load(SeqCst) || REFUSING_REALLOC.load(SeqCst) {
                return None;
            }
            let p = p.map_or(core::ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            NonNull::new(unsafe { realloc(p, len) }.cast())
        }
        unsafe fn free(p: NonNull<u8>) {
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Installs [`Refusing`] as libgit2's allocator, before anything is allocated.
    fn install_refusing() {
        let allocator = Allocator::new::<Refusing>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    const SHA1_HEX: &CStr = c"a94a8fe5ccb19ba61c4c0873d391e987982fbbd3";
    const SHA1_RAW: [u8; 20] = [
        0xa9, 0x4a, 0x8f, 0xe5, 0xcc, 0xb1, 0x9b, 0xa6, 0x1c, 0x4c, 0x08, 0x73, 0xd3, 0x91, 0xe9,
        0x87, 0x98, 0x2f, 0xbb, 0xd3,
    ];

    fn padded(raw: &[u8]) -> [u8; MAX_SIZE] {
        let mut id = [0; MAX_SIZE];
        id[..raw.len()].copy_from_slice(raw);
        id
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<Oid>(), 33);
        assert_eq!(size_of::<OidRef<'_>>(), size_of::<*const ffi::git_oid>());
        assert_eq!(size_of::<OidMut<'_>>(), size_of::<*const ffi::git_oid>());
        assert_eq!(size_of::<OidShortenerRef<'_, '_>>(), size_of::<*const ()>());
        assert_eq!(size_of::<OidShortenerMut<'_, '_>>(), size_of::<*const ()>());
        assert_eq!(
            size_of::<OidShortenerBox<'_>>(),
            size_of::<*mut ffi::git_oid_shorten>()
        );
        assert_eq!(
            size_of::<Option<OidShortenerBox<'_>>>(),
            size_of::<*mut ffi::git_oid_shorten>()
        );
    }

    /// The accessors over Rust storage, without libgit2, so Miri runs them.
    #[test]
    fn accessors_round_trip_in_rust_storage() {
        let mut oid = Oid::zeroed();
        assert_eq!(oid.as_ref().raw_kind(), 0);
        assert_eq!(oid.as_ref().id(), [0; MAX_SIZE]);

        let mut m = oid.as_mut();
        m.set_kind(OidType::Sha1);
        m.set_id(padded(&SHA1_RAW));
        assert_eq!(m.as_ref().raw_kind(), 1);
        assert_eq!(m.as_ref().kind(), Some(OidType::Sha1));
        m.set_kind(OidType::Sha256);
        assert_eq!(m.as_ref().raw_kind(), 2);
        assert_eq!(m.as_ref().kind(), Some(OidType::Sha256));
        {
            let mut id = m.id_mut();
            assert_eq!(id.len(), MAX_SIZE);
            assert!(id.set_elem(31, 0xee));
            assert!(!id.set_elem(32, 0xee));
            assert_eq!(id.elem(0), Some(0xa9));
            assert_eq!(id.elem(32), None);
        }
        let bytes = oid.as_ref().id_bytes();
        assert_eq!(bytes.len(), MAX_SIZE);
        let mut copy = [0; MAX_SIZE];
        assert!(bytes.copy_to_slice(&mut copy));
        let mut expected = padded(&SHA1_RAW);
        expected[31] = 0xee;
        assert_eq!(copy, expected);
        assert_eq!(oid.as_ref().id(), expected);

        // Any type byte is stored as given; unnamed ones read as no type.
        for raw in [0, 3, u8::MAX] {
            oid.as_mut().set_raw_kind(raw);
            assert_eq!(oid.as_ref().raw_kind(), raw);
            assert_eq!(oid.as_ref().kind(), None);
        }
    }

    #[test]
    fn oid_is_a_copyable_value() {
        let mut a = Oid::zeroed();
        a.as_mut().set_id([7; MAX_SIZE]);
        let b = a;
        a.as_mut().set_id([8; MAX_SIZE]);
        assert_eq!(b.as_ref().id(), [7; MAX_SIZE]);
        assert_eq!(a.as_ref().id(), [8; MAX_SIZE]);
    }

    #[test]
    fn debug_shows_kind_and_hex_id() {
        let mut oid = Oid::zeroed();
        oid.as_mut().set_raw_kind(1);
        oid.as_mut().set_id(padded(&SHA1_RAW));
        let hex = format!("{}{}", SHA1_HEX.to_str().unwrap(), "0".repeat(24));
        let expected = format!("Oid {{ kind: Sha1, id: {hex} }}");
        assert_eq!(format!("{oid:?}"), expected);
        assert_eq!(format!("{:?}", oid.as_ref()), expected);
        assert_eq!(
            format!("{:?}", Oid::zeroed()),
            format!("Oid {{ kind: 0, id: {} }}", "0".repeat(64))
        );
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copy_from_and_to_oid_copy_every_byte() {
        let mut src = Oid::zeroed();
        src.as_mut().set_raw_kind(0xab);
        src.as_mut().set_id(core::array::from_fn(|i| i as u8));
        let mut dst = Oid::zeroed();
        dst.as_mut().set_id([0xff; MAX_SIZE]);
        dst.as_mut().copy_from(src.as_ref());
        assert_eq!(dst.as_ref().raw_kind(), 0xab);
        assert_eq!(dst.as_ref().id(), src.as_ref().id());

        let copy = src.as_ref().to_oid();
        assert_eq!(copy.as_ref().raw_kind(), 0xab);
        assert_eq!(copy.as_ref().id(), src.as_ref().id());
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn from_raw_refuses_a_wrong_length_before_ffi() {
        for (raw, kind) in [
            (&SHA1_RAW[..19], OidType::Sha1),
            (&[0u8; 21][..], OidType::Sha1),
            (&[][..], OidType::Sha1),
            (&SHA1_RAW[..], OidType::Sha256),
            (&[0u8; 33][..], OidType::Sha256),
        ] {
            let err = Oid::from_raw(raw, kind).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            assert_eq!(err.message(), c"unable to parse OID - invalid raw length");
        }
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_parse_what_c_parses() {
        let oid = Oid::from_string(SHA1_HEX, OidType::Sha1).unwrap();
        assert_eq!(oid.as_ref().raw_kind(), 1);
        assert_eq!(oid.as_ref().id(), padded(&SHA1_RAW));
        let upper = CString::new(SHA1_HEX.to_str().unwrap().to_uppercase()).unwrap();
        let oid = Oid::from_string(&upper, OidType::Sha1).unwrap();
        assert_eq!(oid.as_ref().id(), padded(&SHA1_RAW));

        let oid = Oid::from_prefix(b"a94", OidType::Sha1).unwrap();
        assert_eq!(oid.as_ref().id(), padded(&[0xa9, 0x40]));

        let oid = Oid::from_raw(&SHA1_RAW, OidType::Sha1).unwrap();
        assert_eq!(oid.as_ref().id(), padded(&SHA1_RAW));
        let oid = Oid::from_raw(&[0x5a; 32], OidType::Sha256).unwrap();
        assert_eq!(oid.as_ref().raw_kind(), 2);
        assert_eq!(oid.as_ref().id(), [0x5a; 32]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shortener_refuses_an_oversized_min_length_before_ffi() {
        // Under a debug build of libgit2 (the coverage variant) the C assertion would
        // abort the test binary.
        let refused = [
            c_int::MAX as usize + 1,
            u32::MAX as usize,
            usize::MAX - c_int::MAX as usize - 1,
            usize::MAX / 2,
        ];
        for min_length in refused {
            let err = OidShortener::new(min_length).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            assert_eq!(
                err.message(),
                c"invalid argument: '(size_t)((int)min_length) == min_length'"
            );
        }
        // Sign-extended negative `int`s round-trip, as in C.
        for min_length in [c_int::MAX as usize, c_int::MIN as usize, usize::MAX] {
            assert!(OidShortener::new(min_length).is_ok(), "{min_length}");
        }
    }

    /// Adopting a raw `git_oid_shorten_new` result, and the drop plumbing: each owner
    /// releases its shortener once (LSan, in the asan variant, flags a leak, ASan a
    /// double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adopted_shorteners_are_freed_once() {
        assert!(runtime_pinned());
        // SAFETY: the runtime is pinned; the result is a fresh shortener or NULL.
        let raw = unsafe { ffi::git_oid_shorten_new(4) };
        // SAFETY: a fresh shortener nothing else references, released by
        // `git_oid_shorten_free`.
        let owned = unsafe { OidShortenerBox::from_c(raw) }.unwrap();
        assert_eq!(owned.as_c_ptr(), raw);

        // Released by hand after surrendering ownership.
        let raw = owned.into_c();
        // SAFETY: `raw` is live and owned here again.
        unsafe { OidShortenerFree.c_drop(NonNull::new(raw).unwrap().cast()) };

        // SAFETY: NULL is refused without adopting anything.
        assert!(unsafe { OidShortenerBox::from_c(core::ptr::null_mut()) }.is_none());

        let mut b = OidShortener::new(0).unwrap();
        let mut m = b.as_mut();
        let p = m.as_mut_ptr();
        assert_eq!(m.as_mut().as_ref().as_ptr(), p.cast_const());
        assert_eq!(format!("{m:?}"), "OidShortener { .. }");
        drop(b);
    }

    /// When libgit2 cannot be initialised, every constructor reports C's static "not
    /// initialized" error without calling into it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_without_runtime_fail_cleanly() {
        isolated(
            concat!(
                module_path!(),
                "::constructors_without_runtime_fail_cleanly"
            ),
            || {
                install_refusing();
                REFUSING.store(true, SeqCst);
                let errors = [
                    Oid::from_string(SHA1_HEX, OidType::Sha1).unwrap_err(),
                    Oid::from_prefix(b"a94a", OidType::Sha1).unwrap_err(),
                    Oid::from_raw(&SHA1_RAW, OidType::Sha1).unwrap_err(),
                    OidShortener::new(0).unwrap_err(),
                ];
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                for e in errors {
                    assert_eq!(e.class(), Some(ErrorClass::Invalid));
                    assert_eq!(e.message(), c"library has not been initialized");
                }
            },
        );
    }

    /// When libgit2 cannot be initialised, every formatter reports C's static "not
    /// initialized" error without calling into it, whatever the oid's type.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn formatters_without_runtime_fail_cleanly() {
        isolated(
            concat!(module_path!(), "::formatters_without_runtime_fail_cleanly"),
            || {
                install_refusing();
                REFUSING.store(true, SeqCst);
                let mut oid = Oid::zeroed();
                oid.as_mut().set_kind(OidType::Sha1);
                let r = oid.as_ref();
                let mut buf = [b'#'; MAX_HEX_SIZE + 1];
                let errors = [
                    r.format((&mut buf[..MAX_HEX_SIZE]).try_into().unwrap())
                        .map(drop)
                        .unwrap_err(),
                    r.format_prefix(&mut buf).map(drop).unwrap_err(),
                    r.format_path(&mut buf).map(drop).unwrap_err(),
                    r.format_cstr(&mut buf).map(drop).unwrap_err(),
                    r.to_hex_string().map(drop).unwrap_err(),
                ];
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                assert_eq!(buf, [b'#'; MAX_HEX_SIZE + 1]);
                for e in errors {
                    assert_eq!(e.class(), Some(ErrorClass::Invalid));
                    assert_eq!(e.message(), c"library has not been initialized");
                }
            },
        );
    }

    /// An allocation failure in `git_oid_shorten_new` (the struct, or its trie) is
    /// reported as libgit2's out-of-memory error, and nothing is adopted.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shortener_out_of_memory_is_an_error() {
        isolated(
            concat!(module_path!(), "::shortener_out_of_memory_is_an_error"),
            || {
                install_refusing();
                assert!(runtime_pinned());
                // Allocate the thread's error state before allocations start failing.
                set(ErrorClass::Invalid, c"primed");
                drop(OidShortener::new(0).unwrap());
                REFUSING.store(true, SeqCst);
                let err = OidShortener::new(0).unwrap_err();
                REFUSING.store(false, SeqCst);
                assert_eq!(err.class(), Some(ErrorClass::NoMemory));
                assert_eq!(err.message(), c"Out of memory");

                // The struct is allocated, its trie is not: C frees the struct (LSan, in
                // the asan variant, flags it otherwise).
                set(ErrorClass::Invalid, c"primed");
                REFUSING_REALLOC.store(true, SeqCst);
                let err = OidShortener::new(0).unwrap_err();
                REFUSING_REALLOC.store(false, SeqCst);
                assert_eq!(err.class(), Some(ErrorClass::NoMemory));
                drop(OidShortener::new(0).unwrap());
            },
        );
    }

    /// Like [`isolated`], for a body that must end the child with `abort()`.
    fn isolated_aborts(test: &str, body: fn()) {
        use std::os::unix::process::ExitStatusExt;
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
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.signal(),
            Some(libc::SIGABRT),
            "child `{name}` did not abort: {:?}\n{}\n{stderr}",
            out.status,
            String::from_utf8_lossy(&out.stdout)
        );
        assert!(stderr.contains("lost its trie"), "{stderr}");
    }

    /// The 40-digit check `add` makes before the call, without libgit2.
    #[test]
    fn shortenable_ids_start_with_forty_hex_digits() {
        let hex = SHA1_HEX.to_str().unwrap();
        assert!(shortenable(hex));
        assert!(shortenable(&hex.to_uppercase()));
        assert!(shortenable(&format!("{hex}{hex}")));
        assert!(shortenable(&format!("{hex}not hex")));
        assert!(!shortenable(""));
        assert!(!shortenable(&hex[..39]));
        for i in 0..SHORTEN_DIGITS {
            for bad in ['g', 'G', ' ', '\0', '/', ':', '@', '`', 'é'] {
                let mut id: Vec<char> = hex.chars().collect();
                id[i] = bad;
                let id: String = id.into_iter().collect();
                assert!(!shortenable(&id), "{id:?}");
            }
        }
        assert_eq!(hex_str(b"09af"), "09af");
    }

    /// `Oid` orders and compares through its shared handle, so sorting values sorts
    /// as libgit2 does: type first, then the raw id.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn oid_values_order_like_their_handles() {
        let mut oids: Vec<Oid> = [(2, 0x00), (1, 0xff), (1, 0x10), (0, 0x99), (1, 0x10)]
            .into_iter()
            .map(|(kind, first)| {
                let mut oid = Oid::zeroed();
                oid.as_mut().set_raw_kind(kind);
                assert!(oid.as_mut().id_mut().set_elem(0, first));
                oid
            })
            .collect();
        assert_eq!(oids[2], oids[4]);
        assert_ne!(oids[1], oids[2]);
        assert_eq!(oids[2].partial_cmp(&oids[1]), Some(Ordering::Less));
        assert_eq!(
            oids[0].as_ref().partial_cmp(&oids[1].as_ref()),
            Some(Ordering::Greater)
        );
        oids.sort();
        let order: Vec<(u8, u8)> = oids
            .iter()
            .map(|o| (o.as_ref().raw_kind(), o.as_ref().id()[0]))
            .collect();
        assert_eq!(
            order,
            [(0, 0x99), (1, 0x10), (1, 0x10), (1, 0xff), (2, 0x00)]
        );
        // Handles of different lifetimes compare.
        let long = oids[1];
        let r = long.as_ref();
        {
            let short = oids[2];
            assert!(r == short.as_ref());
        }
    }

    /// C's `git_oid_tostr_s` returns its per-thread string unwritten for an unknown
    /// type, uninitialised and unterminated on the thread's first call; the wrapper
    /// refuses the type first, with the error C sets.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn to_hex_string_refuses_unknown_types_before_ffi() {
        std::thread::spawn(|| {
            for raw in [0, 3, u8::MAX] {
                let mut oid = Oid::zeroed();
                oid.as_mut().set_raw_kind(raw);
                let err = oid.as_ref().to_hex_string().unwrap_err();
                assert_eq!(err.class(), Some(ErrorClass::Invalid));
                assert_eq!(err.message(), c"unable to parse OID - unknown type");
            }
            let oid = Oid::from_string(SHA1_HEX, OidType::Sha1).unwrap();
            assert_eq!(
                oid.as_ref().to_hex_string().unwrap(),
                SHA1_HEX.to_str().unwrap()
            );
        })
        .join()
        .unwrap();
    }

    /// libgit2 cannot allocate a thread's string: its error, and the next call on the
    /// thread allocates it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn to_hex_string_out_of_memory_is_an_error() {
        isolated(
            concat!(module_path!(), "::to_hex_string_out_of_memory_is_an_error"),
            || {
                install_refusing();
                assert!(runtime_pinned());
                std::thread::spawn(|| {
                    // Allocate the thread's error state before allocations start failing.
                    set(ErrorClass::Invalid, c"primed");
                    let oid = Oid::from_string(SHA1_HEX, OidType::Sha1).unwrap();
                    REFUSING.store(true, SeqCst);
                    let err = oid.as_ref().to_hex_string().unwrap_err();
                    REFUSING.store(false, SeqCst);
                    assert_eq!(err.class(), Some(ErrorClass::NoMemory));
                    assert_eq!(
                        oid.as_ref().to_hex_string().unwrap(),
                        SHA1_HEX.to_str().unwrap()
                    );
                })
                .join()
                .unwrap();
            },
        );
    }

    /// `add` refuses ids C would store and later index its trie out of bounds with,
    /// and the shortener stays as it was.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn add_refuses_malformed_ids_before_ffi() {
        let hex = SHA1_HEX.to_str().unwrap();
        let mut bad_tail = hex.to_owned();
        bad_tail.replace_range(39.., "g");
        let malformed = ["", "a", "ab", &hex[..39], &bad_tail, "zz"];
        let mut b = OidShortener::new(0).unwrap();
        for id in malformed {
            let err = b.as_mut().add(id).unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Invalid));
            assert_eq!(err.message(), c"unable to shorten OID - invalid hex value");
        }
        assert_eq!(b.as_mut().add(hex), Ok(1));
        assert_eq!(
            b.as_mut().add("a94b000000000000000000000000000000000000"),
            Ok(4)
        );
        assert_eq!(b.as_mut().add(hex), Ok(41));
    }

    /// A trie libgit2 could not grow is NULL in C, and the next addition would
    /// dereference it: `add` aborts instead of returning.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn add_aborts_when_the_trie_cannot_grow() {
        isolated_aborts(
            concat!(module_path!(), "::add_aborts_when_the_trie_cannot_grow"),
            || {
                install_refusing();
                assert!(runtime_pinned());
                set(ErrorClass::Invalid, c"primed");
                let ids: Vec<String> = "0123456789abcdef"
                    .chars()
                    .map(|c| format!("{c}{}", &SHA1_HEX.to_str().unwrap()[1..]))
                    .collect();
                let mut b = OidShortener::new(0).unwrap();
                REFUSING_REALLOC.store(true, SeqCst);
                // The trie starts with 16 nodes, the root among them: the 16th leaf
                // needs it grown.
                for id in &ids[..15] {
                    assert_eq!(b.as_mut().add(id), Ok(1));
                }
                let _ = b.as_mut().add(&ids[15]);
                unreachable!("add returned with the trie lost");
            },
        );
    }

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
