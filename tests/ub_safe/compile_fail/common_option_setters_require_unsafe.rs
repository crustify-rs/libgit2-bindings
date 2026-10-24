use libgit2::libgit2::src::libgit2::settings;
// The option setters write libgit2 globals its other routines read without
// synchronisation, so safe code cannot call them, nor `cached_memory`.

fn main() {
    settings::set_mwindow_size(1 << 20);
    let _ = settings::set_user_agent(Some(c"agent"));
    settings::set_allocator(None);
    let _ = settings::cached_memory();
}
