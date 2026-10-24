// Two exclusive handles to one signature cannot coexist.
use {libgit2::libgit2::src::libgit2::signature::Signature, libgit2::libgit2::src::libgit2::signature::Time};

fn main() {
    let mut sig = Signature::new(c"A", c"a@b", 0, 0).unwrap();
    let mut a = sig.as_mut();
    let mut b = sig.as_mut();
    a.set_when(Time::new(1, 0));
    b.set_when(Time::new(2, 0));
}
