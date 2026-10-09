// The time handle projected out of a signature reborrows the exclusive handle it came
// from, so no other write to the signature may happen while it is held.
use libgit2::libgit2::core::types::Signature;
use libgit2::libgit2::raw::OdbDataStrFree;

fn main() {
    let mut sig = Signature::new(c"A", c"a@b", 0, 0).unwrap();
    let mut m = sig.as_mut();
    let mut when = m.when_mut();
    m.set_name(OdbDataStrFree::dup(c"B").unwrap());
    when.set_seconds(1);
}
