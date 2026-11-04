//! UB tests for `git2/sys/odb_backend.h`'s `git_odb_backend`: the `OdbBackend`
//! template, whose members are all unset, and every call through which fails or does
//! nothing, never calling through NULL.
//!
//! The tests of `OdbBackendBox` over libgit2's packed and loose backends need their
//! constructors (`git_odb_backend_pack`, `git_odb_backend_loose`), which are not wrapped
//! yet: they come with those wrappers.

use std::num::NonZero;
use std::ops::ControlFlow;

use libgit2::libgit2::src::libgit2::odb::{ForeachCallback, Odb, OdbBackend};
use libgit2::libgit2::src::libgit2::oid::OidRef;
use libgit2::libgit2::src::util::errors::{ErrorClass, ErrorCode};

use crate::fixture::init;

#[test]
#[cfg_attr(miri, ignore = "calls C")]
fn the_template_never_calls_an_unset_member() {
    init();
    for _ in 0..8 {
        let mut template = OdbBackend::new();
        let r = template.as_ref();
        assert_eq!(r.version(), 1);
        assert!(!r.in_odb() && !r.has_free() && !r.has_foreach() && !r.has_refresh());
        let mut m = template.as_mut();
        m.refresh().unwrap();
        let mut never = |_: OidRef<'_>| -> ControlFlow<NonZero<i32>> { unreachable!() };
        let err = m
            .foreach(&mut ForeachCallback::new(&mut never))
            .unwrap_err();
        assert_eq!(
            (err.code(), err.class()),
            (Some(ErrorCode::Generic), Some(ErrorClass::Odb))
        );
        let err = m.as_mut().write_multi_pack_index().unwrap_err();
        assert_eq!(err.class(), Some(ErrorClass::Odb));
        assert!(format!("{template:?}").contains("has_read: false"));
        let odb = Odb::new().unwrap();
        assert!(!template.as_ref().belongs_to(odb.as_ref()));
    }
}
