// Replacing the name frees the old one, so no borrowed name may be held across it.
use libgit2::libgit2::src::libgit2::signature::Signature;
use libgit2::libgit2::src::libgit2::odb::OdbDataStrFree;

fn main() {
    let mut sig = Signature::new(c"A", c"a@b", 0, 0).unwrap();
    let name = sig.as_ref().name();
    sig.as_mut().set_name(OdbDataStrFree::dup(c"B").unwrap());
    let _ = name;
}
