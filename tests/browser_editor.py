"""Run with browser-harness; SM_BROWSER_URL must serve a disposable catalog with a self Skill.
All submissions are intercepted in the browser. No remote or live files are changed.
"""
import os
import time

base = os.environ.get("SM_BROWSER_URL", "http://127.0.0.1:8765")
new_tab(base + "/resources")
wait_for_load()
cdp('Network.enable')
cdp('Network.setCacheDisabled', cacheDisabled=True)
existing = js("JSON.parse(document.querySelector('.editor-data').textContent).catalog.resources.find(r=>r.kind==='skill'&&r.origin==='self').id")
failures = []


def prepare(path):
    previous = js("performance.timeOrigin")
    goto_url(base + path)
    deadline = time.monotonic() + 10
    while js("performance.timeOrigin") == previous and time.monotonic() < deadline:
        time.sleep(0.05)
    wait_for_load()
    js("window.smTestRequest=null;window.fetch=async(url,options)=>{window.smTestRequest=JSON.parse(options.body);return {ok:true,json:async()=>({message:'Intercepted test proposal'})}}")


prepare("/resources")
js("(()=>{const f=document.querySelector('.resource-editor');f.elements.resource_id.value=" + repr(existing) + ";f.elements.resource_id.dispatchEvent(new Event('change'));f.elements.reason.value='Duplicate ID test';f.requestSubmit()})()")
if js("window.smTestRequest!==null"):
    failures.append("Creation accepted an existing resource ID")

prepare("/resources")
js("(()=>{const f=document.querySelector('.resource-editor');f.elements.resource_id.value='draft-one';f.elements.resource_id.dispatchEvent(new Event('change'));f.querySelector('.payload-fields textarea').value+='\\nPRESERVE_EDITED_DRAFT\\n';f.elements.resource_id.value='draft-two';f.elements.resource_id.dispatchEvent(new Event('change'))})()")
if not js("document.querySelector('.payload-fields textarea').value.includes('PRESERVE_EDITED_DRAFT')"):
    failures.append("Correcting the name discarded edited content")

prepare("/resources/" + existing)
readonly = js("document.querySelector('[name=resource_id]').readOnly")
js("(()=>{const f=document.querySelector('.resource-editor');f.elements.resource_id.value='renamed-probe';f.elements.resource_id.dispatchEvent(new Event('change'));f.elements.reason.value='Rename test';f.requestSubmit()})()")
body = js("window.smTestRequest")
if readonly or not body or body.get("renames") != {existing: "renamed-probe"}:
    failures.append("Existing resource has no complete rename proposal")

print({"editor_regressions": "PASS" if not failures else "FAIL", "failures": failures})
assert not failures, failures
