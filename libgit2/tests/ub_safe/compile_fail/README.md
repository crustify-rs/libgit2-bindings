Tests that must fail to compile: one `.rs` file per illegal safe path, each with its
expected diagnostic in a sibling `.stderr` file (`TRYBUILD=overwrite` records it).
