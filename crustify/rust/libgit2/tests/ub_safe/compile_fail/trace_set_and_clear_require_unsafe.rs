// libgit2 keeps the trace configuration in a global its traced routines read without
// synchronisation, so safe code can neither install a tracer nor remove it.
use libgit2::libgit2::core::trace::{self, TraceCallback, TraceLevel, Tracer};

struct Quiet;

impl Tracer for Quiet {
    fn trace(_: TraceLevel, _: &std::ffi::CStr) {}
}

fn main() {
    trace::set(TraceLevel::Debug, TraceCallback::new::<Quiet>());
    trace::clear();
}
