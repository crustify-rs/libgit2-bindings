/* Definitions of the shims declared in shims.h. */
#include "shims.h"

git_oid_t crustify_GIT_OID_DEFAULT(void)
{
	return GIT_OID_DEFAULT;
}

int crustify_GIT_INDEX_ENTRY_STAGE(const git_index_entry *entry)
{
	return GIT_INDEX_ENTRY_STAGE(entry);
}

void crustify_GIT_INDEX_ENTRY_STAGE_SET(git_index_entry *entry, int stage)
{
	GIT_INDEX_ENTRY_STAGE_SET(entry, stage);
}
