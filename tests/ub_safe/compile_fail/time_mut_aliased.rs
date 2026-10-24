// Two exclusive handles to one `Time` cannot coexist, nor a shared one beside them.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::signature::Time;

fn main() {
    let mut time = Time::zeroed();
    let mut a = time.as_mut();
    let r = time.as_ref();
    let mut b = time.as_mut();
    a.set_seconds(1);
    b.set_seconds(2);
    let _ = r.seconds();
}
