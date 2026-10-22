import type { ChangeSet, EditorData } from "../frontend/types.js";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";

declare global { interface Window { smTestRequest: ChangeSet | null } }
type BrowserCommand = { type: "open" | "navigate"; url: string } | { type: "evaluate"; code: string };
function browser<T>(command: BrowserCommand): T {
  const payload = Buffer.from(JSON.stringify(command), "utf-8").toString("base64");
  // Browser Harness owns the Python bridge; all regression logic below is type checked.
  const bridge = `import base64,json,time
request=json.loads(base64.b64decode("${payload}"))
if request['type']=='open':
    new_tab(request['url']);wait_for_load();cdp('Network.enable');cdp('Network.setCacheDisabled',cacheDisabled=True);result=None
elif request['type']=='navigate':
    previous=js('performance.timeOrigin');goto_url(request['url']);deadline=time.monotonic()+10
    while js('performance.timeOrigin')==previous and time.monotonic()<deadline: time.sleep(0.05)
    wait_for_load();result=None
else: result=js(request['code'])
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
function prepare(path: string): void {
  browser<null>({ type: "navigate", url: base + path });
  evaluate(() => {
    window.smTestRequest = null;
    window.fetch = async (_url: RequestInfo | URL, options?: RequestInit): Promise<Response> => {
      if (typeof options?.body !== "string") throw new Error("Expected a JSON proposal body");
      window.smTestRequest = JSON.parse(options.body) as ChangeSet;
      return new Response(JSON.stringify({ message: "Intercepted test proposal" }), { status: 200, headers: { "Content-Type": "application/json" } });
    };
  }, null);
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
browser<null>({ type: "navigate", url: base + "/resources" });
console.log("Browser editor regressions: 3/3 passed; all submissions intercepted.");
