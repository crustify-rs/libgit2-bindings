//! The iterator type of `src/libgit2/iterator.c` as `include/git2/notes.h` publishes it
//! (`git_note_iterator` is a `git_iterator`): [`NoteIterator`] and its handles. Opening
//! and advancing one is [`notes`](super::notes)'s.

use core::marker::PhantomData;

use ffibox::{CBorrowedPtr, CBox, CCell};

use crate::ffi;
use crate::libgit2::src::libgit2::notes::NoteIteratorFree;

/// An opaque note iterator. Construct with [`new`](Self::new) or
/// [`from_commit`](Self::from_commit), never as an inline value.
///
/// C retains its own root-tree reference, so the input commit can be dropped after
/// construction. The repository cannot be dropped or exclusively borrowed while
/// this iterator remains usable. There is no public clone, reset or refcount API.
/// Neither iterator nor handles are Send/Sync: traversal mutates unsynchronized
/// state and reaches the thread-bound repository.
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::src::libgit2::iterator::NoteIterator;
/// use libgit2::libgit2::src::libgit2::repository::Repository;
/// let repo = Repository::open(c".").unwrap();
/// let it = NoteIterator::new(repo.as_ref(), None).unwrap();
/// drop(repo);
/// let _ = it.as_ref();
/// ```
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::src::libgit2::iterator::NoteIteratorBox;
/// fn send<T: Send>() {}
/// send::<NoteIteratorBox<'static>>();
/// ```
///
/// Wraps: `git_iterator`
/// Handwritten
#[repr(transparent)]
pub struct NoteIterator<'r>(ffi::git_note_iterator, PhantomData<(*const (), &'r ())>);

/// Shared handle; the public C API has no read-only iterator operations.
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct NoteIteratorRef<'a, 'r>(CBorrowedPtr<'a, NoteIterator<'r>>);

/// Exclusive handle, required for stateful traversal.
/// Handwritten
#[repr(transparent)]
pub struct NoteIteratorMut<'a, 'r>(
    NoteIteratorRef<'a, 'r>,
    PhantomData<&'a mut NoteIterator<'r>>,
);

// SAFETY: the layout is transparent over the opaque FFI type, and both handles
// are transparent over CBorrowedPtr with no Drop. The exclusive handle is
// invariant in the pointee (including its repository lifetime). No C reference
// or inline opaque value is ever constructed.
unsafe impl<'r> CCell for NoteIterator<'r> {
    type C = ffi::git_note_iterator;
    type Mut<'a>
        = NoteIteratorMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = NoteIteratorRef<'a, 'r>
    where
        Self: 'a;
}

impl NoteIteratorRef<'_, '_> {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_note_iterator {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> NoteIteratorMut<'_, 'r> {
    /// Reborrows shared for no longer than this handle.
    #[must_use]
    pub fn as_ref(&self) -> NoteIteratorRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively; the original handle is frozen until this expires.
    pub fn as_mut(&mut self) -> NoteIteratorMut<'_, 'r> {
        NoteIteratorMut(self.0, PhantomData)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_note_iterator {
        self.0.0.as_non_null().as_ptr().cast()
    }
}

/// Sole owner of a note iterator, borrowing its repository for `'r`.
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::src::libgit2::iterator::NoteIteratorBox;
/// fn alias(mut it: NoteIteratorBox<'_>) {
///     let mut exclusive = it.as_mut();
///     let shared = exclusive.as_ref();
///     let second = exclusive.as_mut();
///     let _ = (shared, second);
/// }
/// ```
pub type NoteIteratorBox<'r> = CBox<NoteIterator<'r>, NoteIteratorFree>;
