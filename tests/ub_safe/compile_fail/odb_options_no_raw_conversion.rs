// `OdbOptions` holds only an `oid_type` libgit2 names (or 0), so no raw `git_odb_options`, whose
// `oid_type` may be any value, converts into one.
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::OdbOptions;

fn main() {
    let raw = ffi::git_odb_options {
        version: ffi::GIT_ODB_OPTIONS_VERSION,
        oid_type: 7,
    };
    let _ = OdbOptions::from(raw);
}
