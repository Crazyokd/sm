# ECC file provenance

The explicit source selection preserves 488 existing legacy resource paths without vendoring upstream bodies. Each file was verified against its ECC 2.2.1 native installation record (`~/.claude/ecc/install-state.json`) and the original published package bytes. This includes 202 runtime files under `.claude/scripts`; executable and library modules are independently managed Script resources. Hook configuration groups, Agents, rules, prompts and complete Skills retain their actual resource types.

ECC is one shared upstream fetch in the catalog's named source registry. Its members have independent resource identities, scopes, status and proposal selections. The generator writes directly to intended target-relative paths, including legacy client mirrors. Complete Skill trees follow additions and removals in the generated source; a missing selected member leaves only that resource unavailable. Unsafe or malformed sources remain hard failures.

Ten old rule aliases remain symlinks and derive ownership from their canonical rule targets without duplicate file claims. Locally adapted or unverified files remain separately declared self-maintained resources with their provenance retained. Each self-maintained native configuration fragment is stored as a JSON object in its own resource directory and merged into the native JSON/TOML target; private values stay local.

Native generation against published ECC 2.2.3 produced 1,066 unique targets. All 488 extra files matched the published archive bytes, SHA-256 values and executable flags. The new generator reproduced all 1,066 target bytes and executable flags exactly in a temporary directory. No selected published source was missing in that validation, and no resource-defined business workflow was executed.

The independent-resource handoff was checked with a copied SQLite state and the actual filesystem. It preserves installed file bytes and executable flags, native configuration values and array order. This is development-side conversion evidence. Local deployment and upstream refresh acceptance are recorded separately in the delivery MR.
