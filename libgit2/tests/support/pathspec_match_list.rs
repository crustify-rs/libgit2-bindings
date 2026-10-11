//! One fixture for the compiled pathspec type whose safe wrapper has not landed.
#![allow(dead_code)]
use ffibox::{CArc, define_ctype, impl_cdrop};
use libgit2::ffi;
use libgit2::libgit2::index::index::Index;
use std::ffi::CStr;
use std::ptr;

define_ctype!(Compiled, CompiledRef, CompiledMut, ffi::git_pathspec);
#[derive(Clone, Copy, Default)]
pub struct Free;
impl_cdrop!(Free, Compiled, ffi::git_pathspec_free);
pub type CompiledArc = CArc<Compiled, Free>;

pub fn patterns(names: &[&CStr]) -> CompiledArc {
    // The existing safe constructor pins the runtime for the process.
    drop(Index::new().unwrap());
    let mut strings: Vec<_> = names.iter().map(|s| s.as_ptr().cast_mut()).collect();
    let array = ffi::git_strarray {
        strings: strings.as_mut_ptr(),
        count: strings.len(),
    };
    let mut out = ptr::null_mut();
    // SAFETY: initialized terminated strings, borrowed only for this call; C
    // copies them and transfers a counted reference through the writable output.
    assert_eq!(unsafe { ffi::git_pathspec_new(&mut out, &array) }, 0);
    // SAFETY: one fresh initialized reference; CArc explicitly allows the list
    // constructors to retain additional counted references. Free releases one.
    unsafe { CompiledArc::from_c(out) }.unwrap()
}
