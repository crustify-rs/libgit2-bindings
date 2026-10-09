// A handle cannot outlive the `Time` value it borrows.
use ffibox::CZeroable;
use libgit2::libgit2::core::types::Time;

fn main() {
    let handle = {
        let time = Time::zeroed();
        time.as_ref()
    };
    let _ = handle.seconds();
}
