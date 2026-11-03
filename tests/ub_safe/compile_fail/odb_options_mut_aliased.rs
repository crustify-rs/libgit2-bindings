// Two exclusive handles to one `OdbOptions` cannot coexist, nor a shared one beside them.
use libgit2::libgit2::src::libgit2::odb::OdbOptions;

fn main() {
    let mut opts = OdbOptions::new();
    let mut a = opts.as_mut();
    let r = opts.as_ref();
    let mut b = opts.as_mut();
    a.set_version(1);
    b.set_oid_type(None);
    let _ = r.version();
}
