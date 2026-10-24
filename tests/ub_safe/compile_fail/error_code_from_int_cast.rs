// An `int` cannot be cast into an error enum: `from_raw` is the only way in, and it
// rejects values without a variant.
use {libgit2::libgit2::src::util::errors::ErrorClass, libgit2::libgit2::src::util::errors::ErrorCode};

fn main() {
    let raw: i32 = -2;
    let _ = raw as ErrorCode;
    let _ = 37 as ErrorClass;
}
