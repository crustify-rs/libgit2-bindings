// A borrowed name cannot outlive the signature that owns it.
use libgit2::libgit2::core::types::Signature;

fn main() {
    let name = {
        let sig = Signature::new(c"A", c"a@b", 0, 0).unwrap();
        sig.as_ref().name()
    };
    let _ = name;
}
