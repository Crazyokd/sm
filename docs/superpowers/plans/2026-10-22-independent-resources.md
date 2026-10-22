# Independent Resource Management Implementation Plan

> **For agentic workers:** Use the interfaces below for the parallel implementation tasks; do not change another worker's owned files.

**Goal:** Every Skill, Hook, rule, Agent, MCP, prompt, policy and script is classified and managed independently; shared upstreams share fetches only.

**Architecture:** The resource domain contains eight actual kinds. Catalog separates named upstream sources from individual resources. Strict file, complete-Skill-tree and configuration-body projections support one actual resource type at a time. Reviewed adoption moves ownership to the new identities; histories remain historical.

**Tech Stack:** Rust 1.88+, strict TypeScript, existing SQLite and GitLab MR workflow, no added dependencies.

**Spec:** User confirmed “逐项分类和管理”; source, scope and actual resource type remain independent dimensions.

## Global Constraints

- Preserve all current installed bytes, modes, disabled flags, credentials and trust states; no live writes before reviewed merge.
- Use the named source registry and current independent-resource schema only; no parallel compatibility model.
- Self maintained updates remain MR -> review/merge -> next pull/apply. Upstream content is not optimized.
- Keep user/project scope. Kimi project Hooks remain unsupported.
- Retain historical observations honestly; do not invent per-item attribution for old aggregate observations.

## Contract

- `ResourceKind`: Skill, Hook, Rule, Agent, Mcp, Prompt, Policy, Script; labels describe the actual resource type.
- Existing `ResourceContent` Skill/Hook/Rule/Agent/Mcp variants remain for current native payload adapters.
- Use `Files { kind: ResourceKind, files: FileProjection }` for selected projections and `Installation { installation: InstallationPolicy }` for native lifecycle policies, always kind Policy.
- `FileProjection { root: PathBuf, entries: Vec<ProjectionEntry> }` uses strict File `{target,source}`, Skill-only Tree `{target,tree}` and Fragment `{target,fragment}` entries. Source paths resolve relative to the resource root. Tree selects one complete Skill directory and its mirrors so additions/removals follow the source. Self mirror bodies share one editable source; canonical-only provenance may be a non-overlapping File overlay. Fragment reads one resource-owned JSON object body and merges it through the native JSON/TOML format. Configuration values contain one actual resource type and retain unknown fields and array order. Projection validates selected files/trees without walking a shared upstream root for every resource. Existing strict inline JSON/TOML projection adapters retain the same type and native-target checks; shipped self configurations use editable Fragment bodies.
- Serialized resource: `{id,kind,origin,scope,project,files:{root,entries}}`, or existing native fields; installation policy uses `{id,kind:"policy",origin:"self",scope:"user",project:null,installation:{agent,source,selector,enabled,paths,update}}`.
- `Source::Upstream { upstream: String }` references `Catalog.upstreams: BTreeMap<String,UpstreamSource>`; no inline duplicate sources.
- `Resource::paths()` for Files yields selected root/source files, root/fragment configuration files, and root/tree Skill directories only. Installation has no payload paths; policies live in catalog.json. Skill/native paths keep current meaning.
- `Resource::targets()` Files maps both supported clients to files.root; provider detects Files before native client checks and uses exact entries once (plan deduplicates identical writes). Installation returns no targets.
- `InstallationPolicy` is a public strict typed struct in model with agent,source,selector,enabled,paths,update, no duplicate id. Native runtime `Installation {id, #[serde(flatten)] policy: InstallationPolicy}` keeps the native helper wire format unchanged.
- `installations::load(source)` deserializes Catalog (not Catalog::load recursively) and reads installation policies from individual catalog resources, not a separate installations.json. Validate native policies as before. Kind must be Policy, origin self, scope user.
- `repository::fetch_upstream(upstream: &UpstreamSource, root: &Path, output: &Path) -> Result<PathBuf>`; service fetches each referenced source once and maps each resource id to its fetch result. Missing individual payloads fail only those resources; shared target failure protection remains.
- Files projection is internal representation, not a user resource kind. UI shows per-resource body/configuration and the actual kind; never asks users to manage a mixed collection.
- Self FileProjection raw files move into `resources/<resource-id>/files/...` so proposal path authorization stays resource specific. Self configuration bodies live in resource-owned JSON files referenced by Fragment entries so the existing AI file-edit proposal/evaluation path can update them independently.
- Source generators output files at their intended relative target path (including legacy mirrors). Only target-relative content is emitted. Each source-backed resource projects selected files or complete Skill trees from that shared output root.
- Reviewed adoption entries may specify `previous_resource` for explicit safe file ownership handoff. Only the named old owner, matching reviewed file hash/mode and current stored ownership may transfer. For JSON/TOML, subtract only the exact claimed fragment from the previous ownership record before adding the new one; validate the entire old fragment against the current document. Changed/unrelated files remain conflicts. Handoff is transactional and does not write installed files.

## Review Focus

- A shared source is fetched once, but per-resource status, counts, content and MR selections stay separate.
- A selected resource cannot modify another self resource's files, source definition or native installation policy.
- Splitting config fragments preserves all unknown fields and array order; identical expected projections are idempotent.
- A handoff cannot overwrite a changed file or steal ownership from an unrelated resource.
- Shared native config paths cannot cause usage attribution to arbitrarily pick the first resource.

## Tasks

### 1. Typed model and source/application pipeline

Files: src/model.rs, src/projections.rs, src/providers.rs, src/repository.rs, src/service.rs, src/installations.rs, src/inventory.rs (projection import only), src/lib.rs; associated focused tests.

- [x] Reject aggregate or mixed-type declarations and verify shared-source fetch once.
- [x] Implement contract, preserving file/config validation, local bindings and source/native subprocess boundaries.
- [x] Preserve local-first startup and source failure isolation. Installation policies are pending until native update verification.
- [x] Run focused model/provider/service/native tests; coordinate fixture schema conversion with root.

### 2. Individual resource declarations and source output

Files: catalog.json, resources/**, src/ecc.rs, src/skill_source.rs, tests/ecc.rs, tests/skill_source.rs.

- [x] Split all aggregate declarations into stable typed identities and a deduplicated upstream registry.
- [x] Group each complete Skill tree and its mirrors as one resource; MCP servers and native Hook groups are separate identities. Native runtime source modules are independently managed Script resources. Keep one native skill configuration table as a Policy where that native table is the atomic merge unit.
- [x] Move self payload bytes/modes unchanged to resource-specific directories; remove obsolete aggregate manifests and directories.
- [x] Update generators to write stable target-relative files; verify target bytes/modes match the previous generation.
- [x] Produce new reviewed adoption fingerprints plus explicit previous owners from current state without writing that state.

### 3. Per-resource UI

Files: frontend/**, assets/templates/**, assets/static/app.js, src/web.rs, tests/web.rs, tests/browser_editor.ts.

- [x] Remove aggregate type/options/templates. Show all eight actual resource kinds and individual resources.
- [x] Support typed Files body/config editors; keep source definition editing separate from actual resource kind.
- [x] Update source registry editing without changing other resources' shared sources implicitly.
- [x] Native installation toggles change only that policy resource in catalog.json and submit a normal MR.
- [x] Verify filtering, proposals, resource details, disabled state and 390px layout.

### 4. Ownership, proposal boundaries and observation attribution

Files: src/adoption.rs, src/proposals.rs, src/observe.rs, remaining affected test fixtures, docs.

- [x] Add reviewed atomic ownership handoff with changed/unrelated file regression coverage.
- [x] Enforce selected-resource limits for changed shared source definitions and resource files.
- [x] Capture per-item usage; ambiguous shared paths remain unattributed rather than choosing a resource arbitrarily.
- [x] Verify old/new rendered target content equivalence on a copied state and actual filesystem, and zero unmanaged/drift after handoff.
- [x] Complete the final delivery gates and independent Rust/TS/general review.
- [ ] Deliver the MR, successful CI, reviewed merge and actual local deployment.


## Current implementation evidence

The branch declares 1,041 resources, 19 shared upstream sources, 10 projects and 10 native installation policies. It contains 229 Skill projection resources with 365 Tree entries and 103 existing native Skill directory adapters. All 32 self-maintained tree resources have one editable body each, covering 44 client targets; two canonical-only provenance files remain explicit overlays. All 59 self configuration fragments have resource-owned JSON bodies.

The copied-state/live-filesystem report at `/tmp/sm-independent-preflight-report.json` records 1,846 inventory entries, zero unmanaged/drift/errors/local failures/residual old owners, and 4,114 resulting ownership records. Historical cleanup of three stale links is complete and is not pending work for this branch. Exact target verification preserves 1,469 reviewed file payloads and all eight native configuration documents, including array order.

The added `kimi-open-code-review` policy governs the canonical installation already created by an older updater. Four Kimi native registry entries retain their enabled flags. The legacy `open-code-review-codex` identity mismatch remains an explicit failed refresh with its installation preserved; immutable manifest identity is checked before installation so it cannot create an undeclared identity silently.

The final complete Rust regression run passed 207 tests with zero failures and one credential-dependent test ignored. All-target Clippy, formatting, release build and strict TypeScript checks passed. Browser regression passed on the read-only preview with every submission intercepted; desktop and 390px layouts were visually checked. No external submission or native update was performed by that browser run.

Implementation, local verification and independent review are complete. MR/CI, reviewed merge, local deployment and upstream refresh acceptance remain delivery steps whose results are recorded externally in the delivery MR.
