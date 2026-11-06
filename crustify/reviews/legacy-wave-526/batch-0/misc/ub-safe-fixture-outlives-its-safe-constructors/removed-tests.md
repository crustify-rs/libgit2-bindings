# Tests removed from `tests/ub_safe` under `forbid(unsafe_code)`

The safe UB suite forbids `unsafe_code` at its root from this commit on, as the testing spec
and the conventions ask. Its tests that built their inputs through the fixture's raw
producers, and one test module that called libgit2 directly, are removed: each comes back
in the batch that wraps the producer it needs.

## 118 tests, by file

- `libgit2/annotated_commit.rs` (8): `description_copy_failure_adopts_nothing`, `every_id_type_and_string`, `every_object_and_id_type`, `every_reference_kind_outliving_the_reference`, `every_revision_kind`, `frees_what_libgit2_built_when_a_copy_fails`, `frees_what_libgit2_built_when_the_name_copy_fails`, `results_drop_on_other_threads_while_the_cache_hands_out_more`
- `libgit2/blob.rs` (2): `cached_blobs_are_shared_and_outlive_eviction`, `shared_cached_blobs_lend_their_content`
- `libgit2/buf.rs` (1): `libgit2_results_are_read_reused_and_released_once`
- `libgit2/checkout.rs` (4): `a_baseline_tree_is_read_then_released_after_the_options`, `a_boxed_baseline_index_is_read_then_released_after_the_options`, `a_shared_baseline_index_outlives_the_options_and_the_repository`, `options_sharing_a_baseline_tree_are_dropped_before_it`
- `libgit2/commit.rs` (8, one of its two `libgit2_reads_the_options_for_the_call_only`): `derived_owners_survive_eviction_and_a_disabled_cache`, `every_routine_reads_heap_inputs_for_the_call_only`, `head_at_a_tag_or_a_tree_is_not_written_as_a_parent`, `owners_survive_eviction_and_a_disabled_cache`, `results_of_fresh_commits_outlive_earlier_ones`, `signers_refuse_panic_and_reenter_through_every_routine`, `unreadable_merge_heads_leave_cached_commits_alone`
- `libgit2/filter.rs` (11): `blob_of_another_repository`, `checks_see_the_id_and_the_list_keeps_no_blob`, `constructors_build_lists_freed_once`, `edge_cases_build_nothing_and_leave_the_blob_alone`, `ext_checks_see_the_id_and_the_list_keeps_no_blob`, `first_loads_on_threads_do_not_race`, `handles_reborrow_and_drop_once`, `only_blobs_are_read`, `owners_shorten_their_repository_borrow`, `retained_repository_across_rust_reborrows`, `threads_load_and_drop_their_own_lists`
- `libgit2/index.rs` (2): `additions_through_a_repository_index`, `unborn_nested_repository_is_refused`
- `libgit2/index/edits.rs` (7): `capability_changes_resort_before_removals`, `path_borrows_outlive_writes_and_reads`, `removals_free_entries_once`, `scan_callbacks_read_lent_paths_while_entries_go`, `tree_writes_refuse_foreign_ids_without_overflow`, `tree_writes_refuse_long_names`, `update_all_without_an_owner_runs_no_callback`
- `libgit2/index/entries.rs` (3): `oversized_buffers_are_refused_unread`, `owned_index_additions_fail_and_resolve_without_leaks`, `snapshot_copies_outlive_index_changes_and_the_index`
- `libgit2/notes.rs` (1): `iterator_allocation_failure_releases_the_tree`
- `libgit2/object.rs` (5): `derived_objects_outlive_cache_eviction`, `id_options_apply_the_list_they_borrow`, `ids_of_files_apply_the_list_they_borrow`, `owners_outlive_cache_eviction`, `without_a_cache_each_lookup_is_a_fresh_object`
- `libgit2/odb/backend.rs` (4): `a_failed_rescan_leaves_a_releasable_backend`, `backends_release_once_however_they_were_used`, `handles_taken_between_calls_see_one_backend`, `threads_each_walk_their_own_backend`
- `libgit2/odb/stream.rs` (9): `many_streams_at_once_finish_out_of_order`, `read_streams_read_what_was_written`, `read_streams_refuse_ids_of_another_type`, `streams_of_a_repository_odb`, `threads_open_their_own_streams_on_one_odb`, `write_streams_open_and_release_in_every_state`, `write_streams_store_every_object_type`, `writepacks_fail_and_discard`, `writepacks_take_a_pack_in_pieces_and_commit_it`
- `libgit2/odb/sys.rs` (4): `box_zeroed_reads_zero_then_takes_writes`, `owners_cross_threads`, `owners_work_before_any_init`, `vec_uninit_clones_as_bytes_before_promotion`
- `libgit2/refs.rs` (11): `clone_from_replaces_a_reference_with_a_copy`, `constructors_copy_their_arguments`, `copies_outlive_their_originals`, `copy_chains_release_every_copy_in_any_order`, `first_constructors_race_to_pin_the_runtime`, `handles_reborrow_the_owner`, `ids_of_every_type_byte_are_copied`, `names_of_every_length_stay_in_bounds`, `repositories_on_separate_threads_copy_independently`, `unattached_handles_and_raw_seam`, `unattached_references_cross_threads`
- `libgit2/repository.rs` (2): `owned_results_outlive_their_repository`, `worktree_heads_reach_no_freed_repository`
- `libgit2/repository/sys.rs` (1): `cached_submodules_outlive_the_cache_and_the_repository`
- `libgit2/signature.rs` (2): `failures_release_everything`, `signatures_outlive_their_repository`
- `libgit2/signature/types.rs` (9): `blob_stream_close_consumes_it`, `blob_stream_drops_unwritten_and_written`, `blob_stream_is_written_and_committed`, `c_keeps_no_pointer_past_the_call`, `c_streams_on_many_threads`, `rust_filter_stream_is_freed_by_c`, `rust_target_behind_a_c_filter_stream`, `rust_target_closed_when_a_filter_fails_to_start`, `rust_target_is_written_and_closed_by_c`
- `libgit2/strarray.rs` (1): `libgit2_results_edit_and_release`
- `libgit2/tag.rs` (3): `getters_survive_cache_eviction`, `owners_outlive_cache_eviction`, `without_a_cache_each_lookup_is_a_fresh_tag`
- `libgit2/trace.rs` (2): `libgit2_traces_from_several_threads`, `panicking_tracer_aborts`
- `libgit2/tree.rs` (3): `owners_outlive_cache_eviction`, `walks_lend_entries_for_the_call`, `without_a_cache_each_lookup_is_a_fresh_tree`
- `libgit2/worktree.rs` (11): `a_repository_without_paths_is_refused`, `added_worktree_outlives_its_repository_and_options`, `borrowed_reference_is_read_by_c_and_released_once`, `c_reads_between_reborrows`, `checkout_options_borrows_are_read_by_c`, `failing_adds_release_everything`, `lent_references_are_read_for_the_call_only`, `many_threads_build_and_use_their_own`, `reference_from_another_repository_is_only_read`, `refused_reference_leaves_nothing_behind`, `threads_add_to_their_own_repositories`
- `util/alloc.rs` (4): `copies_are_independent`, `handles_cross_threads`, `libgit2_runs_on_a_rust_allocator`, `threads_share_a_rust_allocator`

## The unwrapped producers the removed fixtures called (24)

Each producer, with the fixture helpers that called it raw; the tests above used those helpers.

- `git_annotated_commit_id`: `read`
- `git_annotated_commit_ref`: `read`
- `git_filter_init`: `register`
- `git_filter_list_apply_to_buffer`: `apply`
- `git_filter_list_length`: `filter_count`
- `git_filter_list_push`: `filter_list`
- `git_filter_list_stream_buffer`: `stream_buffer`
- `git_filter_lookup`: `filter_list`
- `git_filter_register`: `register`
- `git_odb_stream_finalize_write`: `finalize`
- `git_odb_stream_read`: `read`
- `git_odb_write_pack`: `write_pack`
- `git_reference_cmp`: `cmp`
- `git_reference_lookup`: `has_reference`, `reference`
- `git_reference_name`: `view_raw`
- `git_reference_owner`: `owner`
- `git_reference_symbolic_target`: `view_raw`
- `git_reference_target`: `view_raw`
- `git_reference_target_peel`: `view_raw`
- `git_reference_type`: `view_raw`
- `git_remote_connect`: `connect`
- `git_remote_create_detached`: `connect`
- `git_remote_free`: `connect`
- `git_submodule_ignore`: `ignore`
