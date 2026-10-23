import type { ChangeSet, EditorData, Resource } from "../frontend/types.js";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

declare global { interface Window { smTestRequest: ChangeSet | null } }
type BrowserCommand = { type: "open" | "navigate"; url: string } | { type: "evaluate"; code: string };
function browser<T>(command: BrowserCommand): T {
  const payload = Buffer.from(JSON.stringify(command), "utf-8").toString("base64");
  // Browser Harness owns the Python bridge; all regression logic below is type checked.
  const bridge = String.raw`import base64,json,os,time
request=json.loads(base64.b64decode("${payload}"))
if request['type']=='open':
    tab_id=os.environ.get('SM_BROWSER_TAB_ID')
    if tab_id: switch_tab(tab_id);goto_url(request['url'])
    else: new_tab(request['url'])
    wait_for_load();cdp('Network.enable');cdp('Network.setCacheDisabled',cacheDisabled=True);result=None
elif request['type']=='navigate':
    previous=js('performance.timeOrigin');goto_url(request['url']);deadline=time.monotonic()+10
    while js('performance.timeOrigin')==previous and time.monotonic()<deadline: time.sleep(0.05)
    wait_for_load();result=None
else:
    try:
        js("globalThis.__smEditorTestCode='';globalThis.__smEditorTestResult='';null")
        encoded=base64.b64encode(request['code'].encode('utf-8')).decode('ascii')
        for offset in range(0,len(encoded),16384):
            js('globalThis.__smEditorTestCode+='+json.dumps(encoded[offset:offset+16384])+';null')
        js('''(async () => { const code = new TextDecoder().decode(Uint8Array.from(atob(globalThis.__smEditorTestCode), char => char.charCodeAt(0))); const value = await (0, eval)(code); globalThis.__smEditorTestResult = JSON.stringify(value === undefined ? null : value); return null; })()''')
        length=js('globalThis.__smEditorTestResult.length')
        result=json.loads(''.join(js(f'globalThis.__smEditorTestResult.slice({offset},{offset+16384})') for offset in range(0,length,16384)))
    finally:
        js('delete globalThis.__smEditorTestCode;delete globalThis.__smEditorTestResult;null')
print(json.dumps(result))
`;
  const output = execFileSync("browser-harness", [], { input: bridge, encoding: "utf-8", timeout: 30000 });
  return JSON.parse(output) as T;
}
function evaluate<A, T>(work: (args: A) => T, args: A): T {
  return browser<T>({ type: "evaluate", code: `(${work.toString()})(${JSON.stringify(args)})` });
}
const base = process.env["SM_BROWSER_URL"] || "http://127.0.0.1:8765";
browser<null>({ type: "open", url: base + "/resources" });
const existing = evaluate(() => {
  const text = document.querySelector(".editor-data")?.textContent;
  if (!text) throw new Error("An isolated catalog with a self Skill is required.");
  const data: EditorData = JSON.parse(text);
  return data.catalog.resources.find(resource => resource.kind === "skill" && resource.origin === "self")?.id;
}, null);
assert.ok(existing, "An existing self Skill is required");
function prepare(path: string, catalog?: EditorData["catalog"]): void {
  browser<null>({ type: "navigate", url: base + path });
  evaluate(catalog => {
    if (catalog) {
      // Feed the prior intercepted catalog into the next proposal without changing the server.
      const clone = window.structuredClone;
      window.structuredClone = <T>(value: T): T => clone(typeof value === "object" && value !== null && "resources" in value ? catalog : value) as T;
    }
    window.smTestRequest = null;
    window.fetch = async (_url: RequestInfo | URL, options?: RequestInit): Promise<Response> => {
      if (typeof options?.body !== "string") throw new Error("Expected a JSON proposal body");
      window.smTestRequest = JSON.parse(options.body) as ChangeSet;
      return new Response(JSON.stringify({ message: "Intercepted test proposal" }), { status: 200, headers: { "Content-Type": "application/json" } });
    };
  }, catalog);
}
prepare("/resources");
evaluate(id => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = id; name.dispatchEvent(new Event("change"));
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Duplicate ID test";
  form.requestSubmit();
}, existing);
assert.equal(evaluate(() => window.smTestRequest, null), null, "Creation accepted an existing resource ID");
prepare("/resources");
const edited = evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = "draft-one"; name.dispatchEvent(new Event("change"));
  form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!.value += "\nPRESERVE_EDITED_DRAFT\n";
  name.value = "draft-two"; name.dispatchEvent(new Event("change"));
  return form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!.value;
}, null);
assert.ok(edited.includes("PRESERVE_EDITED_DRAFT"), "Correcting the name discarded edited content");
prepare("/resources/" + existing);
const readonly = evaluate(() => document.querySelector<HTMLInputElement>("[name=resource_id]")!.readOnly, null);
evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = "renamed-probe"; name.dispatchEvent(new Event("change"));
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Rename test";
  form.requestSubmit();
}, null);
const proposal = evaluate(() => window.smTestRequest, null);
assert.equal(readonly, false, "Existing resource cannot be renamed");
assert.ok(proposal);
assert.deepEqual(proposal.renames, { [existing]: "renamed-probe" });
prepare("/resources");
const resources = evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const text = form.querySelector(".editor-data")!.textContent!;
  const data: EditorData = JSON.parse(text);
  const options = [...form.querySelectorAll<HTMLOptionElement>("[name=kind] option")].map(option => option.value);
  return { resources: data.catalog.resources, catalog: data.catalog, options };
}, null);
assert.ok(!resources.options.includes("bundle"), "A collection is still offered as a resource type");
assert.deepEqual(resources.options, ["skill", "hook", "rule", "agent", "mcp", "prompt", "policy"], "Only complete resource types should be offered");
const fileResources = resources.resources.filter(resource => resource.origin === "self" && "files" in resource && resource.files.entries.some(entry => "source" in entry || "tree" in entry));
assert.ok(fileResources.length >= 2, "Two independent self file resources are required");
const [first, second] = fileResources as [Resource, Resource, ...Resource[]];
assert.notEqual(first.id, second.id);
browser<null>({ type: "navigate", url: base + `/resources?kind=${first.kind}&q=${first.id}` });
const table = evaluate(() => document.querySelector("tbody")!.textContent!, null);
assert.ok(table.includes(first.id) && !table.includes(second.id), "Filtering did not isolate the selected resource");
prepare("/resources/" + first.id);
const displayed = evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const select = form.querySelector<HTMLSelectElement>(".payload-fields select")!;
  select.value = [...select.options].find(option => !option.value.startsWith("fragment:"))!.value;
  select.dispatchEvent(new Event("change"));
  const editor = form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!;
  editor.value += "\nINDEPENDENT_FILE_EDIT\n";
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Edit one independently managed resource";
  form.requestSubmit();
  return form.querySelector(".payload-fields select")!.textContent!;
}, null);
assert.ok(displayed.includes("."), "The content editor did not show the projected target");
const firstChange = evaluate(() => window.smTestRequest, null);
assert.ok(firstChange);
assert.deepEqual(firstChange.resource_ids, [first.id]);
const firstCatalog: EditorData["catalog"] = JSON.parse(firstChange.writes["catalog.json"]!);
assert.deepEqual(firstCatalog.resources.find(resource => resource.id === second.id), second);
assert.ok(Object.keys(firstChange.writes).every(path => path === "catalog.json" || path.startsWith(`resources/${first.id}/`)));
prepare("/resources/" + second.id);
evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Retire one independent resource";
  form.querySelector<HTMLButtonElement>(".retire")!.click();
}, null);
const retirement = evaluate(() => window.smTestRequest, null);
assert.ok(retirement);
assert.deepEqual(retirement.resource_ids, [second.id]);
const retiredCatalog: EditorData["catalog"] = JSON.parse(retirement.writes["catalog.json"]!);
assert.deepEqual(retiredCatalog.resources.find(resource => resource.id === first.id), first);
assert.ok(!retiredCatalog.resources.some(resource => resource.id === second.id));

const stagedCatalog = structuredClone(resources.catalog);
const supportId = "ui-shared-runtime-probe", supportSource = "ui-support-source-probe";
stagedCatalog.dependencies ??= {};
stagedCatalog.dependencies[supportId] = { origin: "upstream", upstream: supportSource, files: { root: "hooks", entries: [{ target: ".codex/hooks/ui-shared-runtime.sh", source: "runtime.sh" }] } };
stagedCatalog.upstreams[supportSource] = { git: "https://example.invalid/shared-runtime.git" };
for (const id of [first.id, second.id]) {
  const resource = stagedCatalog.resources.find(resource => resource.id === id)!;
  resource.dependencies = [...(resource.dependencies ?? []), supportId];
}
let afterRetirement = stagedCatalog;
for (const [index, resource] of [first, second].entries()) {
  prepare("/resources/" + resource.id, afterRetirement);
  evaluate(() => {
    const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
    form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Retire a shared supporting file consumer";
    form.querySelector<HTMLButtonElement>(".retire")!.click();
  }, null);
  const change = evaluate(() => window.smTestRequest, null);
  assert.ok(change);
  afterRetirement = JSON.parse(change.writes["catalog.json"]!) as EditorData["catalog"];
  assert.equal(supportId in (afterRetirement.dependencies ?? {}), index === 0, "Supporting files must remain until the last consumer retires");
  assert.equal(supportSource in afterRetirement.upstreams, index === 0, "A retained dependency must keep its upstream source registered");
}

const binaryResource = resources.resources.find(resource => resource.origin === "self" && "files" in resource && resource.files.entries.some(entry => "source" in entry && /\.(png|jpe?g|gif|webp|woff2?)$/i.test(entry.source)));
if (binaryResource) {
  prepare("/resources/" + binaryResource.id);
  evaluate(() => {
    const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
    form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Keep binary assets unchanged";
    form.requestSubmit();
  }, null);
  const unchangedAssets = evaluate(() => window.smTestRequest, null);
  assert.ok(unchangedAssets);
  assert.deepEqual(Object.keys(unchangedAssets.writes), ["catalog.json"], "An uncached binary asset became an empty text write");
}

const treeSkill = resources.resources.find(resource => resource.origin === "self" && resource.kind === "skill" && "files" in resource && resource.files.entries.some(entry => "tree" in entry));
assert.ok(treeSkill && "files" in treeSkill, "An individual Skill tree is required");
prepare("/resources/" + treeSkill.id);
const treeEdit = evaluate(id => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const data: EditorData = JSON.parse(form.querySelector(".editor-data")!.textContent!);
  const resource = data.catalog.resources.find(candidate => candidate.id === id)!;
  if (!("files" in resource)) throw new Error("Expected a Skill tree");
  const entry = resource.files.entries.find(candidate => "tree" in candidate)!;
  if (!("tree" in entry)) throw new Error("Expected a Skill tree entry");
  const root = `${resource.files.root}/${entry.tree}`.split("/").filter(part => part && part !== ".").join("/");
  const prefix = root ? root + "/" : "";
  const paths = Object.keys(data.files).filter(path => path.startsWith(prefix));
  if (!paths.length) throw new Error("The isolated Skill tree needs cached text files");
  const select = form.querySelector<HTMLSelectElement>(".payload-fields select")!;
  for (const path of paths) {
    const option = [...select.options].find(candidate => candidate.value === path);
    if (!option || !option.textContent?.includes(path.slice(prefix.length))) throw new Error("A Skill helper was omitted from the editor");
  }
  const path = paths.find(candidate => !candidate.endsWith("SKILL.md")) ?? paths[0]!;
  select.value = path; select.dispatchEvent(new Event("change"));
  const editor = form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!;
  editor.value += "\nPRESERVE_SKILL_TREE_EDIT\n";
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = "renamed-tree-probe"; name.dispatchEvent(new Event("change"));
  const renamedPath = path.replace(`resources/${id}/`, "resources/renamed-tree-probe/");
  const renamedSelect = form.querySelector<HTMLSelectElement>(".payload-fields select")!;
  renamedSelect.value = renamedPath; renamedSelect.dispatchEvent(new Event("change"));
  const retained = form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!.value;
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Edit and rename one complete Skill tree";
  form.requestSubmit();
  return { renamedPath, retained };
}, treeSkill.id);
assert.ok(treeEdit.retained.includes("PRESERVE_SKILL_TREE_EDIT"), "Renaming discarded a Skill tree file edit");
const treeChange = evaluate(() => window.smTestRequest, null);
assert.ok(treeChange);
assert.deepEqual(treeChange.resource_ids, [treeSkill.id, "renamed-tree-probe"]);
assert.ok(treeChange.writes[treeEdit.renamedPath]?.includes("PRESERVE_SKILL_TREE_EDIT"));
assert.ok(Object.keys(treeChange.writes).every(path => path === "catalog.json" || path.startsWith("resources/renamed-tree-probe/")));
const treeCatalog: EditorData["catalog"] = JSON.parse(treeChange.writes["catalog.json"]!);
const renamedTree = treeCatalog.resources.find(resource => resource.id === "renamed-tree-probe");
assert.ok(renamedTree && "files" in renamedTree);
assert.deepEqual(renamedTree.files.entries, treeSkill.files.entries);
for (const resource of resources.resources) if (resource.id !== treeSkill.id) assert.deepEqual(treeCatalog.resources.find(next => next.id === resource.id), resource);

const configuration = resources.resources.find(resource => resource.origin === "self" && "files" in resource && resource.files.entries.some(entry => "fragment" in entry));
assert.ok(configuration && "files" in configuration, "An independently managed configuration fragment file is required");
const configEntry = configuration.files.entries.find(entry => "fragment" in entry)!;
assert.ok("fragment" in configEntry);
const fragmentPath = `${configuration.files.root}/${configEntry.fragment}`.split("/").filter(part => part && part !== ".").join("/");
prepare("/resources/" + configuration.id);
evaluate(path => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const select = form.querySelector<HTMLSelectElement>(".payload-fields select")!;
  select.value = path;
  select.dispatchEvent(new Event("change"));
  const editor = form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!;
  const fragment: Record<string, unknown> = JSON.parse(editor.value);
  const edit = (object: Record<string, unknown>): boolean => {
    for (const key of Object.keys(object)) {
      const child = object[key];
      if (typeof child === "string") { object[key] = child + "-edited"; return true; }
      if (typeof child === "boolean") { object[key] = !child; return true; }
      if (typeof child === "object" && child !== null && edit(child as Record<string, unknown>)) return true;
    }
    return false;
  };
  if (!edit(fragment)) throw new Error("An editable configuration value is required");
  editor.value = JSON.stringify(fragment);
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Edit this configuration fragment";
  form.requestSubmit();
}, fragmentPath);
const configChange = evaluate(() => window.smTestRequest, null);
assert.ok(configChange);
assert.deepEqual(configChange.resource_ids, [configuration.id]);
assert.deepEqual(Object.keys(configChange.writes).sort(), ["catalog.json", fragmentPath].sort(), "Configuration edit did not stay within the resource-owned fragment file");
assert.doesNotThrow(() => JSON.parse(configChange.writes[fragmentPath]!));
const configCatalog: EditorData["catalog"] = JSON.parse(configChange.writes["catalog.json"]!);
for (const resource of resources.resources) assert.deepEqual(configCatalog.resources.find(next => next.id === resource.id), resource);

const completeHook = resources.resources.find(resource => resource.origin === "self" && resource.kind === "hook" && "files" in resource && resource.files.entries.some(entry => "fragment" in entry) && resource.files.entries.some(entry => "source" in entry));
assert.ok(completeHook && "files" in completeHook, "A complete Hook with configuration and its script is required");
prepare("/resources/" + completeHook.id);
const hookEdit = evaluate(id => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const data: EditorData = JSON.parse(form.querySelector(".editor-data")!.textContent!);
  const resource = data.catalog.resources.find(resource => resource.id === id)!;
  if (!("files" in resource)) throw new Error("Expected complete Hook files");
  const paths: string[] = [];
  const select = form.querySelector<HTMLSelectElement>(".payload-fields select")!;
  for (const entry of resource.files.entries) {
    if (!("fragment" in entry) && !("source" in entry)) continue;
    const path = `${resource.files.root}/${"source" in entry ? entry.source : entry.fragment}`.split("/").filter(part => part && part !== ".").join("/");
    if (!(path in data.files)) continue;
    if (![...select.options].some(option => option.value === path)) throw new Error("A Hook configuration or script was omitted from the editor");
    select.value = path; select.dispatchEvent(new Event("change"));
    const editor = form.querySelector<HTMLTextAreaElement>(".payload-fields textarea")!;
    if (editor.readOnly) throw new Error("A self Hook file is read only");
    editor.value = "fragment" in entry ? "\n" + JSON.stringify(JSON.parse(editor.value), null, 2) + "\n" : editor.value + "\n# COMPLETE_HOOK_SCRIPT_EDIT\n";
    paths.push(path.replace(`resources/${id}/`, "resources/renamed-complete-hook-probe/"));
  }
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = "renamed-complete-hook-probe"; name.dispatchEvent(new Event("change"));
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Edit and rename a complete Hook with its configuration and script";
  form.requestSubmit();
  return paths;
}, completeHook.id);
const hookChange = evaluate(() => window.smTestRequest, null);
assert.ok(hookChange);
assert.ok(hookEdit.length >= 2 && hookEdit.every(path => path in hookChange.writes), "A complete Hook edit omitted its configuration or script");
assert.ok(Object.values(hookChange.writes).some(text => text.includes("# COMPLETE_HOOK_SCRIPT_EDIT")));
const hookCatalog: EditorData["catalog"] = JSON.parse(hookChange.writes["catalog.json"]!);
const renamedHook = hookCatalog.resources.find(resource => resource.id === "renamed-complete-hook-probe");
assert.ok(renamedHook && "files" in renamedHook);
assert.deepEqual(renamedHook.files.entries, completeHook.files.entries);
assert.deepEqual(renamedHook.dependencies, completeHook.dependencies);
assert.deepEqual(hookCatalog.dependencies, resources.catalog.dependencies);

const dependent = resources.resources.find(resource => resource.dependencies?.length);
assert.ok(dependent, "A resource referencing supporting files is required");
prepare("/resources/" + dependent.id);
const supportDetails = evaluate(() => {
  const details = document.querySelector<HTMLDetailsElement>("details.supporting-files");
  if (!details) throw new Error("Referenced supporting files are missing from the resource detail");
  return { open: details.open, summary: details.querySelector("summary")!.textContent! };
}, null);
assert.equal(supportDetails.open, false, "Supporting files must start collapsed after the resource body");
assert.match(supportDetails.summary, /^支持文件（\d+）$/);
evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = "renamed-dependent-probe"; name.dispatchEvent(new Event("change"));
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Retain supporting file references through resource rename";
  form.requestSubmit();
}, null);
const dependencyChange = evaluate(() => window.smTestRequest, null);
assert.ok(dependencyChange);
const dependencyCatalog: EditorData["catalog"] = JSON.parse(dependencyChange.writes["catalog.json"]!);
assert.deepEqual(dependencyCatalog.resources.find(resource => resource.id === "renamed-dependent-probe")?.dependencies, dependent.dependencies);
assert.deepEqual(dependencyCatalog.dependencies, resources.catalog.dependencies);
assert.deepEqual(dependencyCatalog.upstreams, resources.catalog.upstreams);

const shared = resources.resources.find(resource => resource.origin === "upstream" && resources.resources.some(other => other.id !== resource.id && other.origin === "upstream" && other.upstream === resource.upstream));
assert.ok(shared && shared.origin === "upstream", "Two resources sharing an upstream are required");
prepare("/resources/" + shared.id);
evaluate(id => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const data: EditorData = JSON.parse(form.querySelector(".editor-data")!.textContent!);
  const resource = data.catalog.resources.find(candidate => candidate.id === id)!;
  if ("files" in resource) for (const entry of resource.files.entries) {
    if ("tree" in entry) {
      const root = `${resource.files.root}/${entry.tree}`.split("/").filter(part => part && part !== ".").join("/");
      const prefix = root ? root + "/" : "";
      for (const path of Object.keys(data.files).filter(candidate => candidate.startsWith(prefix))) if (![...form.querySelectorAll<HTMLOptionElement>(".payload-fields option")].some(option => option.value === path)) throw new Error("A cached Skill tree source with a dot root was omitted from the editor");
      continue;
    }
    const source = "source" in entry ? entry.source : "fragment" in entry ? entry.fragment : null;
    if (source === null) continue;
    const key = `${resource.files.root}/${source}`.split("/").filter(part => part && part !== ".").join("/");
    if (key in data.files && ![...form.querySelectorAll<HTMLOptionElement>(".payload-fields option")].some(option => option.value === key)) throw new Error("A cached source with a dot root was omitted from the editor");
  }
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Keep the registered source unchanged";
  form.requestSubmit();
}, shared.id);
const unchangedSource = evaluate(() => window.smTestRequest, null);
assert.ok(unchangedSource);
const unchangedCatalog: EditorData["catalog"] = JSON.parse(unchangedSource.writes["catalog.json"]!);
assert.deepEqual(unchangedCatalog.resources.find(resource => resource.id === shared.id), shared, "An unchanged source reference was replaced");
prepare("/resources/" + shared.id);
evaluate(() => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  form.querySelector<HTMLInputElement>("[name=upstream_git]")!.value = "https://example.invalid/independent-resource.git";
  form.querySelector<HTMLTextAreaElement>("[name=upstream_argv]")!.value = "";
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Change this resource source only";
  form.requestSubmit();
}, null);
const sourceChange = evaluate(() => window.smTestRequest, null);
assert.ok(sourceChange);
assert.deepEqual(sourceChange.resource_ids, [shared.id]);
const sourceCatalog: EditorData["catalog"] = JSON.parse(sourceChange.writes["catalog.json"]!);
const changed = sourceCatalog.resources.find(resource => resource.id === shared.id)!;
assert.ok(changed.origin === "upstream" && changed.upstream !== shared.upstream);
for (const resource of resources.resources) if (resource.id !== shared.id) assert.deepEqual(sourceCatalog.resources.find(next => next.id === resource.id), resource);
assert.ok(sourceCatalog.upstreams[shared.upstream], "Editing one shared source removed the original definition");
const installation = resources.resources.find(resource => resource.origin === "self" && "installation" in resource);
assert.ok(installation && "installation" in installation, "An individual native installation policy is required");
prepare("/resources/" + installation.id);
const toggled = evaluate(id => {
  const form = document.querySelector<HTMLFormElement>(".resource-editor")!;
  const enabled = form.querySelector<HTMLInputElement>("[name=installation_enabled]")!;
  enabled.click();
  const choice = enabled.checked;
  const name = form.querySelector<HTMLInputElement>("[name=resource_id]")!;
  name.value = id; name.dispatchEvent(new Event("change"));
  form.querySelector<HTMLSelectElement>("[name=scope]")!.dispatchEvent(new Event("change"));
  const retained = form.querySelector<HTMLInputElement>("[name=installation_enabled]")!.checked;
  const userScopeOnly = ["scope", "project", "project_path"].every(field => form.querySelector<HTMLInputElement | HTMLSelectElement>(`[name=${field}]`)!.disabled);
  form.querySelector<HTMLTextAreaElement>("[name=reason]")!.value = "Rename this native policy and preserve the chosen enabled state";
  form.requestSubmit();
  return { choice, retained, userScopeOnly };
}, "renamed-native-policy-probe");
assert.equal(toggled.choice, !installation.installation.enabled);
assert.equal(toggled.retained, toggled.choice, "Rendering after rename discarded the installation enabled draft");
assert.ok(toggled.userScopeOnly, "An installation policy offered unsupported project scope controls");
const installationChange = evaluate(() => window.smTestRequest, null);
assert.ok(installationChange);
assert.deepEqual(installationChange.renames, { [installation.id]: "renamed-native-policy-probe" });
assert.deepEqual(installationChange.resource_ids, [installation.id, "renamed-native-policy-probe"]);
const installationCatalog: EditorData["catalog"] = JSON.parse(installationChange.writes["catalog.json"]!);
const renamedInstallation = installationCatalog.resources.find(resource => resource.id === "renamed-native-policy-probe");
assert.ok(renamedInstallation && "installation" in renamedInstallation);
assert.equal(renamedInstallation.installation.enabled, toggled.choice);
assert.equal(renamedInstallation.scope, "user");
assert.equal(renamedInstallation.project, null);
for (const resource of resources.resources) if (resource.id !== installation.id) assert.deepEqual(installationCatalog.resources.find(next => next.id === resource.id), resource);
browser<null>({ type: "navigate", url: base + "/resources" });
console.log("Browser editor regressions passed: seven types, creation, rename, individual filter/edit/retire, complete Hook and Skill editing, supporting file reference retention and last-consumer retirement, shared source isolation and native policy toggle/rename; all submissions intercepted.");
