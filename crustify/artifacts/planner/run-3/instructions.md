# Planner run 3: the module-ordered schedule, every wave re-planned

The user decided the following departures from your prompt. Keep them; do not reconcile them
with the rest of it.

- **Every wave is re-planned, the landed ones included.** `schedule.json` holds only your new
  plan: every wave it holds now, done, `wip` (180 and 228) and `todo`, translation and review
  waves alike, is removed, and the landed items are scheduled again like any other item, in
  the place the order gives them. Number the new waves after the highest index the schedule
  holds now. Leave every new wave and batch `todo` with empty `commits`: the assistant marks
  the landed part done and attributes its commits after your run.
- **The decomposition stays as it is.** Add, remove, split or merge no module, and change no
  file entry, item, test, objective, status or home. Only bring it to the current schema:
  `modules.json`'s `depends_on`, the single wavefront configuration, and the out-of-tree
  records. A decomposition decision you doubt goes in your commit message, not in an edit
  or a report.

The T1/T2 tables were re-extracted on 2026-11-06, at wavefront 6150082, through
`crustify/tmp/extract-config.json`, a closure-only conversion of the old configuration made
for the extraction alone.
