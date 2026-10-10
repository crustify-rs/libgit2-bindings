//! Safe wrappers over `include/git2/blob.h`.
//!
//! # Filter flags
//!
//! A [`FilterFlag`] is a `git_blob_filter_flag_t`, one bit of the `uint32_t` `flags`
//! of `git_blob_filter_options`; a [`FilterFlags`] set ORs them. `git_blob_filter`
//! tests the four bits the header names, one at a time, and ignores every other, so a
//! set keeps any bit it is given ([`FilterFlags::from_bits_retain`]) and passes it
//! through unchanged.
//!
//! The empty set is [`FilterFlags`]'s `Default`, as for every flag set; the options'
//! own default (`GIT_BLOB_FILTER_OPTIONS_INIT`, and `git_blob_filter_options_init`) is
//! [`FilterFlags::OPTIONS_DEFAULT`], which holds [`FilterFlag::CheckForBinary`].
//!
//! # Binary content
//!
//! [`data_is_binary`] applies the binary-content heuristic `git_blob_is_binary` uses
//! (core git's) to a byte slice. libgit2 reads the bytes for the call only and touches
//! neither its allocator nor the thread's error state, so the check needs no
//! initialised runtime and is safe from any thread.

use core::ffi::c_int;
use core::fmt;
use core::ops::{BitOr, BitOrAssign};

use crate::ffi;

/// One option of `git_blob_filter`, a single bit of a [`FilterFlags`] set
/// (`git_blob_filter_flag_t`).
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_blob_filter_flag_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum FilterFlag {
    /// Leave a binary blob unfiltered: `git_blob_filter` succeeds without writing its
    /// output when the blob's content looks binary.
    CheckForBinary = ffi::GIT_BLOB_FILTER_CHECK_FOR_BINARY,
    /// Do not load the system-wide `gitattributes` (in `/etc`, or the system's
    /// equivalent).
    NoSystemAttributes = ffi::GIT_BLOB_FILTER_NO_SYSTEM_ATTRIBUTES,
    /// Also load the `.gitattributes` file of the `HEAD` commit.
    AttributesFromHead = ffi::GIT_BLOB_FILTER_ATTRIBUTES_FROM_HEAD,
    /// Also load the `.gitattributes` file of the commit the options'
    /// `attr_commit_id` names.
    AttributesFromCommit = ffi::GIT_BLOB_FILTER_ATTRIBUTES_FROM_COMMIT,
}

impl FilterFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [FilterFlag; 4] = [
        Self::CheckForBinary,
        Self::NoSystemAttributes,
        Self::AttributesFromHead,
        Self::AttributesFromCommit,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_blob_filter_flag_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_blob_filter_flag_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value, a single `git_blob_filter_flag_t` bit.
    pub const fn raw(self) -> ffi::git_blob_filter_flag_t {
        self as ffi::git_blob_filter_flag_t
    }

    /// The flag's bit in `git_blob_filter_options.flags`.
    const fn bit(self) -> u32 {
        self as u32
    }
}

impl TryFrom<ffi::git_blob_filter_flag_t> for FilterFlag {
    type Error = UnnamedFilterFlag;

    fn try_from(raw: ffi::git_blob_filter_flag_t) -> Result<Self, UnnamedFilterFlag> {
        Self::from_raw(raw).ok_or(UnnamedFilterFlag(raw))
    }
}

impl From<FilterFlag> for ffi::git_blob_filter_flag_t {
    fn from(value: FilterFlag) -> ffi::git_blob_filter_flag_t {
        value.raw()
    }
}

// A `FilterFlag` passes where C takes a `git_blob_filter_flag_t`.
const _: () = assert!(
    size_of::<FilterFlag>() == size_of::<ffi::git_blob_filter_flag_t>()
        && align_of::<FilterFlag>() == align_of::<ffi::git_blob_filter_flag_t>()
);

/// A blob filter flag value that this version of libgit2's header does not name: `0`,
/// a combination of flags, or a later header's bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedFilterFlag(ffi::git_blob_filter_flag_t);

impl UnnamedFilterFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_blob_filter_flag_t {
        self.0
    }
}

impl fmt::Display for UnnamedFilterFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 blob filter flag {:#x} is not named by git2/blob.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedFilterFlag {}

/// A set of [`FilterFlag`]s: the `uint32_t` combination of `GIT_BLOB_FILTER_*` bits
/// that `git_blob_filter_options.flags` holds. The default is the empty set;
/// [`OPTIONS_DEFAULT`](Self::OPTIONS_DEFAULT) is the options' default.
///
/// It keeps every bit it is given, named or not, as the field does (`git_blob_filter`
/// ignores the bits it does not name), so [`bits`](Self::bits) returns what was
/// stored; [`from_bits_truncate`](Self::from_bits_truncate) keeps the named ones only.
/// [`iter`](Self::iter) and [`contains`](Self::contains) see the named flags, and
/// [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_blob_filter_flag_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FilterFlags(u32);

impl FilterFlags {
    /// The bits of every [`FilterFlag`].
    const NAMED: u32 = {
        let mut bits = 0;
        let mut i = 0;
        while i < FilterFlag::ALL.len() {
            bits |= FilterFlag::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The flags `GIT_BLOB_FILTER_OPTIONS_INIT` and `git_blob_filter_options_init`
    /// set: [`FilterFlag::CheckForBinary`] alone.
    pub const OPTIONS_DEFAULT: Self = Self(FilterFlag::CheckForBinary.bit());

    /// The set with no flag: binary blobs are filtered too, and only the system and
    /// working-directory attributes are loaded.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as `git_blob_filter_options.flags` holds them;
    /// bits that no [`FilterFlag`] names are kept.
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The set of the bits of `bits` that a [`FilterFlag`] names; the rest are dropped.
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::NAMED)
    }

    /// The bits of the set, as `git_blob_filter_options.flags` holds them.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The bits of the set that no [`FilterFlag`] names.
    pub const fn unnamed_bits(self) -> u32 {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: FilterFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: FilterFlag) {
        self.0 |= flag.bit();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: FilterFlag) {
        self.0 &= !flag.bit();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> FilterFlagsIter {
        FilterFlagsIter { set: self, next: 0 }
    }
}

impl From<FilterFlag> for FilterFlags {
    fn from(flag: FilterFlag) -> Self {
        Self(flag.bit())
    }
}

impl BitOr for FilterFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<FilterFlag> for FilterFlags {
    type Output = Self;

    fn bitor(self, rhs: FilterFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for FilterFlag {
    type Output = FilterFlags;

    fn bitor(self, rhs: Self) -> FilterFlags {
        FilterFlags::from(self) | rhs
    }
}

impl BitOrAssign for FilterFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<FilterFlag> for FilterFlags {
    fn bitor_assign(&mut self, rhs: FilterFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<FilterFlag> for FilterFlags {
    fn from_iter<I: IntoIterator<Item = FilterFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<FilterFlag> for FilterFlags {
    fn extend<I: IntoIterator<Item = FilterFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for FilterFlags {
    type Item = FilterFlag;
    type IntoIter = FilterFlagsIter;

    fn into_iter(self) -> FilterFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex:
/// `{CheckForBinary | AttributesFromHead, 0x80}`.
impl fmt::Debug for FilterFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FilterFlags {")?;
        let mut sep = "";
        for flag in self.iter() {
            write!(f, "{sep}{flag:?}")?;
            sep = " | ";
        }
        if self.unnamed_bits() != 0 {
            let sep = if sep.is_empty() { "" } else { ", " };
            write!(f, "{sep}{:#x}", self.unnamed_bits())?;
        }
        f.write_str("}")
    }
}

/// The named flags of a [`FilterFlags`] set, in ascending bit order
/// ([`FilterFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct FilterFlagsIter {
    set: FilterFlags,
    /// The index into [`FilterFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for FilterFlagsIter {
    type Item = FilterFlag;

    fn next(&mut self) -> Option<FilterFlag> {
        while let Some(&flag) = FilterFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(FilterFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for FilterFlagsIter {}

/// Whether `data` is most likely binary, by core git's heuristic: it is binary when it
/// opens with a UTF-16 or UTF-32 byte-order mark, when it holds a NUL byte, or when
/// fewer than 128 printable bytes come for each non-printable one. Printable bytes are
/// those above `0x1f` but DEL (space included), and backspace, escape and form feed;
/// tab, line feed, vertical tab and carriage return count as neither; every other byte
/// below `0x20`, and DEL, is non-printable. A UTF-8 byte-order mark is skipped, and
/// empty data is text.
///
/// Unlike `git_blob_is_binary`, it reads the whole slice, not its first 8000 bytes.
///
/// libgit2 counts printable and non-printable bytes in C `int`s, which overflow (UB)
/// once a slice holds more than `INT_MAX` of either. A slice that long never reaches it:
/// the same heuristic runs in Rust, with counters as wide as the slice's length.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::object::blob::data_is_binary;
///
/// assert!(!data_is_binary(b"hello, world\n"));
/// assert!(data_is_binary(b"hello\0world"));
/// assert!(data_is_binary(b"\xff\xfeh\0i\0"));
/// assert!(!data_is_binary(b""));
/// ```
///
/// Wraps: git_blob_data_is_binary
pub fn data_is_binary(data: &[u8]) -> bool {
    if !fits_c_counters(data.len()) {
        return wide::is_binary(data);
    }
    // SAFETY: `data` is live for `data.len()` bytes, read for the call only and never
    // retained or written; with a length of 0 libgit2 does not read the pointer at all.
    // The length is at most `INT_MAX`, so neither of libgit2's `int` counters, each
    // bounded by it, overflows. No allocation or error state is touched, so no runtime
    // pin is needed.
    let rc = unsafe { ffi::git_blob_data_is_binary(data.as_ptr().cast(), data.len()) };
    rc != 0
}

/// Whether libgit2's `int` counters can count a slice of `len` bytes: each counts a
/// subset of its bytes, so at most `INT_MAX` of them never overflows either.
fn fits_c_counters(len: usize) -> bool {
    len <= c_int::MAX as usize
}

/// `git_str_is_binary` (`src/util/str.c`), over slices longer than its `int` counters
/// can count.
mod wide {
    /// The bytes a byte-order mark at the start of `data` takes, and whether it marks
    /// UTF-16 or UTF-32 (which make the data binary), as `git_str_detect_bom` reads it.
    pub(super) fn bom(data: &[u8]) -> (usize, bool) {
        match data {
            [0, 0, 0xfe, 0xff, ..] => (4, true),
            [0xef, 0xbb, 0xbf, ..] => (3, false),
            [0xfe, 0xff, ..] => (2, true),
            [0xff, 0xfe, 0, 0, ..] => (4, true),
            [0xff, 0xfe, ..] => (2, true),
            _ => (0, false),
        }
    }

    /// [`super::data_is_binary`]'s heuristic, with `usize` counters.
    pub(super) fn is_binary(data: &[u8]) -> bool {
        let (skip, wide_bom) = bom(data);
        if wide_bom {
            return true;
        }
        let (mut printable, mut nonprintable) = (0usize, 0usize);
        for &c in &data[skip..] {
            if (c > 0x1f && c != 0x7f) || matches!(c, 0x08 | 0x1b | 0x0c) {
                printable += 1;
            } else if c == 0 {
                return true;
            } else if !matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
                nonprintable += 1;
            }
        }
        (printable >> 7) < nonprintable
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// Values probed for `from_raw`: zero, each bit, their neighbours and combinations.
    fn probes() -> impl Iterator<Item = u32> {
        (0..=64u32).chain((0..32).map(|s| 1u32 << s)).chain([
            u32::MAX,
            u32::MAX - 1,
            0x8000_0000,
            0x0f,
            0x10,
        ])
    }

    #[test]
    fn from_raw_round_trips_each_named_flag() {
        for flag in FilterFlag::ALL {
            assert_eq!(FilterFlag::from_raw(flag.raw()), Some(flag));
            assert_eq!(FilterFlag::try_from(flag.raw()), Ok(flag));
            assert_eq!(ffi::git_blob_filter_flag_t::from(flag), flag.raw());
            assert_eq!(flag.raw().count_ones(), 1, "{flag:?}");
        }
        let named = probes()
            .filter(|&r| FilterFlag::from_raw(r).is_some())
            .count();
        // Each named bit appears twice in `probes`: in the range and as a shift.
        assert_eq!(named, 2 * FilterFlag::ALL.len());
    }

    #[test]
    fn unnamed_values_are_refused_with_their_value() {
        for raw in probes().filter(|&r| FilterFlag::from_raw(r).is_none()) {
            let err = FilterFlag::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 blob filter flag {raw:#x} is not named by git2/blob.h")
            );
        }
    }

    #[test]
    fn set_operations() {
        let mut set = FilterFlags::empty();
        assert!(set.is_empty() && set == FilterFlags::default());
        assert_eq!(set.iter().next(), None);
        set.insert(FilterFlag::AttributesFromHead);
        set |= FilterFlag::CheckForBinary;
        assert_eq!(set.bits(), 0b101);
        assert!(set.contains(FilterFlag::CheckForBinary));
        assert!(!set.contains(FilterFlag::NoSystemAttributes));
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [FilterFlag::CheckForBinary, FilterFlag::AttributesFromHead]
        );
        set.remove(FilterFlag::CheckForBinary);
        set.remove(FilterFlag::CheckForBinary);
        assert_eq!(set, FilterFlag::AttributesFromHead.into());
        set |= FilterFlags::from_bits_retain(0x8000_0000);
        assert_eq!(set.unnamed_bits(), 0x8000_0000);
        assert!(!set.is_empty());
        assert_eq!(
            FilterFlags::from_bits_truncate(set.bits()),
            FilterFlag::AttributesFromHead.into()
        );
        let all: FilterFlags = FilterFlag::ALL.into_iter().collect();
        assert_eq!(all.bits(), 0b1111);
        assert_eq!(all, FilterFlags::from_bits_truncate(u32::MAX));
        assert_eq!(all.unnamed_bits(), 0);
        assert_eq!(
            FilterFlag::NoSystemAttributes | FilterFlag::AttributesFromCommit,
            FilterFlags::from_bits_retain(0b1010)
        );
        let mut ext = FilterFlags::from(FilterFlag::CheckForBinary);
        ext.extend([FilterFlag::AttributesFromCommit]);
        ext |= FilterFlags::from(FilterFlag::NoSystemAttributes);
        assert_eq!(ext.into_iter().count(), 3);
        assert_eq!(
            FilterFlags::OPTIONS_DEFAULT,
            FilterFlag::CheckForBinary.into()
        );
    }

    #[test]
    fn iter_is_fused_and_bounded() {
        let mut it = FilterFlags::from_bits_retain(u32::MAX).iter();
        assert_eq!(it.size_hint(), (0, Some(4)));
        assert_eq!(it.by_ref().count(), 4);
        assert_eq!(it.size_hint(), (0, Some(0)));
        assert_eq!(it.next(), None);
        assert_eq!(it.clone().next(), None);
    }

    #[test]
    fn debug_lists_named_then_unnamed_bits() {
        assert_eq!(format!("{:?}", FilterFlags::empty()), "FilterFlags {}");
        assert_eq!(
            format!(
                "{:?}",
                FilterFlag::CheckForBinary | FilterFlag::AttributesFromHead
            ),
            "FilterFlags {CheckForBinary | AttributesFromHead}"
        );
        assert_eq!(
            format!("{:?}", FilterFlags::from_bits_retain(0x81)),
            "FilterFlags {CheckForBinary, 0x80}"
        );
        assert_eq!(
            format!("{:?}", FilterFlags::from_bits_retain(0x80)),
            "FilterFlags {0x80}"
        );
    }

    /// Inputs around every branch of the heuristic: each byte-order mark and its
    /// truncations, NULs before and after one, each class of byte alone, and the
    /// printable-to-non-printable ratio on both sides of 128.
    fn corpus() -> Vec<Vec<u8>> {
        let mut inputs: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"a".to_vec(),
            b"\0".to_vec(),
            b"\xef\xbb\xbf".to_vec(),
            b"\xef\xbb\xbfplain".to_vec(),
            b"\xef\xbb\xbf\0".to_vec(),
            b"\xef\xbb".to_vec(),
            b"\xfe\xff".to_vec(),
            b"\xfe\xfe".to_vec(),
            b"\xff\xfe".to_vec(),
            b"\xff\xfe\0".to_vec(),
            b"\xff\xfe\0\0".to_vec(),
            b"\xff\xfe\0a".to_vec(),
            b"\0\0\xfe\xff".to_vec(),
            b"\0\0\xfe".to_vec(),
            b"\0\0\xfe\xfe".to_vec(),
            b"\xff\xff".to_vec(),
            b"line\r\n\tindented\x0b\x0c".to_vec(),
            b"\x08\x1b\x0c".to_vec(),
            b"\x7f".to_vec(),
            b"\x01".to_vec(),
            b"\x80\xff\xa0".to_vec(),
        ];
        for (printable, nonprintable) in [(127, 1), (128, 1), (255, 1), (256, 2), (256, 3)] {
            let mut ratio = vec![b'x'; printable];
            ratio.extend(std::iter::repeat_n(0x01, nonprintable));
            inputs.push(ratio);
        }
        for ws in [b' ', b'\t', b'\n', 0x0b, 0x0c, b'\r', 0x08, 0x1b] {
            let mut run = vec![ws; 128];
            run.push(0x01);
            inputs.push(run);
        }
        inputs.extend((0..=u8::MAX).map(|c| vec![c]));
        inputs.extend((0..=u8::MAX).map(|c| vec![b'a', c, b'b']));
        inputs
    }

    /// `git_blob_data_is_binary` itself, as the C reference.
    fn c_is_binary(data: &[u8]) -> bool {
        // SAFETY: `data` is live for `data.len()` bytes, at most `INT_MAX`.
        unsafe { ffi::git_blob_data_is_binary(data.as_ptr().cast(), data.len()) != 0 }
    }

    /// The Rust heuristic that stands in for libgit2 on oversized slices agrees with it
    /// on every input libgit2 can take.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn wide_heuristic_matches_libgit2() {
        for input in corpus() {
            assert_eq!(wide::is_binary(&input), c_is_binary(&input), "{input:x?}");
            assert_eq!(data_is_binary(&input), c_is_binary(&input), "{input:x?}");
        }
    }

    /// The byte-order marks `git_str_detect_bom` recognises, and the bytes each takes.
    #[test]
    fn byte_order_marks() {
        assert_eq!(wide::bom(b"\0\0\xfe\xffx"), (4, true));
        assert_eq!(wide::bom(b"\xef\xbb\xbfx"), (3, false));
        assert_eq!(wide::bom(b"\xfe\xffx"), (2, true));
        assert_eq!(wide::bom(b"\xff\xfe\0\0x"), (4, true));
        assert_eq!(wide::bom(b"\xff\xfe\0x"), (2, true));
        assert_eq!(wide::bom(b"\xef\xbb"), (0, false));
        assert_eq!(wide::bom(b"\xff"), (0, false));
        assert_eq!(wide::bom(b""), (0, false));
    }

    /// The Rust heuristic's verdicts on each branch, without libgit2, so Miri checks
    /// its slicing past a byte-order mark too.
    #[test]
    fn wide_heuristic_verdicts() {
        for (data, binary) in [
            (&b""[..], false),
            (b"\xef\xbb\xbf", false),
            (b"\xef\xbb\xbf\0", true),
            (b"\xff\xfe", true),
            (b"\0\0\xfe\xff", true),
            (b"text\r\n\t\x0b\x0c\x08\x1b", false),
            (b"\x01", true),
            (b"\x7f", true),
            (b"\x80\xff", false),
        ] {
            assert_eq!(wide::is_binary(data), binary, "{data:x?}");
        }
        let mut ratio = vec![b'x'; 256];
        ratio.push(0x01);
        assert!(!wide::is_binary(&ratio));
        ratio.push(0x01);
        assert!(!wide::is_binary(&ratio));
        ratio.push(0x01);
        assert!(wide::is_binary(&ratio));
    }

    /// Space, form feed, backspace and escape are printable: 128 of them carry one
    /// control byte. Tab, line feed, vertical tab and carriage return count as neither,
    /// so the same control byte makes the data binary. The Rust heuristic runs under
    /// Miri too; the end-to-end check reaches libgit2.
    #[test]
    fn whitespace_classes() {
        for (ws, binary) in [
            (b' ', false),
            (0x0c, false),
            (0x08, false),
            (0x1b, false),
            (b'\t', true),
            (b'\n', true),
            (0x0b, true),
            (b'\r', true),
        ] {
            let mut run = vec![ws; 128];
            run.push(0x01);
            assert_eq!(wide::is_binary(&run), binary, "{ws:#x}");
            if !cfg!(miri) {
                assert_eq!(data_is_binary(&run), binary, "{ws:#x}");
            }
        }
    }

    /// Slices up to `INT_MAX` bytes reach libgit2; one byte more takes the Rust
    /// heuristic.
    #[test]
    fn longest_slice_for_libgit2_is_int_max() {
        assert!(fits_c_counters(0));
        assert!(fits_c_counters(c_int::MAX as usize));
        assert!(!fits_c_counters(c_int::MAX as usize + 1));
        assert!(!fits_c_counters(usize::MAX));
    }

    /// A slice one byte longer than libgit2's counters can count takes the Rust
    /// heuristic. The slice is zeroed (never touched by `vec!` past the allocator's
    /// own zero-filling, and binary at its first byte), so the test costs one
    /// allocation, not a 2 GiB scan.
    #[test]
    #[cfg_attr(miri, ignore = "allocates 2 GiB")]
    fn oversized_slice_takes_the_rust_heuristic() {
        let mut data = vec![0u8; c_int::MAX as usize + 1];
        assert!(data_is_binary(&data));
        data[..2].copy_from_slice(b"\xff\xfe");
        assert!(data_is_binary(&data));
        data[..3].copy_from_slice(b"\xef\xbb\xbf");
        assert!(data_is_binary(&data));
    }

    /// End to end, a slice whose printable bytes would overflow libgit2's `int` counter
    /// (`INT_MAX + 1` of them) is counted in Rust. It scans 2 GiB, which takes minutes
    /// under the sanitizers: run it with `--ignored`.
    #[test]
    #[ignore = "scans 2 GiB; run with --ignored"]
    fn oversized_slices_are_counted_in_rust() {
        let mut data = vec![b'a'; c_int::MAX as usize + 2];
        // One non-printable byte for 2^24 printable ones is still text.
        data[0] = 0x01;
        assert!(!data_is_binary(&data));
    }
}
