//! `OidRef::format_cstr` told an empty buffer apart from a written one by comparing
//! the pointer `git_oid_tostr` returned with the slice's address. An empty slice's
//! address is arbitrary, so one that happens to sit at libgit2's static "" (which C
//! returns for a length of 0) reads as "written", and the wrapper then looks for a
//! NUL in zero bytes and panics. Exits non-zero (the panic) on the affected commit;
//! prints `Ok("")` once the wrapper decides by `out.is_empty()`.

use libgit2::ffi;
use libgit2::libgit2::oid::oid::{Oid, OidType};

fn main() {
    let c_oid = ffi::git_oid {
        type_: 1,
        id: [0; 32],
    };
    let mut scratch = [b'#'; 1];
    // SAFETY: a length of 0 writes nothing through `scratch`; the oid is live.
    let empty = unsafe { ffi::git_oid_tostr(scratch.as_mut_ptr().cast(), 0, &c_oid) };
    println!("libgit2's static \"\" is at {empty:?}");

    // SAFETY: a zero-length slice needs a non-null, aligned pointer and no bytes behind
    // it: nothing is read or written through it, so the static string stays untouched.
    let out: &mut [u8] = unsafe { core::slice::from_raw_parts_mut(empty.cast::<u8>(), 0) };
    let oid = Oid::zero(OidType::Sha1);
    let formatted = oid.as_ref().format_cstr(out);
    println!("{formatted:?}");
    assert_eq!(scratch, [b'#']);
}
