// A handle cannot outlive the `Allocator` value it borrows.
use libgit2::libgit2::util::sys::alloc::{Alloc, Allocator};

struct Refuse;
// SAFETY: refuses every request, which `Alloc` allows; nothing is ever live.
unsafe impl Alloc for Refuse {
    fn malloc(_: usize, _: Option<&core::ffi::CStr>, _: i32) -> Option<core::ptr::NonNull<u8>> {
        None
    }
    unsafe fn realloc(
        _: Option<core::ptr::NonNull<u8>>,
        _: usize,
        _: Option<&core::ffi::CStr>,
        _: i32,
    ) -> Option<core::ptr::NonNull<u8>> {
        None
    }
    unsafe fn free(_: core::ptr::NonNull<u8>) {}
}

fn main() {
    let handle = {
        let allocator = Allocator::new::<Refuse>();
        allocator.as_ref()
    };
    let _ = handle.malloc();
}
