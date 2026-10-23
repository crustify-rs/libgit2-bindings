// The option setters write libgit2 globals its other routines read without
// synchronisation, so safe code cannot call them, nor `cached_memory`.
use libgit2::libgit2::core::common;

fn main() {
    common::set_mwindow_size(1 << 20);
    let _ = common::set_user_agent(Some(c"agent"));
    common::set_allocator(None);
    let _ = common::cached_memory();
}
