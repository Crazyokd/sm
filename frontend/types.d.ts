export type Client = "codex" | "kimi";
export type NativeKind = "hook" | "rule" | "agent" | "mcp";
export type Kind = "skill" | NativeKind;
export type Upstream = { git: string } | { argv: string[] };
export type ScopeFields = { scope: "user"; project: null } | { scope: "project"; project: string };
export type SourceFields = { origin: "self" } | { origin: "upstream"; upstream: Upstream };
export type ContentFields = { kind: "skill"; payload: string } | { kind: NativeKind; agents: Client[]; payloads: Partial<Record<Client, string>> };
export type Resource = { id: string } & ScopeFields & SourceFields & ContentFields;
export interface Catalog { projects: Record<string, string>; resources: Resource[] }
export interface EditorData { catalog: Catalog; files: Record<string, string> }
export interface ChangeSet {
  resource_ids: string[]; reason: string; evidence: string[];
  writes: Record<string, string>; deletions: string[]; renames: Record<string, string>;
  validation: { status: "partial"; checks: never[]; unverified: string[] };
}
