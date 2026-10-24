// Adopting a C function as a trace callback vouches that it is safe to call from any
// thread, which the compiler cannot check.
use std::ffi::c_char;

use libgit2::ffi;
use libgit2::libgit2::src::libgit2::trace::TraceCallback;

unsafe extern "C" fn c_tracer(_: ffi::git_trace_level_t, _: *const c_char) {}

fn main() {
    let _ = TraceCallback::from_fn(c_tracer);
}
