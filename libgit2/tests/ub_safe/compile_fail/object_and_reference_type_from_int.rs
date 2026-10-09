// An `int` cannot become an object or reference type by a cast or an infallible
// conversion: `from_raw` / `try_from` are the only ways in, and they reject values
// without a variant.
use libgit2::libgit2::core::types::{ObjectType, ReferenceType};

fn main() {
    let raw: i32 = -2;
    let _ = raw as ObjectType;
    let _ = 0u32 as ReferenceType;
    let _: ObjectType = raw.into();
    let _: ReferenceType = 3u32.into();
}
