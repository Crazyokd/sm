# ECC file provenance

The explicit `files` selection preserves 488 existing legacy resource paths without vendoring upstream bodies. Every file matches both its ECC 2.2.1 native installation record (`~/.claude/ecc/install-state.json`) and the original published package bytes. This includes 202 installed runtime files under `.claude/scripts`, providing the copied hook and library dependencies. Current hook commands retain their original text and native plugin dependencies stay with their plugin package.

Ten old rule aliases are preserved as symlinks; they derive ownership from the selected canonical rule targets and have no duplicate file claim. Current runtime settings and four locally changed or unverified user files remain separate local adaptations. Native configuration is stored only as selected fragments with private values left local. Initial adoption retains the current executable flag in ownership metadata, and the generator preserves published source flags.

Final native generation against published ECC 2.2.3 produced 1,066 unique bundle targets. All 488 extra files matched the published archive bytes, SHA-256 values, and executable flags; no selected source path was missing. Validation used a temporary output directory.
