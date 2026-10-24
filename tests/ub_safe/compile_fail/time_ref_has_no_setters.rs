// The shared handle only reads: setters live on `TimeMut`.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::signature::Time;

fn main() {
    let time = Time::zeroed();
    let r = time.as_ref();
    r.set_seconds(1);
}
