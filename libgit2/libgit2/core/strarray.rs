//! Safe wrappers over `include/git2/strarray.h`.
//!
//! A `git_strarray` is `count` NUL-terminated strings, `strings`. libgit2 never
//! allocates the struct itself, and uses it in two directions under two contracts:
//!
//! - **Results.** `git_reference_list`, `git_tag_list`, `git_remote_list`,
//!   `git_worktree_list`, `GIT_OPT_GET_EXTENSIONS` and the like fill a caller's struct
//!   with an array and strings from libgit2's allocator (a detached `git_vector` of
//!   `git__strdup` copies), which the struct then owns: `git_strarray_dispose` frees
//!   every string, then the array, with `git__free`, and zeroes the struct. Rust holds
//!   such a struct by value as a [`StrArrayVal`], which disposes it on drop. The strings
//!   move in and out as an [`OdbDataStrVec`], the counted run of libgit2-allocated
//!   strings that the same `git__free` releases.
//! - **Arguments.** `const git_strarray *` parameters (pathspecs, refspecs, custom
//!   headers) and the by-value `git_strarray` fields of options structs are only read:
//!   libgit2 copies what it keeps and never writes or frees the caller's array. Rust
//!   builds one over borrowed strings as a [`StrArrayView`], which allocates nothing
//!   from libgit2 and hands out only the shared handle.
//!
//! Both forms reach C, and the getters, through [`StrArrayRef`]; only an owned array
//! has a [`StrArrayMut`], whose setters free what they replace.
//!
//! Every handle upholds one invariant, which the safe constructors and setters keep and
//! a crate-internal `from_ptr` over a C-owned struct must vouch for: `strings` is NULL
//! with `count == 0`, or addresses at least `count` non-NULL NUL-terminated strings
//! that outlive the handle; and behind a `StrArrayMut` the array and every one of
//! those strings are allocations from libgit2's allocator that the struct owns. A NULL
//! `strings` reads as the empty array whatever `count` says, and is never handed to C.
//! libgit2's vectors allocate eagerly, so an empty result is often a non-NULL array
//! with `count == 0`; that array is owned all the same, and released.
//!
//! libgit2's producers overwrite the struct without disposing what it held, so their
//! wrappers fill a fresh [`StrArray::new`], never a caller's populated array.
//!
//! An empty array without a buffer never reaches libgit2: nothing allocated it, so
//! nothing has initialised the library on its behalf, and even `git__free(NULL)` reads
//! the allocator a concurrent first `git_libgit2_init` writes. Every other owned array
//! carries a buffer from libgit2's allocator, whose allocation initialised the library
//! first.
//!
//! `git_strarray_copy` is compiled hidden (`GIT_DEPRECATE_HARD`), so a copy goes
//! through [`OdbDataStrVec::dup_all`], in the same order: the array, then each string.
//!
//! There is no public `StrArray` value outside an owner: one holding strings would leak
//! them when dropped, so the constructors return a [`StrArrayVal`], and `StrArray` is
//! not `CZeroable`. [`CVal::into_inner`] still surrenders the bare value, as
//! `mem::forget` surrenders a `Vec`.

use core::ffi::{CStr, c_char};
use core::fmt;
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::ops::Range;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CDispose, CVal, define_ctype};

use crate::ffi;
use crate::libgit2::raw::{OdbDataStr, OdbDataStrVec};

define_ctype!(
    /// An array of strings (`git_strarray`): `count` NUL-terminated strings in libgit2's
    /// allocator, which the struct owns, as libgit2 hands out its results.
    ///
    /// Held by value inside a [`StrArrayVal`], which [`new`](Self::new),
    /// [`from_strings`](Self::from_strings) and [`from_strs`](Self::from_strs) build;
    /// read and write it through the owner's [`as_ref`](CVal::as_ref) /
    /// [`as_mut`](CVal::as_mut) handles. An argument array over borrowed strings is a
    /// [`StrArrayView`] instead. Not `Clone`: it owns its strings, so a copy is
    /// [`StrArrayRef::to_str_array`], which allocates.
    ///
    /// Wraps: git_strarray
    StrArray, StrArrayRef, StrArrayMut, ffi::git_strarray
);

// SAFETY: an owned `StrArray` is a pointer to an array of pointers to strings, all of
// which it owns exclusively, and their count. They are libgit2-allocator storage, which
// `git__free` releases on any thread (as `OdbDataStrVec`, which is `Send`), and no
// libgit2 routine keeps thread-local state about them; moving the struct moves sole
// ownership of them.
unsafe impl Send for StrArray {}
// SAFETY: `StrArrayMut` is an exclusive borrow of such a value, as `&mut StrArray`.
unsafe impl Send for StrArrayMut<'_> {}

// SAFETY: shared access only reads `strings`, `count` and the bytes they address:
// every write goes through `StrArrayMut`, which excludes other handles, and no libgit2
// routine writes through a `const git_strarray *`.
unsafe impl Sync for StrArray {}
// SAFETY: as above; `StrArrayRef` is a shared borrow, as `&StrArray`. The array it
// reads may also be a `StrArrayView`'s, over strings borrowed as `&CStr`, which is
// `Sync`.
unsafe impl Send for StrArrayRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for StrArrayRef<'_> {}
// SAFETY: `&StrArrayMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for StrArrayMut<'_> {}

/// Releases an array's strings through `git_strarray_dispose`, which hands each of
/// them, then the array, to `git__free`, and zeroes the struct, Rust's storage.
///
/// An array without a buffer (`strings` NULL) is not passed to C: it owns nothing, and
/// the call would be `git__free(NULL)`, which reads `git__allocator` without
/// synchronisation against the first `git_libgit2_init` another thread may be running.
/// Nothing orders such an array after that initialisation, since nothing allocated for
/// it. A non-NULL array is passed even when empty: libgit2 allocated it.
///
/// Wraps: git_strarray_dispose
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct StrArrayDispose;

// SAFETY: `git_strarray_dispose` frees `strings[0..count)` and `strings` exactly once
// and never the struct. It accepts every state the safe API reaches behind an owner:
// an array `set_strings` stored from an `OdbDataStrVec`, i.e. non-NULL strings in an
// array from libgit2's allocator (whose allocation initialised the library, so the
// allocator it reads is settled); `truncate` frees the strings it drops before
// lowering `count`. The array without a buffer, which `new`, `take_strings` and
// `set_strings` of an empty run leave, owns nothing and is released without C.
unsafe impl CDispose<StrArray> for StrArrayDispose {
    #[inline]
    unsafe fn c_dispose(&self, ptr: NonNull<StrArray>) {
        let raw: *mut ffi::git_strarray = ptr.as_ptr().cast();
        // SAFETY: per `c_dispose`'s contract `raw` addresses a live `git_strarray`.
        let strings = unsafe { addr_of!((*raw).strings).read() };
        if strings.is_null() {
            return;
        }
        // SAFETY: `raw` is live and, per the handles' invariant, owns `strings` and its
        // first `count` strings, all from libgit2's allocator; the caller gives the
        // value up, so they are released once.
        unsafe { ffi::git_strarray_dispose(raw) }
    }
}

/// An owned [`StrArray`], held inline; its strings are released by
/// `git_strarray_dispose` on drop.
pub type StrArrayVal = CVal<StrArray, StrArrayDispose>;

/// The array without a buffer: `git_strarray array = {0}`.
const EMPTY: ffi::git_strarray = ffi::git_strarray {
    strings: ptr::null_mut(),
    count: 0,
};

impl StrArray {
    /// An empty array: no strings, nothing allocated, as `git_strarray array = {0}`.
    /// The storage libgit2 fills a result into.
    pub fn new() -> StrArrayVal {
        CVal::new(Self(EMPTY, PhantomData))
    }

    /// An array owning `strings`, as libgit2 hands one out: the run becomes the
    /// array's, released by `git_strarray_dispose`.
    pub fn from_strings(strings: OdbDataStrVec) -> StrArrayVal {
        let mut array = Self::new();
        array.as_mut().set_strings(strings);
        array
    }

    /// An array holding copies of `strs`, in order, in libgit2's allocator, as
    /// `git_strarray_copy` makes one; empty and unallocated for no strings.
    ///
    /// `None` when an allocation fails, or libgit2 cannot be initialised.
    pub fn from_strs<S: AsRef<CStr>>(strs: &[S]) -> Option<StrArrayVal> {
        OdbDataStrVec::dup_all(strs).map(Self::from_strings)
    }
}

impl<'a> StrArrayRef<'a> {
    /// The number of strings; 0 without a buffer, whatever `count` says, as
    /// [`get`](Self::get) and [`iter`](Self::iter) read it.
    ///
    /// Field: git_strarray.count
    pub fn len(&self) -> usize {
        let array = self.as_ptr();
        // SAFETY: reads through the shared handle's pointer to a live `git_strarray`;
        // any `size_t` is a valid `usize`.
        let (strings, count) = unsafe {
            (
                addr_of!((*array).strings).read(),
                addr_of!((*array).count).read(),
            )
        };
        if strings.is_null() { 0 } else { count }
    }

    /// Whether the array holds no strings.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The string at `index`, borrowed in place for as long as the array is; `None`
    /// past the end.
    ///
    /// Field: git_strarray.strings
    pub fn get(&self, index: usize) -> Option<&'a CStr> {
        (index < self.len()).then(|| self.at(index))
    }

    /// The strings, in order, each borrowed in place for as long as the array is.
    ///
    /// Field: git_strarray.strings
    pub fn iter(&self) -> Iter<'a> {
        Iter {
            array: *self,
            range: 0..self.len(),
        }
    }

    /// A copy of the array, its strings in a fresh array of libgit2's allocator, as
    /// `git_strarray_copy` makes one; empty and unallocated for an empty array. A
    /// [`StrArrayView`] over borrowed strings copies into an owned array this way.
    ///
    /// `None` when an allocation fails, or libgit2 cannot be initialised.
    ///
    /// The owner itself has no `Clone`: a byte copy would dispose the strings twice.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::core::strarray::{StrArray, StrArrayVal};
    ///
    /// let array = StrArray::new();
    /// let copy: StrArrayVal = array.clone();
    /// ```
    pub fn to_str_array(&self) -> Option<StrArrayVal> {
        let copies: Vec<&'a CStr> = self.iter().collect();
        StrArray::from_strs(&copies)
    }

    /// Whether the array has a buffer (`strings` is not NULL), even an empty one: the
    /// mark of a result libgit2 handed over, where a failing producer leaves NULL.
    pub(crate) fn has_buffer(&self) -> bool {
        // SAFETY: a read through the shared handle's pointer to a live `git_strarray`.
        let strings = unsafe { addr_of!((*self.as_ptr()).strings).read() };
        !strings.is_null()
    }

    /// The string at `index`, which must be below [`len`](Self::len).
    fn at(&self, index: usize) -> &'a CStr {
        debug_assert!(index < self.len());
        // SAFETY: a read through the shared handle's pointer to a live `git_strarray`.
        let strings = unsafe { addr_of!((*self.as_ptr()).strings).read() };
        // SAFETY: `index < len`, so `strings` is non-NULL and, per the handles'
        // invariant, addresses at least `count > index` initialised elements.
        let s = unsafe { strings.add(index).read() };
        // SAFETY: per the handles' invariant every element below `count` is a non-NULL
        // NUL-terminated string that outlives the handle, i.e. `'a`; nothing writes it
        // while a shared handle lives, since writes need the exclusive one.
        unsafe { CStr::from_ptr(s) }
    }
}

impl<'a> IntoIterator for StrArrayRef<'a> {
    type Item = &'a CStr;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl fmt::Debug for StrArrayRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl fmt::Debug for StrArrayMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// The strings of a [`StrArrayRef`], in order, each borrowed for the array's lifetime.
///
/// Handwritten
#[derive(Clone)]
pub struct Iter<'a> {
    array: StrArrayRef<'a>,
    range: Range<usize>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = &'a CStr;

    fn next(&mut self) -> Option<&'a CStr> {
        self.range.next().map(|i| self.array.at(i))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }

    fn nth(&mut self, n: usize) -> Option<&'a CStr> {
        self.range.nth(n).map(|i| self.array.at(i))
    }
}

impl DoubleEndedIterator for Iter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.range.next_back().map(|i| self.array.at(i))
    }
}

impl ExactSizeIterator for Iter<'_> {}

impl FusedIterator for Iter<'_> {}

impl fmt::Debug for Iter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.clone()).finish()
    }
}

impl StrArrayMut<'_> {
    /// Replaces the strings, and with them the count, releasing the ones held before.
    ///
    /// Field: git_strarray.strings
    /// Field: git_strarray.count
    pub fn set_strings(&mut self, strings: OdbDataStrVec) {
        drop(self.take_strings());
        let (strings, count) = strings.into_raw_parts();
        let array = self.as_mut_ptr();
        // SAFETY: writes through the exclusive handle's pointer to a live
        // `git_strarray`, whose previous strings `take_strings` released. `strings` is
        // NULL with `count == 0`, or `count` non-NULL strings in an array of libgit2's
        // allocator whose ownership the array takes over, which keeps the handles'
        // invariant.
        unsafe {
            addr_of_mut!((*array).strings).write(strings);
            addr_of_mut!((*array).count).write(count);
        }
    }

    /// Takes the strings out, leaving the array empty and without a buffer: what
    /// `git_strarray_dispose` would release, owned. Edit them there (sort, push,
    /// remove) and [`set_strings`](Self::set_strings) them back.
    ///
    /// Field: git_strarray.strings
    pub fn take_strings(&mut self) -> OdbDataStrVec {
        let array = self.as_mut_ptr();
        // SAFETY: reads, then writes the array without a buffer, through the exclusive
        // handle's pointer to a live `git_strarray`.
        let (strings, count) = unsafe {
            let taken = addr_of!(*array).read();
            addr_of_mut!(*array).write(EMPTY);
            (taken.strings, taken.count)
        };
        // SAFETY: per the handles' invariant `strings` is NULL (owning nothing, which
        // `from_raw_parts` reads as the empty run only when `count == 0`; otherwise it
        // gives `None`, the empty run below, and nothing is lost), or an array of
        // libgit2's allocator holding at least `count` non-NULL strings from the same
        // allocator, owned by the array and now handed over whole, since the array no
        // longer refers to them. `OdbDataArrayFree` frees with `git__free`, which
        // ignores the length, so spare capacity beyond `count` is released with it.
        unsafe { OdbDataStrVec::from_raw_parts(strings, count) }.unwrap_or_default()
    }

    /// Shortens the array to its first `len` strings, releasing the rest; nothing
    /// changes when it holds no more than that. The array buffer is kept, and released
    /// whole on dispose.
    ///
    /// Field: git_strarray.count
    pub fn truncate(&mut self, len: usize) {
        let old = self.as_ref().len();
        if len >= old {
            return;
        }
        let array = self.as_mut_ptr();
        // SAFETY: reads, then writes the shorter count, through the exclusive handle's
        // pointer to a live `git_strarray`; `len < old` means `strings` is non-NULL, and
        // a smaller count keeps it at least `count` long.
        let strings = unsafe {
            addr_of_mut!((*array).count).write(len);
            addr_of!((*array).strings).read()
        };
        for i in len..old {
            // SAFETY: `i < old`, the count before, so per the handles' invariant the
            // element is a non-NULL string of libgit2's allocator the array owned; the
            // array's count no longer covers it, so it is released here, once.
            drop(unsafe { OdbDataStr::from_raw(strings.add(i).read()) });
        }
    }
}

/// An argument array (`git_strarray`) over borrowed strings, for the `const
/// git_strarray *` parameters and by-value options fields that libgit2 only reads.
///
/// The `char *` array is Rust's own, and the strings stay the caller's: nothing is
/// allocated from libgit2, nothing reaches `git_strarray_dispose`, and the view only
/// hands out the shared [`StrArrayRef`], so no setter can free what it borrows. An
/// empty view's `strings` is NULL. [`StrArrayRef::to_str_array`] copies it into an
/// owned array.
///
/// There is no exclusive handle, so nothing can replace or free the borrowed strings:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::core::strarray::StrArrayView;
///
/// let mut view = StrArrayView::default();
/// view.as_mut().truncate(0);
/// ```
///
/// Wraps: git_strarray
/// Handwritten
pub struct StrArrayView<'a> {
    /// `strings` addresses `_ptrs`' buffer, or is NULL when empty.
    array: StrArray,
    /// The `char *` array, one pointer per borrowed string; never touched after
    /// construction, so `array`'s pointer into its buffer stays valid when it moves.
    _ptrs: Vec<*mut c_char>,
    _strs: PhantomData<&'a CStr>,
}

// SAFETY: a view is a `Vec` of pointers to strings borrowed as `&'a CStr`, which is
// `Send` and `Sync`, and a `git_strarray` pointing at that `Vec`'s buffer; nothing
// writes either after construction, and the strings are never freed through it.
unsafe impl Send for StrArrayView<'_> {}
// SAFETY: as above: shared access only reads.
unsafe impl Sync for StrArrayView<'_> {}

impl<'a> StrArrayView<'a> {
    /// A view of `strs`, in order. Allocates the `char *` array, in Rust.
    pub fn new<S: AsRef<CStr>>(strs: &'a [S]) -> Self {
        strs.iter().map(AsRef::as_ref).collect()
    }

    /// The shared handle, for reading the strings or passing the array to C.
    pub fn as_ref(&self) -> StrArrayRef<'_> {
        self.array.as_ref()
    }
}

impl<'a> FromIterator<&'a CStr> for StrArrayView<'a> {
    fn from_iter<I: IntoIterator<Item = &'a CStr>>(strs: I) -> Self {
        let mut ptrs: Vec<*mut c_char> = strs.into_iter().map(|s| s.as_ptr().cast_mut()).collect();
        let raw = if ptrs.is_empty() {
            EMPTY
        } else {
            ffi::git_strarray {
                strings: ptrs.as_mut_ptr(),
                count: ptrs.len(),
            }
        };
        Self {
            array: StrArray(raw, PhantomData),
            _ptrs: ptrs,
            _strs: PhantomData,
        }
    }
}

impl Default for StrArrayView<'_> {
    fn default() -> Self {
        Self::new::<&CStr>(&[])
    }
}

impl fmt::Debug for StrArrayView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `StrArray` is a `git_strarray`, so C reads and writes a value Rust holds in place,
// and the owner adds nothing to it.
const _: () = assert!(
    size_of::<StrArray>() == size_of::<ffi::git_strarray>()
        && align_of::<StrArray>() == align_of::<ffi::git_strarray>()
        && size_of::<StrArrayVal>() == size_of::<ffi::git_strarray>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::raw::OdbDataStrFree;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use crate::libgit2::util::sys::errors::runtime_pinned;
    use core::ffi::{c_int, c_void};
    use core::mem::size_of;
    use ffibox::CDispose;
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

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, refusing everything while `REFUSING` is set.
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
            if REFUSING.load(SeqCst) {
                return None;
            }
            let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
            // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
            NonNull::new(unsafe { realloc(p, len) }.cast())
        }
        unsafe fn free(p: NonNull<u8>) {
            // SAFETY: `p` is a live libc block the caller gives up.
            unsafe { free(p.as_ptr().cast()) }
        }
    }

    /// Installs [`Refusing`] before anything initialises libgit2.
    fn install_refusing_allocator() {
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

    /// The strings of `array`, copied out as bytes.
    fn contents(array: StrArrayRef<'_>) -> Vec<Vec<u8>> {
        array.iter().map(|s| s.to_bytes().to_vec()).collect()
    }

    fn expected(strs: &[&str]) -> Vec<Vec<u8>> {
        strs.iter().map(|s| s.as_bytes().to_vec()).collect()
    }

    /// Owned C strings and a `char *` array over them, in Rust's allocator, as C code
    /// builds an argument array.
    struct RustStrings {
        _owned: Vec<CString>,
        ptrs: Vec<*mut c_char>,
    }

    impl RustStrings {
        fn new(strs: &[&str]) -> Self {
            let owned: Vec<CString> = strs.iter().map(|s| CString::new(*s).unwrap()).collect();
            let ptrs = owned.iter().map(|s| s.as_ptr().cast_mut()).collect();
            Self {
                _owned: owned,
                ptrs,
            }
        }

        fn raw(&mut self) -> ffi::git_strarray {
            ffi::git_strarray {
                strings: self.ptrs.as_mut_ptr(),
                count: self.ptrs.len(),
            }
        }
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<StrArray>(), size_of::<ffi::git_strarray>());
        assert_eq!(size_of::<StrArrayVal>(), size_of::<ffi::git_strarray>());
        assert_eq!(
            size_of::<StrArrayRef<'_>>(),
            size_of::<*const ffi::git_strarray>()
        );
        assert_eq!(
            size_of::<StrArrayMut<'_>>(),
            size_of::<*const ffi::git_strarray>()
        );
        assert_eq!(size_of::<StrArrayDispose>(), 0);
    }

    /// The empty array's accessors over a bare value, without libgit2, so Miri runs
    /// them; nothing here allocates or disposes.
    #[test]
    fn empty_array_accessors_in_rust_storage() {
        let mut bare = StrArray::new().into_inner();
        let mut m = bare.as_mut();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().is_empty());
        assert!(m.as_ref().get(0).is_none());
        assert!(m.as_ref().iter().next().is_none());
        m.truncate(0);
        m.truncate(usize::MAX);
        assert!(m.take_strings().is_empty());
        m.set_strings(OdbDataStrVec::new());
        assert_eq!(m.as_ref().len(), 0);
        assert_eq!(format!("{m:?}"), "[]");
        assert_eq!(format!("{:?}", bare.as_ref()), "[]");
        // SAFETY: a read of the value Rust holds: still the array without a buffer.
        let raw = unsafe { addr_of!(bare.0).read() };
        assert!(raw.strings.is_null() && raw.count == 0);
    }

    /// Without a buffer the array is empty and owns nothing, whatever `count` says:
    /// every accessor agrees, and the dispose policy never calls libgit2 (Miri aborts
    /// on any foreign call, so this proves the empty array's drop makes none).
    #[test]
    fn null_buffer_is_empty_and_disposes_without_c() {
        let mut bare = StrArray::new().into_inner();
        // SAFETY: `bare` is a live array Rust owns; the policy's contract accepts the
        // array without a buffer, which owns nothing.
        unsafe { StrArrayDispose.c_dispose(NonNull::from(&mut bare)) };
        assert!(bare.as_ref().is_empty());
        drop(StrArray::new());

        let mut broken = ffi::git_strarray {
            strings: ptr::null_mut(),
            count: 3,
        };
        // SAFETY: `broken` is live for the handle's use; nothing else reaches it.
        let mut m = unsafe { StrArrayMut::from_ptr(&mut broken) }.unwrap();
        assert_eq!(m.as_ref().len(), 0);
        assert!(m.as_ref().get(0).is_none() && m.as_ref().iter().len() == 0);
        assert_eq!(format!("{m:?}"), "[]");
        m.truncate(1);
        assert!(m.take_strings().is_empty());
        assert_eq!((broken.strings, broken.count), (ptr::null_mut(), 0));
        broken.count = 3;
        // SAFETY: `broken` is live and holds no buffer; nothing is released.
        unsafe { StrArrayDispose.c_dispose(NonNull::from(&mut broken).cast::<StrArray>()) };
        assert_eq!((broken.strings, broken.count), (ptr::null_mut(), 3));
    }

    /// The shared handle's reads over a C-style argument array Rust allocated, so Miri
    /// checks the projections and the borrowed strings' bounds.
    #[test]
    fn shared_handle_over_rust_storage() {
        let mut strings = RustStrings::new(&["src/*.c", "", "include/git2/strarray.h"]);
        let mut raw = strings.raw();
        // SAFETY: `raw` is live for the handle's use and addresses 3 strings the test
        // keeps alive; the handle only reads.
        let r = unsafe { StrArrayRef::from_ptr(&mut raw) }.unwrap();
        assert_eq!(r.len(), 3);
        assert!(!r.is_empty());
        assert_eq!(r.get(0), Some(c"src/*.c"));
        assert_eq!(r.get(1), Some(c""));
        assert_eq!(r.get(2), Some(c"include/git2/strarray.h"));
        assert_eq!(r.get(3), None);
        assert_eq!(r.get(usize::MAX), None);
        assert_eq!(
            contents(r),
            expected(&["src/*.c", "", "include/git2/strarray.h"])
        );
        assert_eq!(
            format!("{r:?}"),
            r#"["src/*.c", "", "include/git2/strarray.h"]"#
        );
        // A non-NULL array with no count is empty, as libgit2's eager vectors leave it.
        raw.count = 0;
        // SAFETY: as above.
        let r = unsafe { StrArrayRef::from_ptr(&mut raw) }.unwrap();
        assert!(r.is_empty() && r.get(0).is_none());
        assert!(r.has_buffer());
        assert!(!StrArray::new().as_ref().has_buffer());
    }

    /// The iterator walks both ends, skips, reports its exact length and stays done,
    /// over Rust storage, so Miri runs it.
    #[test]
    fn iterator_is_exact_double_ended_and_fused() {
        let strs = [c"a", c"bc", c"def", c"ghij"];
        let view = StrArrayView::new(&strs);
        let r = view.as_ref();
        let mut it = r.iter();
        assert_eq!(it.len(), 4);
        assert_eq!(it.size_hint(), (4, Some(4)));
        assert_eq!(it.next(), Some(c"a"));
        assert_eq!(it.next_back(), Some(c"ghij"));
        assert_eq!(format!("{it:?}"), r#"["bc", "def"]"#);
        assert_eq!(it.len(), 2);
        let rest = it.clone();
        assert_eq!(it.nth(1), Some(c"def"));
        assert_eq!(it.next(), None);
        assert_eq!(it.next_back(), None);
        assert_eq!(it.next(), None);
        assert_eq!(rest.rev().collect::<Vec<_>>(), [c"def", c"bc"]);
        assert_eq!(r.iter().nth(4), None);
        assert_eq!(r.into_iter().collect::<Vec<_>>(), strs);
        let mut n = 0;
        for s in r {
            assert_eq!(s, strs[n]);
            n += 1;
        }
        assert_eq!(n, 4);
    }

    /// A view lays out the borrowed strings as C reads them: NULL when empty, the
    /// strings' own addresses otherwise, still valid after the view moves. Rust-only.
    #[test]
    fn views_point_at_the_borrowed_strings() {
        let empty = StrArrayView::default();
        // SAFETY: a read through the view's own handle.
        let raw = unsafe { empty.as_ref().as_ptr().read() };
        assert!(raw.strings.is_null() && raw.count == 0);
        assert!(StrArrayView::new::<CString>(&[]).as_ref().is_empty());
        assert_eq!(format!("{empty:?}"), "[]");

        let owned = [CString::new("one").unwrap(), CString::new("two").unwrap()];
        let view = StrArrayView::new(&owned);
        let moved = std::hint::black_box(view);
        // SAFETY: a read through the view's own handle.
        let raw = unsafe { moved.as_ref().as_ptr().read() };
        assert_eq!(raw.count, 2);
        for (i, s) in owned.iter().enumerate() {
            // SAFETY: the view's array holds `count` pointers.
            let element = unsafe { raw.strings.add(i).read() };
            assert_eq!(element.cast_const(), s.as_ptr());
        }
        assert_eq!(contents(moved.as_ref()), expected(&["one", "two"]));
        assert_eq!(format!("{moved:?}"), r#"["one", "two"]"#);

        let collected: StrArrayView<'_> = owned.iter().map(CString::as_c_str).rev().collect();
        assert_eq!(contents(collected.as_ref()), expected(&["two", "one"]));
    }

    /// The exclusive handle's moves over Rust storage: every run taken out is handed
    /// back raw, so nothing reaches libgit2's free and Miri checks the projections.
    #[test]
    fn exclusive_handle_moves_over_rust_storage() {
        let mut strings = RustStrings::new(&["x", "y", "z"]);
        let base = strings.ptrs.as_mut_ptr();
        let mut raw = strings.raw();
        // SAFETY: `raw` is live for the handle's use and addresses 3 strings, which the
        // test keeps alive; no other handle reaches it meanwhile. The strings are never
        // released through the handle: every take below hands them back raw.
        let mut m = unsafe { StrArrayMut::from_ptr(&mut raw) }.unwrap();
        assert_eq!(contents(m.as_ref()), expected(&["x", "y", "z"]));
        let (taken, count) = m.take_strings().into_raw_parts();
        assert_eq!((taken, count), (base, 3));
        assert!(m.as_ref().is_empty());
        // SAFETY: the test's own array of 3 strings; handed back raw below, never
        // dropped, so `OdbDataStrFree` never frees it.
        let adopted = unsafe { OdbDataStrVec::from_raw_parts(taken, 3) }.unwrap();
        m.set_strings(adopted);
        assert_eq!(m.as_ref().get(2), Some(c"z"));
        let (taken, count) = m.take_strings().into_raw_parts();
        assert_eq!((taken, count), (base, 3));
        assert_eq!((raw.strings, raw.count), (ptr::null_mut(), 0));
    }

    /// Copies, edits and replacements each release every string once (LSan, in the
    /// asan variant, flags a leak, ASan a double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owned_arrays_copy_edit_and_release() {
        let mut array = StrArray::from_strs(&[c"refs/heads/main", c"", c"refs/tags/v1"]).unwrap();
        assert_eq!(array.as_ref().len(), 3);
        assert_eq!(
            contents(array.as_ref()),
            expected(&["refs/heads/main", "", "refs/tags/v1"])
        );

        let copy = array.as_ref().to_str_array().unwrap();
        for (a, b) in array.as_ref().iter().zip(copy.as_ref()) {
            assert_eq!(a, b);
            assert_ne!(a.as_ptr(), b.as_ptr(), "a copy has strings of its own");
        }

        array.as_mut().truncate(5);
        assert_eq!(array.as_ref().len(), 3);
        array.as_mut().truncate(1);
        assert_eq!(contents(array.as_ref()), expected(&["refs/heads/main"]));
        // A truncated array copies only what is left, and releases whole.
        let short = array.as_ref().to_str_array().unwrap();
        assert_eq!(contents(short.as_ref()), expected(&["refs/heads/main"]));

        // Edits happen on the taken run, which goes back without a copy.
        let mut strings = copy
            .as_ref()
            .to_str_array()
            .unwrap()
            .as_mut()
            .take_strings();
        strings
            .as_mut_slice()
            .sort_by(|a, b| a.as_c_str().cmp(b.as_c_str()));
        let mut v = strings.into_vec();
        v.push(OdbDataStrFree::dup(c"refs/notes/x").unwrap());
        assert_eq!(v.remove(0).as_c_str(), c"", "sorted first");
        array
            .as_mut()
            .set_strings(OdbDataStrVec::from_vec(v).unwrap());
        assert_eq!(
            contents(array.as_ref()),
            expected(&["refs/heads/main", "refs/tags/v1", "refs/notes/x"])
        );
        let taken = array.as_mut().take_strings();
        assert_eq!(taken.len(), 3);
        assert!(array.as_ref().is_empty() && array.as_ref().get(0).is_none());
        let moved = StrArray::from_strings(taken);
        assert_eq!(moved.as_ref().get(2), Some(c"refs/notes/x"));
        assert_eq!(
            format!("{:?}", moved.as_ref()),
            r#"["refs/heads/main", "refs/tags/v1", "refs/notes/x"]"#
        );
        assert_eq!(format!("{:?}", array.as_mut()), "[]");

        let empty = StrArray::from_strs::<&CStr>(&[]).unwrap();
        assert!(empty.as_ref().is_empty());
        assert!(empty.as_ref().to_str_array().unwrap().as_ref().is_empty());

        // A view's borrowed strings copy into an owned array.
        let names = [c"origin", c"upstream"];
        let owned = StrArrayView::new(&names).as_ref().to_str_array().unwrap();
        assert_eq!(contents(owned.as_ref()), expected(&["origin", "upstream"]));
    }

    /// The dispose policy, driven by hand on a bare value: its strings are freed once
    /// (LSan flags a leak otherwise) and the struct is zeroed.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn dispose_policy_releases_the_strings() {
        let mut bare = StrArray::from_strs(&[c"a", c"b"]).unwrap().into_inner();
        assert_eq!(bare.as_ref().len(), 2);
        // SAFETY: `bare` is a live array Rust owns, disposed once.
        unsafe { StrArrayDispose.c_dispose(NonNull::from(&mut bare)) };
        // SAFETY: a read of the value Rust holds, which C zeroed.
        let raw = unsafe { addr_of!(bare.0).read() };
        assert!(raw.strings.is_null() && raw.count == 0);
    }

    /// `GIT_OPT_GET_EXTENSIONS` into `ptr`; the option's result is a `git_vector`'s
    /// detached array of `git__strdup` copies.
    ///
    /// # Safety
    ///
    /// `ptr` must address a live `git_strarray` that owns nothing.
    unsafe fn get_extensions(ptr: *mut ffi::git_strarray) {
        // SAFETY: `GIT_OPT_GET_EXTENSIONS` takes a `git_strarray *`, which it overwrites;
        // per this function's contract it held nothing to leak.
        let rc = unsafe { ffi::git_libgit2_opts(ffi::GIT_OPT_GET_EXTENSIONS as c_int, ptr) };
        assert_eq!(rc, 0);
    }

    /// libgit2 fills an owner's storage in place, as its producers' wrappers will
    /// through the crate-internal seam: the array it hands over is read through the
    /// handles, edited, and released by the owner's drop (LSan flags a leak, ASan an
    /// allocator mismatch).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_fills_an_owner_in_place() {
        assert!(runtime_pinned());
        let mut array = StrArray::new();
        // SAFETY: the owner's live storage, reached through its exclusive handle for
        // the call, owns nothing. On success it owns an array and strings from
        // libgit2's allocator, which keeps the handles' invariant.
        unsafe { get_extensions(array.as_mut().as_mut_ptr()) };
        let r = array.as_ref();
        assert!(r.len() >= 6, "{r:?}");
        assert!(r.iter().any(|s| s == c"objectformat"), "{r:?}");
        let copy = r.to_str_array().unwrap();
        assert_eq!(contents(copy.as_ref()), contents(array.as_ref()));
        array.as_mut().truncate(2);
        assert_eq!(array.as_ref().len(), 2);
        array
            .as_mut()
            .set_strings(copy.into_inner().as_mut().take_strings());
        assert!(array.as_ref().len() >= 6);
    }

    /// libgit2's eager vectors hand out a non-NULL array with no strings when there is
    /// nothing to list (every builtin extension disabled): the owner reads it as empty
    /// and still releases the array (LSan flags a leak otherwise).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn allocated_empty_results_are_released() {
        isolated(
            concat!(module_path!(), "::allocated_empty_results_are_released"),
            || {
                assert!(runtime_pinned());
                let off = [
                    c"!noop",
                    c"!objectformat",
                    c"!worktreeconfig",
                    c"!preciousobjects",
                    c"!refstorage",
                    c"!relativeworktrees",
                ];
                let ptrs: Vec<*const c_char> = off.iter().map(|s| s.as_ptr()).collect();
                // SAFETY: `GIT_OPT_SET_EXTENSIONS` takes an array of `len` strings, which
                // it copies; this child owns libgit2's globals.
                let rc = unsafe {
                    ffi::git_libgit2_opts(
                        ffi::GIT_OPT_SET_EXTENSIONS as c_int,
                        ptrs.as_ptr(),
                        ptrs.len(),
                    )
                };
                assert_eq!(rc, 0);
                let mut array = StrArray::new();
                // SAFETY: as in `c_fills_an_owner_in_place`.
                unsafe { get_extensions(array.as_mut().as_mut_ptr()) };
                // SAFETY: a read through the owner's handle.
                let raw = unsafe { array.as_ref().as_ptr().read() };
                assert!(!raw.strings.is_null() && raw.count == 0);
                assert!(array.as_ref().is_empty() && array.as_ref().get(0).is_none());
                assert!(array.as_ref().to_str_array().unwrap().as_ref().is_empty());

                // Taken out, the allocated empty run is released by its owner instead.
                let mut again = StrArray::new();
                // SAFETY: as above.
                unsafe { get_extensions(again.as_mut().as_mut_ptr()) };
                let taken = again.as_mut().take_strings();
                assert!(taken.is_empty() && !taken.as_ptr().is_null());
            },
        );
    }

    /// Handles over a C-owned `git_strarray`, as libgit2 hands a callback one: the
    /// setters' strings are what `git_strarray_dispose` releases, and C's strings what
    /// they release (ASan flags an allocator mismatch, LSan a leak).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn handles_over_a_c_owned_array() {
        assert!(runtime_pinned());
        let mut raw = EMPTY;
        // SAFETY: `raw` is live and owns nothing.
        unsafe { get_extensions(&mut raw) };
        let count = raw.count;
        // SAFETY: `raw` is live for the handle's use and owns its strings (libgit2's
        // result); no other handle reaches it meanwhile.
        let mut m = unsafe { StrArrayMut::from_ptr(&mut raw) }.unwrap();
        assert_eq!(m.as_ref().len(), count);
        m.truncate(count - 1);
        // Replacing frees C's array and its strings through `git__free`.
        m.set_strings(OdbDataStrVec::dup_all(&[c"1", c"2", c"3"]).unwrap());
        assert_eq!(raw.count, 3);
        // SAFETY: `raw` is live and only read here.
        let r = unsafe { StrArrayRef::from_ptr(&mut raw) }.unwrap();
        assert_eq!(contents(r), expected(&["1", "2", "3"]));
        // C releases the strings Rust stored.
        // SAFETY: `raw` owns its strings, from libgit2's allocator, released once.
        unsafe { ffi::git_strarray_dispose(&mut raw) };
        assert_eq!((raw.strings, raw.count), (ptr::null_mut(), 0));
    }

    /// libgit2 reads a view, through the handle a `const git_strarray *` wrapper will
    /// pass, exactly as it reads an argument array C code builds: `git_pathspec_new`
    /// compiles the same patterns from both (and the owned copy), NULL for the empty
    /// view; it neither writes nor frees the view's array (ASan flags a Rust-allocated
    /// array freed by `git__free`).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn c_reads_a_view_as_an_argument_array() {
        assert!(runtime_pinned());
        let paths = [
            c"src/a.c",
            c"src/skip.c",
            c"docs/x.md",
            c"include/a.h",
            c"src/sub/b.c",
            c"SRC/A.C",
        ];
        let matches = |spec: *const ffi::git_strarray| -> Vec<(c_int, c_int)> {
            let mut ps = ptr::null_mut();
            // SAFETY: `ps` is a live slot; `spec` is NULL or a live array of live
            // strings, read for the call only.
            let rc = unsafe { ffi::git_pathspec_new(&mut ps, spec) };
            assert_eq!(rc, 0);
            let out = paths
                .iter()
                .map(|p| {
                    let flags = ffi::GIT_PATHSPEC_IGNORE_CASE;
                    // SAFETY: `ps` is a live pathspec and `p` NUL-terminated.
                    unsafe {
                        (
                            ffi::git_pathspec_matches_path(ps, 0, p.as_ptr()),
                            ffi::git_pathspec_matches_path(ps, flags, p.as_ptr()),
                        )
                    }
                })
                .collect();
            // SAFETY: `ps` is live and released once.
            unsafe { ffi::git_pathspec_free(ps) };
            out
        };
        let safe_matches = |spec: StrArrayRef<'_>| -> Vec<(c_int, c_int)> {
            let ps = crate::libgit2::index::pathspec::Pathspec::new(spec).unwrap();
            paths
                .iter()
                .map(|p| {
                    // SAFETY: the safe constructor's owner keeps the compiled pathspec
                    // alive; matching only reads it and the terminated input path.
                    unsafe {
                        (
                            ffi::git_pathspec_matches_path(ps.as_c_ptr(), 0, p.as_ptr()),
                            ffi::git_pathspec_matches_path(
                                ps.as_c_ptr(),
                                ffi::GIT_PATHSPEC_IGNORE_CASE,
                                p.as_ptr(),
                            ),
                        )
                    }
                })
                .collect()
        };
        let text = ["src/*.c", "!src/skip.c", "docs/"];
        let patterns: Vec<CString> = text.iter().map(|s| CString::new(*s).unwrap()).collect();
        let view = StrArrayView::new(&patterns);
        let mut by_hand = RustStrings::new(&text);
        let raw = by_hand.raw();
        let expected_matches = matches(&raw);
        assert!(
            expected_matches.iter().any(|m| m.0 == 1) && expected_matches.iter().any(|m| m.0 == 0)
        );
        assert_eq!(safe_matches(view.as_ref()), expected_matches);
        let owned = view.as_ref().to_str_array().unwrap();
        assert_eq!(safe_matches(owned.as_ref()), expected_matches);
        assert_eq!(contents(view.as_ref()), expected(&text));
        // SAFETY: a read through the view's own handle: C left the array as built.
        let after = unsafe { view.as_ref().as_ptr().read() };
        assert_eq!(after.count, 3);
        // The empty view is NULL, which libgit2 reads as the empty pathspec.
        let empty = StrArrayView::default();
        assert_eq!(safe_matches(empty.as_ref()), matches(ptr::null()));
        assert_eq!(safe_matches(StrArray::new().as_ref()), matches(ptr::null()));
    }

    /// A failed allocation is `None` from every copying constructor, releases the
    /// partial copy (LSan), and leaves the source untouched.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copies_out_of_memory_are_none() {
        isolated(
            concat!(module_path!(), "::copies_out_of_memory_are_none"),
            || {
                install_refusing_allocator();
                assert!(runtime_pinned());
                let array = StrArray::from_strs(&[c"a", c"b"]).unwrap();
                REFUSING.store(true, SeqCst);
                let copied = StrArray::from_strs(&[c"c"]).is_none();
                let cloned = array.as_ref().to_str_array().is_none();
                // Nothing to allocate for an empty array.
                let empty = StrArray::from_strs::<&CStr>(&[]).is_some();
                REFUSING.store(false, SeqCst);
                assert!(copied && cloned && empty);
                assert_eq!(contents(array.as_ref()), expected(&["a", "b"]));
            },
        );
    }

    /// Without libgit2's runtime nothing can be allocated, but the empty array needs
    /// none, and disposing it never reaches libgit2.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn copies_without_runtime_fail_cleanly() {
        isolated(
            concat!(module_path!(), "::copies_without_runtime_fail_cleanly"),
            || {
                REFUSING.store(true, SeqCst);
                install_refusing_allocator();
                let empty = StrArray::new();
                assert!(StrArray::from_strs(&[c"a"]).is_none());
                assert!(empty.as_ref().to_str_array().is_some());
                let names = [c"x"];
                assert!(StrArrayView::new(&names).as_ref().to_str_array().is_none());
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                drop(empty);
            },
        );
    }
}
