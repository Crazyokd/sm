"use strict";
function required(root, selector) {
    const element = root.querySelector(selector);
    if (!element)
        throw new Error(`缺少页面控件：${selector}`);
    return element;
}
function responseFields(body) {
    if (typeof body !== "object" || body === null)
        throw new Error("服务返回了无效结果。");
    const fields = {};
    for (const name of ["message", "error", "url"]) {
        if (name in body) {
            const value = Reflect.get(body, name);
            if (typeof value !== "string")
                throw new Error("服务返回了无效结果。");
            fields[name] = value;
        }
    }
    return fields;
}
for (const form of document.querySelectorAll(".installation-editor")) {
    form.addEventListener("submit", event => {
        event.preventDefault();
        const result = required(form, ".form-result");
        const button = required(form, "button");
        const submit = async () => {
            button.disabled = true;
            result.textContent = "正在提交修改提案…";
            try {
                const token = required(document, 'meta[name="sm-csrf"]').content;
                const reason = required(form, "input[name=reason]").value.trim();
                const response = await fetch(`/installations/${encodeURIComponent(form.dataset.id ?? "")}`, {
                    method: "POST", headers: { "Content-Type": "application/json", "X-SM-CSRF": token },
                    body: JSON.stringify({ enabled: form.dataset.enabled === "true", reason }),
                });
                const body = responseFields(await response.json());
                if (!response.ok || !body.message)
                    throw new Error(body.error ?? "提交失败，请查看运行状态。");
                result.textContent = body.message;
                if (body.url && /^https?:\/\//.test(body.url)) {
                    const link = document.createElement("a");
                    link.href = body.url;
                    link.textContent = "打开 MR ↗";
                    link.target = "_blank";
                    link.rel = "noopener noreferrer";
                    result.append(link);
                }
            }
            catch (error) {
                result.textContent = error instanceof Error ? error.message : "提交失败";
            }
            finally {
                button.disabled = false;
            }
        };
        void submit();
    });
}
for (const form of document.querySelectorAll(".resource-editor")) {
    const encoded = required(form, ".editor-data").textContent;
    if (!encoded)
        throw new Error("资源目录尚未加载。");
    // Rust renders this data from its validated catalog and content cache.
    const data = JSON.parse(encoded);
    const original = data.catalog.resources.find(resource => resource.id === form.dataset.existing);
    const originalFiles = data.files;
    const drafts = { ...originalFiles };
    const defaults = {};
    const result = required(form, ".form-result");
    const fields = required(form, ".payload-fields");
    const buttons = form.querySelectorAll(".form-actions button");
    let currentFile = null;
    let textarea = null;
    const control = (name) => {
        const input = form.elements.namedItem(name);
        if (input instanceof HTMLInputElement || input instanceof HTMLSelectElement || input instanceof HTMLTextAreaElement)
            return input;
        throw new Error(`缺少表单字段：${name}`);
    };
    const value = (name) => control(name).value.trim();
    const currentKind = () => {
        const kind = value("kind");
        switch (kind) {
            case "skill":
            case "bundle":
            case "hook":
            case "rule":
            case "agent":
            case "mcp": return kind;
            default: throw new Error("无效的资源类型。");
        }
    };
    const clients = () => (currentKind() === "skill" || currentKind() === "bundle") ? ["codex", "kimi"] : [...form.querySelectorAll("input[name=agents]:checked")].map(input => {
        if (input.value === "codex" || input.value === "kimi")
            return input.value;
        throw new Error("无效的客户端。");
    });
    const save = () => { if (textarea && currentFile)
        drafts[currentFile] = textarea.value; };
    const templates = (kind, client, id) => {
        switch (kind) {
            case "skill": return ["SKILL.md", `---\nname: ${id}\ndescription: 说明何时应使用这项技能\n---\n\n# ${id}\n\n写下工作流程和完成标准。\n`];
            case "bundle": return ["bundle.json", JSON.stringify({ entries: [] }, null, 2) + "\n"];
            case "rule": return ["AGENTS.md", "# 工作约定\n\n在这里写下需要持续遵循的约定。\n"];
            case "agent": return client === "codex" ? ["agent.toml", `name = "${id}"\ndescription = "说明角色用途"\ndeveloper_instructions = "描述角色的工作方法"\n`] : ["agent.md", `---\nname: ${id}\ndescription: 说明角色用途\n---\n\n描述角色的工作方法。\n`];
            case "mcp": return client === "codex" ? ["config.toml", `[mcp_servers.${id}]\ncommand = "your-mcp-command"\n`] : ["mcp.json", JSON.stringify({ mcpServers: { [id]: { command: "your-mcp-command" } } }, null, 2)];
            case "hook": return client === "codex" ? ["hooks.json", JSON.stringify({ hooks: { Stop: [{ hooks: [{ type: "command", command: "true" }] }] } }, null, 2)] : ["config.toml", '[[hooks]]\nevent = "Stop"\ncommand = "true"\n'];
        }
    };
    const renamedPath = (path, id) => original && id !== original.id ? path.replace(`resources/${original.id}/`, `resources/${id}/`) : path;
    const sourcePath = (path, id) => original && id !== original.id ? path.replace(`resources/${id}/`, `resources/${original.id}/`) : path;
    const payloadPath = (client, id, kind) => {
        const path = (original?.kind === "skill" || original?.kind === "bundle") ? original.payload : original?.payloads[client];
        return renamedPath(path || `resources/${id}/${kind === "skill" || kind === "bundle" || kind === "rule" ? "shared" : client}`, id);
    };
    const render = () => {
        save();
        const upstream = value("origin") === "upstream";
        required(form, ".upstream-fields").hidden = !upstream;
        required(form, ".agent-options").hidden = (currentKind() === "skill" || currentKind() === "bundle");
        for (const name of ["project", "project_path"]) {
            const label = control(name).closest("label");
            if (label)
                label.hidden = value("scope") !== "project";
        }
        fields.replaceChildren();
        currentFile = null;
        textarea = null;
        if (upstream)
            return;
        const id = value("resource_id") || "new-resource";
        const kind = currentKind();
        const names = new Set();
        for (const client of clients()) {
            const root = payloadPath(client, id, kind);
            const existing = Object.keys(originalFiles).map(path => renamedPath(path, id)).filter(path => path.startsWith(root + "/"));
            if (existing.length) {
                for (const path of existing) {
                    names.add(path);
                    if (!(path in drafts))
                        drafts[path] = originalFiles[sourcePath(path, id)] ?? "";
                }
                continue;
            }
            const [filename, content] = templates(kind, client, id);
            const path = `${root}/${filename}`;
            names.add(path);
            defaults[path] = content;
            if (!(path in drafts))
                drafts[path] = content;
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
        select.addEventListener("change", () => { save(); currentFile = select.value; if (textarea)
            textarea.value = drafts[currentFile] || ""; });
    };
    for (const name of ["kind", "origin", "scope", "agents"]) {
        for (const input of form.querySelectorAll(`[name=${name}]`))
            input.addEventListener("change", render);
    }
    control("resource_id").addEventListener("change", () => {
        save();
        const prefix = currentFile?.match(/^resources\/[^/]+\//)?.[0];
        const next = `resources/${value("resource_id") || "new-resource"}/`;
        if (prefix && prefix !== next)
            for (const [path, content] of Object.entries(drafts)) {
                if (path.startsWith(prefix) && content !== defaults[path])
                    drafts[next + path.slice(prefix.length)] = content;
            }
        render();
    });
    async function submit(retire = false) {
        save();
        result.textContent = "";
        result.classList.remove("error");
        try {
            const id = retire && original ? original.id : value("resource_id");
            if (!/^[a-z0-9][a-z0-9_-]*$/.test(id))
                throw new Error("请填写有效的资源名称。");
            if ((!original || id !== original.id) && data.catalog.resources.some(resource => resource.id === id))
                throw new Error("资源名称已存在，请从已有资源的详情修改。");
            if (!value("reason"))
                throw new Error("请说明修改原因。");
            if (retire && !original)
                throw new Error("只能从已有资源申请移除纳管。");
            const catalog = structuredClone(data.catalog);
            const writes = {};
            if (!retire) {
                const kind = currentKind(), selected = clients();
                if (!selected.length)
                    throw new Error("请选择至少一个客户端。");
                const scope = value("scope"), project = value("project");
                if (scope !== "user" && scope !== "project")
                    throw new Error("无效的作用域。");
                if (scope === "project" && !project)
                    throw new Error("项目级资源需要项目归属。");
                if (kind === "hook" && scope === "project" && selected.includes("kimi"))
                    throw new Error("Kimi 当前不支持原生项目级 Hook，请调整适用客户端或作用域。");
                if (scope === "project" && !(project in catalog.projects)) {
                    if (!value("project_path"))
                        throw new Error("新项目需要填写约定目录，例如 ~/repo/app。");
                    catalog.projects[project] = value("project_path");
                }
                const attribution = scope === "project" ? { scope, project } : { scope, project: null };
                const origin = value("origin");
                if (origin !== "self" && origin !== "upstream")
                    throw new Error("无效的资源来源。");
                let source;
                if (origin === "upstream") {
                    const upstream = value("upstream_git") ? { git: value("upstream_git") } : original?.origin === "upstream" ? original.upstream : undefined;
                    if (!upstream)
                        throw new Error("请填写上游 Git 地址。");
                    source = { origin, upstream };
                }
                else
                    source = { origin };
                let definition;
                if (kind === "skill" || kind === "bundle")
                    definition = { kind, payload: origin === "upstream" ? value("upstream_path") || "." : payloadPath("codex", id, kind) };
                else {
                    const payloads = {};
                    const oldPath = (original?.kind === "skill" || original?.kind === "bundle") ? original.payload : Object.values(original?.payloads || {})[0];
                    const changedPath = value("upstream_path") !== oldPath;
                    for (const client of selected)
                        payloads[client] = origin === "self" ? payloadPath(client, id, kind) : original && original.kind !== "skill" && original.kind !== "bundle" && original.payloads[client] && !changedPath ? original.payloads[client] : value("upstream_path") || ".";
                    definition = { kind, agents: selected, payloads };
                }
                if (origin === "self")
                    for (const option of fields.querySelectorAll("option")) {
                        const content = drafts[option.value];
                        if (content !== undefined && content !== originalFiles[sourcePath(option.value, id)])
                            writes[option.value] = content;
                    }
                catalog.resources = catalog.resources.filter(resource => resource.id !== (original?.id || id));
                catalog.resources.push({ id, ...attribution, ...source, ...definition });
            }
            else
                catalog.resources = catalog.resources.filter(resource => resource.id !== id);
            writes["catalog.json"] = JSON.stringify(catalog, null, 2) + "\n";
            const renames = !retire && original && id !== original.id ? { [original.id]: id } : {};
            const change = { resource_ids: [...new Set([original?.id || id, id])], reason: value("reason"), evidence: [], writes, deletions: [], renames,
                validation: { status: "partial", checks: [], unverified: ["用户主动修改，待人工审核和行为验证"] } };
            buttons.forEach(button => { button.disabled = true; });
            result.textContent = "正在准备并提交提案…";
            const token = required(document, 'meta[name="sm-csrf"]').content;
            const response = await fetch("/proposals", { method: "POST", headers: { "Content-Type": "application/json", "X-SM-CSRF": token }, body: JSON.stringify(change) });
            const raw = await response.json();
            const body = responseFields(raw);
            if (!response.ok)
                throw new Error(body.error || "提交失败，请查看运行状态。");
            if (!body.message)
                throw new Error("服务未返回提案结果。");
            result.textContent = body.message;
            if (body.url && /^https?:\/\//.test(body.url)) {
                const link = document.createElement("a");
                link.href = body.url;
                link.target = "_blank";
                link.rel = "noopener noreferrer";
                link.textContent = " 打开 MR ↗";
                result.append(link);
            }
        }
        catch (error) {
            result.classList.add("error");
            result.textContent = error instanceof Error ? error.message : String(error);
        }
        finally {
            buttons.forEach(button => { button.disabled = false; });
        }
    }
    form.addEventListener("submit", event => { event.preventDefault(); void submit(false); });
    form.querySelector(".retire")?.addEventListener("click", () => { void submit(true); });
    render();
}
export {};
