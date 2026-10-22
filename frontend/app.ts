"use strict";

import type { Client, Kind, ScopeFields, SourceFields, ContentFields, Upstream, EditorData, ChangeSet } from "./types.js";

type Field = "resource_id" | "kind" | "origin" | "scope" | "project" | "project_path" | "upstream_git" | "upstream_argv" | "upstream_path" | "reason";
function required<E extends Element>(root: ParentNode, selector: string): E {
  const element = root.querySelector<E>(selector);
  if (!element) throw new Error(`缺少页面控件：${selector}`);
  return element;
}
function responseFields(body: unknown): { message?: string; error?: string; url?: string } {
  if (typeof body !== "object" || body === null) throw new Error("服务返回了无效结果。");
  const fields: { message?: string; error?: string; url?: string } = {};
  for (const name of ["message", "error", "url"] as const) {
    if (name in body) {
      const value: unknown = Reflect.get(body, name);
      if (typeof value !== "string") throw new Error("服务返回了无效结果。");
      fields[name] = value;
    }
  }
  return fields;
}

for (const form of document.querySelectorAll<HTMLFormElement>(".installation-editor")) {
  form.addEventListener("submit", event => {
    event.preventDefault();
    const result = required<HTMLElement>(form, ".form-result");
    const button = required<HTMLButtonElement>(form, "button");
    const submit = async (): Promise<void> => {
      button.disabled = true; result.textContent = "正在提交修改提案…";
      try {
        const token = required<HTMLMetaElement>(document, 'meta[name="sm-csrf"]').content;
        const reason = required<HTMLInputElement>(form, "input[name=reason]").value.trim();
        const response = await fetch(`/installations/${encodeURIComponent(form.dataset.id ?? "")}`, {
          method: "POST", headers: { "Content-Type": "application/json", "X-SM-CSRF": token },
          body: JSON.stringify({ enabled: form.dataset.enabled === "true", reason }),
        });
        const body = responseFields(await response.json());
        if (!response.ok || !body.message) throw new Error(body.error ?? "提交失败，请查看运行状态。");
        result.textContent = body.message;
        if (body.url && /^https?:\/\//.test(body.url)) {
          const link = document.createElement("a"); link.href = body.url; link.textContent = "打开 MR ↗";
          link.target = "_blank"; link.rel = "noopener noreferrer"; result.append(link);
        }
      } catch (error: unknown) { result.textContent = error instanceof Error ? error.message : "提交失败"; }
      finally { button.disabled = false; }
    };
    void submit();
  });
}

for (const form of document.querySelectorAll<HTMLFormElement>(".resource-editor")) {
  const encoded = required<HTMLScriptElement>(form, ".editor-data").textContent;
  if (!encoded) throw new Error("资源目录尚未加载。");
  // Rust renders this data from its validated catalog and content cache.
  const data: EditorData = JSON.parse(encoded);
  const original = data.catalog.resources.find(resource => resource.id === form.dataset.existing);
  const originalFiles = data.files;
  const drafts: Record<string, string> = { ...originalFiles };
  const defaults: Record<string, string> = {};
  const result = required<HTMLElement>(form, ".form-result");
  const fields = required<HTMLElement>(form, ".payload-fields");
  const buttons = form.querySelectorAll<HTMLButtonElement>(".form-actions button");
  let currentFile: string | null = null;
  let textarea: HTMLTextAreaElement | null = null;
  let installationEnabled = original && "installation" in original ? original.installation.enabled : false;
  const control = (name: Field): HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement => {
    const input = form.elements.namedItem(name);
    if (input instanceof HTMLInputElement || input instanceof HTMLSelectElement || input instanceof HTMLTextAreaElement) return input;
    throw new Error(`缺少表单字段：${name}`);
  };
  const value = (name: Field): string => control(name).value.trim();
  const currentKind = (): Kind => {
    const kind = value("kind");
    switch (kind) {
      case "skill": case "hook": case "rule": case "agent": case "mcp": case "prompt": case "policy": case "script": return kind;
      default: throw new Error("无效的资源类型。");
    }
  };
  const clients = (): Client[] => currentKind() === "prompt" || currentKind() === "policy" ? ["codex"] : (currentKind() === "skill" || original && ("files" in original || "installation" in original)) ? ["codex", "kimi"] : [...form.querySelectorAll<HTMLInputElement>("input[name=agents]:checked")].map(input => {
    if (input.value === "codex" || input.value === "kimi") return input.value;
    throw new Error("无效的客户端。");
  });
  const save = (): void => {
    if (textarea && currentFile) drafts[currentFile] = textarea.value;
    const enabled = fields.querySelector<HTMLInputElement>("[name=installation_enabled]");
    if (enabled) installationEnabled = enabled.checked;
  };
  const templates = (kind: Kind, client: Client, id: string): [string, string] => {
    switch (kind) {
      case "skill": return ["SKILL.md", `---\nname: ${id}\ndescription: 说明何时应使用这项技能\n---\n\n# ${id}\n\n写下工作流程和完成标准。\n`];
      case "script": throw new Error("请从已有脚本发起修改。");
      case "prompt": return ["prompt.md", "# 提示词\n\n写下提示词内容。\n"];
      case "policy": throw new Error("执行策略使用配置片段编辑器。");
      case "rule": return ["AGENTS.md", "# 工作约定\n\n在这里写下需要持续遵循的约定。\n"];
      case "agent": return client === "codex" ? ["agent.toml", `name = "${id}"\ndescription = "说明角色用途"\ndeveloper_instructions = "描述角色的工作方法"\n`] : ["agent.md", `---\nname: ${id}\ndescription: 说明角色用途\n---\n\n描述角色的工作方法。\n`];
      case "mcp": return client === "codex" ? ["config.toml", `[mcp_servers.${id}]\ncommand = "your-mcp-command"\n`] : ["mcp.json", JSON.stringify({ mcpServers: { [id]: { command: "your-mcp-command" } } }, null, 2)];
      case "hook": return client === "codex" ? ["hooks.json", JSON.stringify({ hooks: { Stop: [{ hooks: [{ type: "command", command: "true" }] }] } }, null, 2)] : ["config.toml", '[[hooks]]\nevent = "Stop"\ncommand = "true"\n'];
    }
  };
  const renamedPath = (path: string, id: string): string => original && id !== original.id ? path.replace(`resources/${original.id}/`, `resources/${id}/`) : path;
  const sourcePath = (path: string, id: string): string => original && id !== original.id ? path.replace(`resources/${id}/`, `resources/${original.id}/`) : path;
  const projectionPath = (root: string, source: string): string => `${root}/${source}`.split("/").filter(part => part && part !== ".").join("/");
  const payloadPath = (client: Client, id: string, kind: Kind): string => {
    const path = original && "payload" in original ? original.payload : original && "payloads" in original ? original.payloads[client] : undefined;
    return renamedPath(path || `resources/${id}/${kind === "prompt" || kind === "policy" ? "files" : kind === "skill" || kind === "rule" ? "shared" : client}`, id);
  };
  const render = (): void => {
    save();
    const upstream = value("origin") === "upstream";
    required<HTMLElement>(form, ".upstream-fields").hidden = !upstream;
    required<HTMLElement>(form, ".agent-options").hidden = currentKind() === "skill" || currentKind() === "prompt" || currentKind() === "policy" || !!original && "files" in original;
    for (const name of ["project", "project_path"] as const) {
      const label = control(name).closest("label");
      if (label) label.hidden = value("scope") !== "project";
    }
    fields.replaceChildren(); currentFile = null; textarea = null;
    if (original && "installation" in original) {
      const label = document.createElement("label"); label.textContent = "启用原生安装";
      const enabled = document.createElement("input"); enabled.type = "checkbox"; enabled.name = "installation_enabled";
      enabled.checked = installationEnabled; label.append(enabled); fields.append(label);
      return;
    }
    if (upstream && !(original && "files" in original)) return;
    const id = value("resource_id") || "new-resource";
    const kind = currentKind();
    const names = new Map<string, string>();
    if (!original && kind === "policy") {
      const key = `resources/${id}/files/config.json`;
      names.set(key, ".codex/config.toml · Skill 启停策略（JSON）");
      const content = JSON.stringify({ skills: { config: [{ name: id, enabled: true }] } }, null, 2) + "\n";
      if (!(key in drafts) || drafts[key] === defaults[key]) drafts[key] = content;
      defaults[key] = content;
    } else if (original && "files" in original) {
      original.files.entries.forEach((entry, index) => {
        if ("tree" in entry) {
          const root = projectionPath(original.files.root, entry.tree);
          const prefix = root ? root + "/" : "";
          for (const [path, content] of Object.entries(originalFiles)) if (path.startsWith(prefix)) {
            const key = renamedPath(path, id);
            names.set(key, projectionPath(entry.target, path.slice(prefix.length)));
            if (!(key in drafts)) drafts[key] = content;
          }
          return;
        }
        const source = "source" in entry ? entry.source : "fragment" in entry ? entry.fragment : null;
        const key = source === null ? `fragment:${index}` : renamedPath(projectionPath(original.files.root, source), id);
        if (source !== null && !(sourcePath(key, id) in originalFiles)) return;
        names.set(key, entry.target + ("source" in entry ? "" : " · 配置片段（JSON）"));
        if (!(key in drafts)) drafts[key] = source !== null ? originalFiles[sourcePath(key, id)] ?? "" : JSON.stringify("json" in entry ? entry.json : "toml" in entry ? entry.toml : {}, null, 2) + "\n";
      });
    } else for (const client of clients()) {
      const root = payloadPath(client, id, kind);
      const existing = Object.keys(originalFiles).map(path => renamedPath(path, id)).filter(path => path.startsWith(root + "/"));
      if (existing.length) {
        for (const path of existing) {
          names.set(path, path.split("/").slice(2).join("/"));
          if (!(path in drafts)) drafts[path] = originalFiles[sourcePath(path, id)] ?? "";
        }
        continue;
      }
      const [filename, content] = templates(kind, client, id);
      const path = `${root}/${filename}`;
      names.set(path, path.split("/").slice(2).join("/")); defaults[path] = content;
      if (!(path in drafts)) drafts[path] = content;
    }
    if (!names.size) {
      const notice = document.createElement("p"); notice.textContent = "尚无可编辑的文本内容，请先查看同步结果。";
      fields.append(notice); return;
    }
    const label = document.createElement("label"); label.textContent = "修改内容";
    const select = document.createElement("select");
    for (const [path, target] of names) {
      const option = document.createElement("option"); option.value = path;
      option.textContent = target; select.append(option);
    }
    label.append(select);
    const textLabel = document.createElement("label");
    textLabel.textContent = `${{ skill: "Skill", hook: "Hook", rule: "规则", agent: "Agent", mcp: "MCP", prompt: "提示词", policy: "执行策略", script: "脚本" }[kind]}内容`;
    textarea = document.createElement("textarea"); textarea.rows = 10; textarea.readOnly = upstream;
    currentFile = select.value; textarea.value = drafts[currentFile] || "";
    textLabel.append(textarea); fields.append(label, textLabel);
    select.addEventListener("change", () => { save(); currentFile = select.value; if (textarea) textarea.value = drafts[currentFile] || ""; });
  };
  for (const name of ["kind", "origin", "scope", "agents"]) {
    for (const input of form.querySelectorAll(`[name=${name}]`)) input.addEventListener("change", render);
  }
  control("resource_id").addEventListener("change", () => {
    save();
    const prefix = currentFile?.match(/^resources\/[^/]+\//)?.[0];
    const next = `resources/${value("resource_id") || "new-resource"}/`;
    if (prefix && prefix !== next) for (const [path, content] of Object.entries(drafts)) {
      if (path.startsWith(prefix) && content !== defaults[path]) drafts[next + path.slice(prefix.length)] = content;
    }
    render();
  });

  async function submit(retire = false): Promise<void> {
    save(); result.textContent = ""; result.classList.remove("error");
    try {
      const id = retire && original ? original.id : value("resource_id");
      if (!/^[a-z0-9][a-z0-9_-]*$/.test(id)) throw new Error("请填写有效的资源名称。");
      if ((!original || id !== original.id) && data.catalog.resources.some(resource => resource.id === id)) throw new Error("资源名称已存在，请从已有资源的详情修改。");
      if (!value("reason")) throw new Error("请说明修改原因。");
      if (retire && !original) throw new Error("只能从已有资源申请移除纳管。");
      const catalog = structuredClone(data.catalog);
      const writes: Record<string, string> = {};
      if (!retire) {
        const kind = currentKind(), selected = clients();
        if (!selected.length) throw new Error("请选择至少一个客户端。");
        const scope = value("scope"), project = value("project");
        if (scope !== "user" && scope !== "project") throw new Error("无效的作用域。");
        if (scope === "project" && !project) throw new Error("项目级资源需要项目归属。");
        if (kind === "hook" && scope === "project" && selected.includes("kimi") && !(original && "files" in original)) throw new Error("Kimi 当前不支持原生项目级 Hook，请调整适用客户端或作用域。");
        if (scope === "project" && !(project in catalog.projects)) {
          if (!value("project_path")) throw new Error("新项目需要填写约定目录，例如 ~/repo/app。");
          catalog.projects[project] = value("project_path");
        }
        const attribution: ScopeFields = scope === "project" ? { scope, project } : { scope, project: null };
        const origin = value("origin");
        if (origin !== "self" && origin !== "upstream") throw new Error("无效的资源来源。");
        let source: SourceFields;
        if (origin === "upstream") {
          const previous = original?.origin === "upstream" ? catalog.upstreams[original.upstream] : undefined;
          if (value("upstream_git") && value("upstream_argv")) throw new Error("上游 Git 地址与生成命令只填写一项。");
          let upstream: Upstream | undefined = value("upstream_git") ? { git: value("upstream_git") } : previous;
          if (value("upstream_argv")) {
            const argv: unknown = JSON.parse(value("upstream_argv"));
            if (!Array.isArray(argv) || !argv.length || !argv.every((arg: unknown) => typeof arg === "string") || !argv[0]) throw new Error("生成命令必须是非空字符串 argv 数组。");
            upstream = { argv: argv as string[] };
          }
          if (!upstream) throw new Error("请填写上游 Git 地址或生成命令。");
          let reference = original?.origin === "upstream" ? original.upstream : id;
          if (JSON.stringify(upstream) !== JSON.stringify(catalog.upstreams[reference])) {
            const shared = catalog.resources.some(resource => resource.id !== original?.id && resource.origin === "upstream" && resource.upstream === reference);
            if (shared || !original) {
              reference = `${id}-source`;
              let suffix = 2;
              while (reference in catalog.upstreams) reference = `${id}-source-${suffix++}`;
            }
            catalog.upstreams[reference] = upstream;
          }
          source = { origin, upstream: reference };
        } else source = { origin };
        let definition: ContentFields;
        if (original && "installation" in original) {
          const enabled = required<HTMLInputElement>(fields, "[name=installation_enabled]").checked;
          definition = { kind: "policy", installation: { ...original.installation, enabled } };
        } else if (original && "files" in original) {
          const entries = original.files.entries.map((entry, index) => {
            if ("source" in entry || "tree" in entry) return entry;
            const key = "fragment" in entry ? renamedPath(projectionPath(original.files.root, entry.fragment), id) : `fragment:${index}`;
            const fragment: unknown = JSON.parse(drafts[key] ?? "{}");
            if (typeof fragment !== "object" || fragment === null || Array.isArray(fragment)) throw new Error("配置片段必须是 JSON 对象。");
            if ("fragment" in entry) return entry;
            return "json" in entry ? { target: entry.target, json: fragment as Record<string, unknown> } : { target: entry.target, toml: fragment as Record<string, unknown> };
          });
          definition = { kind, files: { root: origin === "upstream" ? value("upstream_path") || "." : renamedPath(original.files.root, id), entries } };
        } else if (kind === "skill") definition = { kind, payload: origin === "upstream" ? value("upstream_path") || "." : payloadPath("codex", id, kind) };
        else if (kind === "policy") {
          const fragment: unknown = JSON.parse(drafts[`resources/${id}/files/config.json`] ?? "{}");
          if (typeof fragment !== "object" || fragment === null || Array.isArray(fragment)) throw new Error("配置片段必须是 JSON 对象。");
          definition = { kind, files: { root: origin === "self" ? `resources/${id}/files` : value("upstream_path") || ".", entries: [{ target: ".codex/config.toml", fragment: "config.json" }] } };
        } else if (kind === "prompt") {
          const root = payloadPath("codex", id, kind), filename = templates(kind, "codex", id)[0];
          definition = { kind, files: { root: origin === "self" ? root : value("upstream_path") || ".", entries: [{ target: `.codex/prompts/${id}.md`, source: filename }] } };
        } else if (kind === "script") throw new Error("请从已有脚本发起修改。");
        else {
          const payloads: Partial<Record<Client, string>> = {};
          const oldPath = original && "payload" in original ? original.payload : original && "payloads" in original ? Object.values(original.payloads)[0] : undefined;
          const changedPath = value("upstream_path") !== oldPath;
          for (const client of selected) payloads[client] = origin === "self" ? payloadPath(client, id, kind) : original && "payloads" in original && original.payloads[client] && !changedPath ? original.payloads[client] : value("upstream_path") || ".";
          definition = { kind, agents: selected, payloads };
        }
        if (origin === "self") for (const option of fields.querySelectorAll<HTMLOptionElement>("option")) {
          if (option.value.startsWith("fragment:")) continue;
          const content = drafts[option.value];
          if (content !== undefined && content !== originalFiles[sourcePath(option.value, id)]) writes[option.value] = content;
        }
        catalog.resources = catalog.resources.filter(resource => resource.id !== (original?.id || id));
        catalog.resources.push({ id, ...attribution, ...source, ...definition });
      } else catalog.resources = catalog.resources.filter(resource => resource.id !== id);
      const usedSources = new Set(catalog.resources.flatMap(resource => resource.origin === "upstream" ? [resource.upstream] : []));
      for (const reference of Object.keys(catalog.upstreams)) if (!usedSources.has(reference)) delete catalog.upstreams[reference];
      writes["catalog.json"] = JSON.stringify(catalog, null, 2) + "\n";
      const renames: Record<string, string> = !retire && original && id !== original.id ? { [original.id]: id } : {};
      const change: ChangeSet = { resource_ids: [...new Set([original?.id || id, id])], reason: value("reason"), evidence: [], writes, deletions: [], renames,
        validation: { status: "partial", checks: [], unverified: ["用户主动修改，待人工审核和行为验证"] } };
      buttons.forEach(button => { button.disabled = true; }); result.textContent = "正在准备并提交提案…";
      const token = required<HTMLMetaElement>(document, 'meta[name="sm-csrf"]').content;
      const response = await fetch("/proposals", { method: "POST", headers: { "Content-Type": "application/json", "X-SM-CSRF": token }, body: JSON.stringify(change) });
      const raw: unknown = await response.json(); const body = responseFields(raw);
      if (!response.ok) throw new Error(body.error || "提交失败，请查看运行状态。");
      if (!body.message) throw new Error("服务未返回提案结果。");
      result.textContent = body.message;
      if (body.url && /^https?:\/\//.test(body.url)) {
        const link = document.createElement("a"); link.href = body.url; link.target = "_blank"; link.rel = "noopener noreferrer";
        link.textContent = " 打开 MR ↗"; result.append(link);
      }
    } catch (error: unknown) {
      result.classList.add("error"); result.textContent = error instanceof Error ? error.message : String(error);
    } finally { buttons.forEach(button => { button.disabled = false; }); }
  }
  form.addEventListener("submit", event => { event.preventDefault(); void submit(false); });
  form.querySelector(".retire")?.addEventListener("click", () => { void submit(true); });
  render();
}
