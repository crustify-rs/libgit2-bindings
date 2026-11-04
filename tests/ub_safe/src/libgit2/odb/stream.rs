//! UB tests for `git2/odb_backend.h`'s `git_odb_stream` and `git_odb_writepack`: the
//! stream constructors on `OdbRef`, refused on an odb without backends.
//!
//! The tests that write, read and finish streams, and those of the writepack, need the
//! stream operations and `git_odb_write_pack`, which are not wrapped yet: they come with
//! those wrappers.

use libgit2::libgit2::src::libgit2::object::ObjectType;
use libgit2::libgit2::src::libgit2::odb::Odb;
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};
use libgit2::libgit2::src::util::errors::ErrorClass;

use crate::fixture::init;

const TYPES: [OidType; 2] = [OidType::Sha1, OidType::Sha256];

#[test]
#[cfg_attr(miri, ignore = "calls libgit2")]
fn streams_on_an_odb_without_backends_fail() {
    init();
    for kind in TYPES {
        let odb = Odb::with_oid_type(kind).unwrap();
        for _ in 0..4 {
            let err = odb
                .as_ref()
                .open_write_stream(4, ObjectType::Blob)
                .unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Odb));
            assert!(err.raw_code() < 0);
            let err = odb
                .as_ref()
                .open_read_stream(Oid::zero(kind).as_ref())
                .unwrap_err();
            assert_eq!(err.class(), Some(ErrorClass::Odb));
        }
    }
}
