# Complete Resource Units Implementation Plan

**Goal:** Manage complete functional resources, removing Script as a public type and keeping implementation files with owners or referenced shared dependencies.

**Architecture:** Seven public ResourceKind values remain. Existing FileProjection already supports multiple files and Hook configuration plus scripts. Catalog adds an internal `dependencies` registry; resources reference it using `dependencies: string[]`. Dependencies hold source plus selected file projection, inherit each consumer's user/project scope, and are rendered as part of that consumer's plan. Identical shared file writes may have multiple real resource owners; removal of one consumer retains files for remaining consumers. Dependencies are not resources and have no independent counts, optimization or MR identity.

**Tech Stack:** Existing Rust1.88+, strict TypeScript, SQLite and GitLab; no new dependencies.

**Spec:** User confirmed a resource is a complete functional unit, can contain many files and can share a native configuration file with other resources; Script support files must not be scattered as standalone resources. User's request to change classification approves this bounded correction.

## Contract

- Remove `ResourceKind::Script` and public `script` type/options; reject old Script declarations, no parallel compatibility model.
- `Resource.dependencies: Vec<String>` defaults empty and omits empty in serialization. `RawResource` same field; every existing struct literal must receive an empty field.
- `Catalog.dependencies: BTreeMap<String, SupportingFiles>` defaults empty, omits empty. `SupportingFiles { #[serde(flatten)] source: Source, files: FileProjection }`, strict deserialization, no kind/id/scope. References must exist and be unique; reject unreferenced definitions to avoid unmanaged orphan declarations.
- Dependencies may contain only File entries at approved script/runtime target locations, never native configuration, Skill, rule, Agent or prompt targets. Core exposes `projections::support_target(path)` and `projections::render_support(id, files, payload, scope)` using existing containment/symlink/mode checks. Resolve `files.root` inside source before rendering, like main projection. Self dependency roots must be under `resources/dependencies/<name>/files`; reserve resource id `dependencies` as infrastructure.
- Fetch dependency sources through existing once-per-upstream registry cache. Source map keys for dependencies use `@dependency/<name>`; only referenced definitions fetched. Self dependency maps to approved checkout. Local-first planning includes available self dependencies and preserves pending consumers of upstream dependencies. Missing/failing support blocks complete consumer writes and retains previous installation.
- `apply::prepare` permits shared raw files only when all next bodies+executable bits agree, all previous stored fingerprints agree and actual content/mode match. Preserve existing shared-target preflight/runtime blocking, unknown-file conflicts and retirement protection.
- Reviewed adoption permits explicit handoff from old Script identity to first true consumer, then reviewed additional claims of that exact same file fingerprint/mode by further consumers. No arbitrary changed/different-owner takeover. Root owns this part and extra tests.
- Existing source/scope separation remains. Different dependency sources do not reclassify the resource's main authored configuration as upstream. Upstream dependency bodies are not local optimization targets.
- Proposals changing shared dependency definitions require selecting every referring resource before/after. Infrastructure payload paths remain outside individual proposal file editing. Resource rename retains dependency refs; retirement prunes definitions with no consumers. AI optimization handles only approved self-owned editable bodies.
- UI lists complete resources, displays referenced supporting file paths/source in resource detail, and never lists dependency as a resource type. Existing resource editing preserves refs; remove/rename/new-source operations do not discard other's dependencies. Dependency management details only where they explain complete resource contents.

## Constraints

- Preserve live installed bytes, modes, source, scope, credentials, enabled/disabled and native trust states.
- No actual installed/state writes before reviewed merge. Use actual filesystem and SQLite copies for preflight.
- Keep existing raw event history honest; do not backfill invented usage attribution.
- Do not fold unrelated independent hooks into one umbrella just to hide files. Group self Hindsight events that belong to the same recall/retain integration per client.
- No new plugin/package resource type, source channels, or speculative resource categories.

## Tasks and ownership

- [x] Core worker: src/model.rs, src/projections.rs, src/apply.rs, src/service.rs, src/providers.rs and tests/projections.rs,tests/model_apply.rs,tests/service.rs,tests/catalog.rs. Implement contract + meaningful share/conflict/last-owner/failure/isolation regressions. Only root runs full Cargo gates; coordinate scoped Cargo use.
- [x] Data worker: catalog.json, resources/** except source-provider native TS/JS, docs/resource-governance.md,docs/verification.md. Merge true self Hook units; convert ECC support files to referenced internal dependencies. Update adoption manifests and preserve payload bytes/modes. Report exact previous-target coverage, consumers, source/scope and no orphan dependencies. Historical adoption entries for prior transitions must be retargeted/compacted to current owners using real state without writing it.
- [x] UI worker: frontend/**, assets/**, src/web.rs, tests/web.rs,tests/browser_editor.ts. Seven types, preserve dependency refs and registry, complete Hook file editing, internal dependency source/file display; no standalone dependency/Script resources.
- [x] Root: src/adoption.rs,src/proposals.rs,src/analysis.rs,src/observe.rs,src/inventory.rs, remaining test fixtures. Reviewed shared ownership, selected-resource guards, source-aware evidence and attribution. Regression test old boundary before implementation.
- [ ] Root: copied-state/live-filesystem preflight and safe browser preview, final Rust/TS/general independent reviews, exact-head CI, MR/merge, actual local init/application and settings check. No status claims before evidence.

## Additional verified boundaries

- Source/plan failures defer obsolete owner retirement; structural native installation Pending is excluded. Apply preflight/runtime failures also retain removal-only owners until replacement succeeds, with red/green rename-conflict proof.
- Shared raw support targets are inventoried once with complete owner links/search and drift checks. Distinct native configuration/Markdown fragments remain individually visible.
- A newly discovered existing user Skill easygo-app-checks was registered without moving or executing it:831resources/7kinds.4046oldrawtargets and8configs remainidentical;3existingSkillfiles additionally adopted.
