export type Client = "codex" | "kimi";
export type NativeKind = "hook" | "rule" | "agent" | "mcp";
export type Kind = "skill" | NativeKind | "prompt" | "policy" | "script";
export type Upstream = { git: string } | { argv: string[] };
export type ScopeFields = { scope: "user"; project: null } | { scope: "project"; project: string };
export type SourceFields = { origin: "self" } | { origin: "upstream"; upstream: string };
export type ProjectionEntry = { target: string; source: string } | { target: string; tree: string } | { target: string; fragment: string } | { target: string; json: Record<string, unknown> } | { target: string; toml: Record<string, unknown> };
export interface FileProjection { root: string; entries: ProjectionEntry[] }
export interface InstallationPolicy { agent: Client; source: string; selector: string; enabled: boolean; paths: string[]; update: string[][] }
export type ContentFields = { kind: "skill"; payload: string } | { kind: NativeKind; agents: Client[]; payloads: Partial<Record<Client, string>> } | { kind: Kind; files: FileProjection } | { kind: "policy"; installation: InstallationPolicy };
export type Resource = { id: string } & ScopeFields & SourceFields & ContentFields;
export interface Catalog { projects: Record<string, string>; upstreams: Record<string, Upstream>; resources: Resource[] }
export interface EditorData { catalog: Catalog; files: Record<string, string> }
export interface ChangeSet {
  resource_ids: string[]; reason: string; evidence: string[];
  writes: Record<string, string>; deletions: string[]; renames: Record<string, string>;
  validation: { status: "partial"; checks: never[]; unverified: string[] };
}
