"use strict";

for (const form of document.querySelectorAll(".resource-editor")) {
  const data = JSON.parse(form.querySelector(".editor-data").textContent);
  const original = data.catalog.resources.find(r => r.id === form.dataset.existing);
  const originalFiles = data.files;
  const drafts = {...originalFiles};
  const defaults = {};
  const result = form.querySelector(".form-result");
  const fields = form.querySelector(".payload-fields");
  let currentFile = null;
  let textarea = null;
  const value = name => form.elements.namedItem(name).value.trim();
  const agents = () => value("kind") === "skill" ? ["codex", "kimi"] : [...form.querySelectorAll("input[name=agents]:checked")].map(input => input.value);
  const save = () => { if (textarea && currentFile) drafts[currentFile] = textarea.value; };
  const templates = (kind, agent, id) => {
    if (kind === "skill") return ["SKILL.md", `---\nname: ${id}\ndescription: 说明何时应使用这项技能\n---\n\n# ${id}\n\n写下工作流程和完成标准。\n`];
    if (kind === "rule") return ["AGENTS.md", "# 工作约定\n\n在这里写下需要持续遵循的约定。\n"];
    if (kind === "agent") return agent === "codex" ? ["agent.toml", `name = "${id}"\ndescription = "说明角色用途"\ndeveloper_instructions = "描述角色的工作方法"\n`] : ["agent.md", `---\nname: ${id}\ndescription: 说明角色用途\n---\n\n描述角色的工作方法。\n`];
    if (kind === "mcp") return agent === "codex" ? ["config.toml", `[mcp_servers.${id}]\ncommand = "your-mcp-command"\n`] : ["mcp.json", JSON.stringify({mcpServers:{[id]:{command:"your-mcp-command"}}}, null, 2)];
    return agent === "codex" ? ["hooks.json", JSON.stringify({hooks:{Stop:[{hooks:[{type:"command",command:"true"}]}]}}, null, 2)] : ["config.toml", '[[hooks]]\nevent = "Stop"\ncommand = "true"\n'];
  };
  const renamedPath = (path, id) => original && id !== original.id ? path.replace(`resources/${original.id}/`, `resources/${id}/`) : path;
  const sourcePath = (path, id) => original && id !== original.id ? path.replace(`resources/${id}/`, `resources/${original.id}/`) : path;
  const payloadPath = (agent, id, kind) => renamedPath((kind === "skill" ? original?.payload : original?.payloads[agent]) || `resources/${id}/${["skill","rule"].includes(kind) ? "shared" : agent}`, id);
  const render = () => {
    save();
    const upstream = value("origin") === "upstream";
    form.querySelector(".upstream-fields").hidden = !upstream;
    form.querySelector(".agent-options").hidden = value("kind") === "skill";
    for (const name of ["project", "project_path"]) form.elements.namedItem(name).closest("label").hidden = value("scope") !== "project";
    fields.replaceChildren();
    currentFile = null;
    textarea = null;
    if (upstream) return;
    const id = value("resource_id") || "new-resource";
    const kind = value("kind");
    const names = new Set();
    for (const agent of agents()) {
      const root = payloadPath(agent, id, kind);
      const existing = Object.keys(originalFiles).map(path => renamedPath(path, id)).filter(path => path.startsWith(root + "/"));
      if (existing.length) {
        existing.forEach(path => {
          names.add(path);
          if (!(path in drafts)) drafts[path] = originalFiles[sourcePath(path, id)];
        });
        continue;
      }
      const [filename, content] = templates(kind, agent, id);
      const path = `${root}/${filename}`;
      names.add(path);
      defaults[path] = content;
      if (!(path in drafts)) drafts[path] = content;
    }
    const label = document.createElement("label");
    label.textContent = "修改内容";
    const select = document.createElement("select");
    for (const path of names) {
      const option = document.createElement("option");
      option.value = path;
      option.textContent = path.split("/").slice(2).join("/");
      select.append(option);
    }
    label.append(select);
    const textLabel = document.createElement("label");
    textLabel.textContent = "资源内容";
    textarea = document.createElement("textarea");
    textarea.rows = 10;
    currentFile = select.value;
    textarea.value = drafts[currentFile] || "";
    textLabel.append(textarea);
    fields.append(label, textLabel);
    select.addEventListener("change", () => {
      save(); currentFile = select.value; textarea.value = drafts[currentFile] || "";
    });
  };
  for (const name of ["kind", "origin", "scope", "agents"]) {
    for (const input of form.querySelectorAll(`[name=${name}]`)) input.addEventListener("change", render);
  }
  form.elements.namedItem("resource_id").addEventListener("change", () => {
    save();
    const prefix = currentFile?.match(/^resources\/[^/]+\//)?.[0];
    const next = `resources/${value("resource_id") || "new-resource"}/`;
    if (prefix && prefix !== next) {
      for (const [path, content] of Object.entries(drafts)) {
        if (path.startsWith(prefix) && content !== defaults[path]) drafts[next + path.slice(prefix.length)] = content;
      }
    }
    render();
  });

  async function submit(retire = false) {
    save();
    const submitButton = form.querySelector("button[type=submit]");
    result.textContent = "";
    result.classList.remove("error");
    try {
      const id = retire && original ? original.id : value("resource_id"), kind = value("kind"), origin = value("origin");
      if (!/^[a-z0-9][a-z0-9_-]*$/.test(id)) throw new Error("请填写有效的资源名称。");
      if ((!original || id !== original.id) && data.catalog.resources.some(r => r.id === id)) throw new Error("资源名称已存在，请从已有资源的详情修改。");
      const selected = agents();
      if (!selected.length) throw new Error("请选择至少一个 Agent。");
      if (!value("reason")) throw new Error("请说明修改原因。");
      const scope = value("scope"), project = scope === "project" ? value("project") : null;
      if (scope === "project" && !project) throw new Error("项目级资源需要项目归属。");
      if (!retire && kind === "hook" && scope === "project" && selected.includes("kimi")) throw new Error("Kimi 当前不支持原生项目级 Hook，请调整适用 Agent 或作用域。");
      const catalog = structuredClone(data.catalog);
      if (project && !(project in catalog.projects)) {
        if (!value("project_path")) throw new Error("新项目需要填写约定目录，例如 ~/repo/app。");
        catalog.projects[project] = value("project_path");
      }
      const entry = {id, kind, origin, scope, project, agents:selected, payloads:{}};
      const writes = {};
      if (origin === "upstream" && !retire) {
        entry.upstream = value("upstream_git") ? {git:value("upstream_git")} : original?.upstream;
        if (!entry.upstream) throw new Error("请填写上游 Git 地址。");
        const changedPath = value("upstream_path") !== Object.values(original?.payloads || {})[0];
        if (kind !== "skill") for (const agent of selected) entry.payloads[agent] = original?.payloads[agent] && !changedPath ? original.payloads[agent] : value("upstream_path") || ".";
      } else if (!retire) {
        for (const agent of selected) entry.payloads[agent] = payloadPath(agent, id, kind);
        for (const path of Array.from(fields.querySelectorAll("option"), option => option.value)) {
          const content = drafts[path];
          if (content !== originalFiles[sourcePath(path, id)]) writes[path] = content;
        }
      }
      if (kind === "skill") {
        entry.payload = origin === "upstream" ? value("upstream_path") || "." : payloadPath("", id, kind);
        delete entry.agents;
        delete entry.payloads;
      }
      catalog.resources = catalog.resources.filter(r => r.id !== (original?.id || id));
      if (!retire) catalog.resources.push(entry);
      writes["catalog.json"] = JSON.stringify(catalog, null, 2) + "\n";
      const renames = !retire && original && id !== original.id ? {[original.id]:id} : {};
      const change = {resource_ids:[...new Set([original?.id || id, id])], reason:value("reason"), evidence:[], writes, deletions:[], renames,
                      validation:{status:"partial", checks:[], unverified:["用户主动修改，待人工审核和行为验证"]}};
      submitButton.disabled = true;
      result.textContent = "正在准备并提交提案…";
      const response = await fetch("/proposals", {method:"POST", headers:{"Content-Type":"application/json", "X-SM-CSRF":document.querySelector('meta[name="sm-csrf"]').content}, body:JSON.stringify(change)});
      const body = await response.json();
      if (!response.ok) throw new Error(body.error || "提交失败，请查看运行状态。");
      result.textContent = body.message;
      if (body.url && /^https?:\/\//.test(body.url)) {
        const link = document.createElement("a");
        link.href = body.url; link.target = "_blank"; link.rel = "noopener noreferrer";
        link.textContent = " 打开 MR ↗"; result.append(link);
      }
    } catch (error) {
      result.classList.add("error"); result.textContent = error.message;
    } finally { submitButton.disabled = false; }
  }
  form.addEventListener("submit", event => { event.preventDefault(); submit(false); });
  form.querySelector(".retire")?.addEventListener("click", () => submit(true));
  render();
}
