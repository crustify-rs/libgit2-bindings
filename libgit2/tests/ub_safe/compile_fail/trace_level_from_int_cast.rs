// An integer cannot be cast into `TraceLevel`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::core::trace::TraceLevel;

fn main() {
    let raw: u32 = 7;
    let _ = raw as TraceLevel;
    let _ = 0 as TraceLevel;
}
