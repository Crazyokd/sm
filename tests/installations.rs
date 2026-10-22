use serde_json::{Value, json};
use sm::{installations, model::ApplyStatus, state::State};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

struct Fixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    home: PathBuf,
    state: State,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let home = temp.path().join("home");
        fs::create_dir_all(source.join("resources/native-installations")).unwrap();
        fs::create_dir_all(home.join(".codex/cache/sample")).unwrap();
        fs::write(home.join(".codex/config.toml"), "# keep this comment\nlocal_secret = 'private-key-value'\n[marketplaces.market]\nsource_type = 'git'\nsource = 'https://example.invalid/plugins'\n[plugins.'sample@market']\nenabled = true\n[plugins.'unregistered@market']\nenabled = false\n").unwrap();
        let state = State::open(temp.path().join("state/state.sqlite3")).unwrap();
        Self {
            temp,
            source,
            home,
            state,
        }
    }
    fn executable(&self, body: &str) -> PathBuf {
        let path = self.temp.path().join("installer");
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn declaration(&self, executable: &Path) -> Value {
        json!({"id":"codex-sample","agent":"codex","source":"https://example.invalid/plugins","selector":"sample@market","enabled":false,"paths":["~/.codex/cache/sample"],"update":[[executable]]})
    }
    fn manifest(&self, declarations: Value) {
        fs::write(
            self.source
                .join("resources/native-installations/installations.json"),
            serde_json::to_vec(&declarations).unwrap(),
        )
        .unwrap();
    }
    fn synchronize(&self) -> std::collections::BTreeMap<String, sm::model::ResourceResult> {
        installations::synchronize(&self.source, &self.home, &self.state).unwrap()
    }
    fn receipts(&self) -> Vec<Value> {
        self.state.get("external_installations").unwrap()
    }
}

#[test]
fn approved_argv_is_literal_and_restores_disabled_without_changing_other_config() {
    let fixture = Fixture::new();
    let executable = fixture.executable("test \"$SM_INTERNAL_RUN\" = 1\ntest \"$1\" = 'literal $(touch ignored)'\nprintf '%s' \"$SM_INSTALL_HOME\" > \"$SM_INSTALL_HOME/invoked\"");
    let mut declaration = fixture.declaration(&executable);
    declaration["update"][0]
        .as_array_mut()
        .unwrap()
        .push("literal $(touch ignored)".into());
    fixture.manifest(json!([declaration]));
    let result = fixture.synchronize();
    assert_eq!(result["codex-sample"].status, ApplyStatus::Applied);
    assert_eq!(
        fs::read_to_string(fixture.home.join("invoked")).unwrap(),
        fixture.home.to_str().unwrap()
    );
    assert!(!fixture.source.join("ignored").exists());
    let config = fs::read_to_string(fixture.home.join(".codex/config.toml")).unwrap();
    assert!(config.contains("# keep this comment") && config.contains("private-key-value"));
    assert!(config.contains("[plugins.'unregistered@market']\nenabled = false"));
    let parsed: toml_edit::DocumentMut = config.parse().unwrap();
    assert_eq!(
        parsed["plugins"]["sample@market"]["enabled"].as_bool(),
        Some(false)
    );
    assert_eq!(
        fixture.receipts(),
        vec![json!({"id":"codex-sample","paths":[fixture.home.join(".codex/cache/sample")]})]
    );
}

#[test]
fn reviewed_enabled_value_is_the_only_source_for_native_toggle_arguments() {
    let fixture = Fixture::new();
    let executable = fixture.executable("test \"$1\" = false");
    let mut declaration = fixture.declaration(&executable);
    declaration["update"][0]
        .as_array_mut()
        .unwrap()
        .push("${SM_ENABLED}".into());
    fixture.manifest(json!([declaration]));
    assert_eq!(
        fixture.synchronize()["codex-sample"].status,
        ApplyStatus::Applied
    );
}

#[test]
fn existing_native_registration_is_governed_without_running_a_network_installer() {
    let fixture = Fixture::new();
    let executable = fixture.executable("touch invoked\nexit 1");
    let mut declaration = fixture.declaration(&executable);
    declaration["enabled"] = true.into();
    fixture.manifest(json!([declaration]));
    installations::register_existing(&fixture.source, &fixture.home, &fixture.state).unwrap();
    assert_eq!(fixture.receipts().len(), 1);
    assert!(!fixture.source.join("invoked").exists());
}

#[test]
fn existing_native_registration_with_another_upstream_is_not_claimed() {
    let fixture = Fixture::new();
    let executable = fixture.executable("exit 0");
    let mut declaration = fixture.declaration(&executable);
    declaration["enabled"] = true.into();
    declaration["source"] = "https://example.invalid/other".into();
    fixture.manifest(json!([declaration]));
    installations::register_existing(&fixture.source, &fixture.home, &fixture.state).unwrap();
    assert!(fixture.receipts().is_empty());
    assert_eq!(
        fixture.synchronize()["codex-sample"].status,
        ApplyStatus::Failed
    );
}

#[test]
fn failed_installer_restores_disabled_and_never_claims_new_ownership_or_exports_output() {
    let fixture = Fixture::new();
    let executable = fixture.executable("printf '%s\\n' 'stdout-private-key-value'\nprintf '%s\\n' 'stderr-private-key-value' >&2\nexit 1");
    fixture.manifest(json!([fixture.declaration(&executable)]));
    let result = fixture.synchronize();
    assert_eq!(result["codex-sample"].status, ApplyStatus::Failed);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("private-key-value")
    );
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains(executable.to_str().unwrap())
    );
    assert!(fixture.receipts().is_empty());
    let config: toml_edit::DocumentMut =
        fs::read_to_string(fixture.home.join(".codex/config.toml"))
            .unwrap()
            .parse()
            .unwrap();
    assert_eq!(
        config["plugins"]["sample@market"]["enabled"].as_bool(),
        Some(false)
    );
}

#[test]
fn transient_update_failure_retains_only_previously_verified_coverage() {
    let fixture = Fixture::new();
    let executable = fixture.executable("exit 0");
    fixture.manifest(json!([fixture.declaration(&executable)]));
    assert_eq!(
        fixture.synchronize()["codex-sample"].status,
        ApplyStatus::Applied
    );
    fixture.executable("exit 1");
    assert_eq!(
        fixture.synchronize()["codex-sample"].status,
        ApplyStatus::Failed
    );
    assert_eq!(fixture.receipts().len(), 1);
    fs::remove_dir(fixture.home.join(".codex/cache/sample")).unwrap();
    assert_eq!(
        fixture.synchronize()["codex-sample"].status,
        ApplyStatus::Failed
    );
    assert!(fixture.receipts().is_empty());
}

#[test]
fn successful_command_without_registration_or_payload_is_not_managed() {
    let fixture = Fixture::new();
    let executable = fixture.executable("exit 0");
    let mut declaration = fixture.declaration(&executable);
    declaration["selector"] = "missing@market".into();
    fixture.manifest(json!([declaration]));
    assert_eq!(
        fixture.synchronize()["codex-sample"].status,
        ApplyStatus::Failed
    );
    assert!(fixture.receipts().is_empty());
}

#[test]
fn all_targets_are_validated_before_any_installer_runs() {
    let fixture = Fixture::new();
    let executable = fixture.executable("touch \"$SM_INSTALL_HOME/invoked\"");
    for path in [
        "/tmp/outside-home",
        "~/.codex/../escape",
        "relative/path",
        "~",
    ] {
        fixture.manifest(json!([fixture.declaration(&executable), {"id":"bad","selector":"bad@market", "paths":[path], "agent":"codex","source":"upstream","enabled":true,"update":[[executable]]}]));
        assert!(
            installations::synchronize(&fixture.source, &fixture.home, &fixture.state).is_err()
        );
        assert!(!fixture.home.join("invoked").exists());
    }
    symlink(fixture.temp.path(), fixture.home.join("linked")).unwrap();
    let mut declaration = fixture.declaration(&executable);
    declaration["paths"] = json!(["~/linked"]);
    fixture.manifest(json!([declaration]));
    assert!(installations::synchronize(&fixture.source, &fixture.home, &fixture.state).is_err());
}

#[test]
fn manifest_is_strict_and_removed_declarations_do_not_remove_native_plugins() {
    let fixture = Fixture::new();
    let executable = fixture.executable("exit 0");
    let mut declaration = fixture.declaration(&executable);
    declaration["unexpected"] = true.into();
    fixture.manifest(json!([declaration]));
    assert!(installations::load(&fixture.source).is_err());
    let declaration = fixture.declaration(&executable);
    fixture.manifest(json!([declaration.clone(), declaration]));
    assert!(installations::load(&fixture.source).is_err());
    fixture.manifest(json!([]));
    assert!(fixture.synchronize().is_empty());
    assert!(fixture.home.join(".codex/cache/sample").is_dir());
    assert!(
        fs::read_to_string(fixture.home.join(".codex/config.toml"))
            .unwrap()
            .contains("unregistered@market")
    );
}

#[test]
fn kimi_native_registry_is_verified_and_failed_update_restores_only_selected_enabled() {
    let fixture = Fixture::new();
    let root = fixture.home.join(".kimi-code/plugins/managed/sample");
    fs::create_dir_all(&root).unwrap();
    let path = fixture.home.join(".kimi-code/plugins/installed.json");
    let registry = json!({"version":1,"local_secret":"private-key-value","plugins":[{"id":"sample","root":root,"enabled":false,"state":"ok","originalSource":"https://example.invalid/plugin","other":"preserved"},{"id":"unregistered","enabled":true}]});
    fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
    let executable = fixture.executable("exit 0");
    let declaration = json!({"id":"kimi-sample","agent":"kimi","source":"https://example.invalid/plugin","selector":"sample","enabled":false,"paths":["~/.kimi-code/plugins/managed/sample"],"update":[[executable]]});
    fixture.manifest(json!([declaration]));
    assert_eq!(
        fixture.synchronize()["kimi-sample"].status,
        ApplyStatus::Applied
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
        registry
    );
    let mut changed = registry.clone();
    changed["plugins"][0]["enabled"] = true.into();
    fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
    fixture.executable("exit 1");
    assert_eq!(
        fixture.synchronize()["kimi-sample"].status,
        ApplyStatus::Failed
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
        registry
    );
    assert_eq!(fixture.receipts().len(), 1);
}

#[test]
fn kimi_adapter_uses_native_api_restores_disabled_and_keeps_server_tokens_private() {
    let fixture = Fixture::new();
    let bin = fixture.temp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let kimi_home = fixture.home.join(".kimi-code");
    fs::create_dir_all(&kimi_home).unwrap();
    fs::write(kimi_home.join("server.token"), "fixture-secret-token").unwrap();
    let fake = bin.join("kimi");
    fs::write(&fake, r##"#!/usr/bin/env node
const fs = require('node:fs');
const http = require('node:http');
let enabled = true;
const server = http.createServer((req, res) => {
  if (req.headers.authorization !== 'Bearer fixture-secret-token') process.exit(2);
  let body = '';
  req.on('data', part => body += part);
  req.on('end', () => {
    fs.appendFileSync(process.env.SM_NATIVE_TEST_RECORD, JSON.stringify({method:req.method,path:req.url,body}) + '\n');
    res.setHeader('Content-Type', 'application/json');
    if (req.method === 'POST' && req.url === '/api/v1/plugins') {
      enabled = true;
      res.end(JSON.stringify({code:process.env.SM_NATIVE_TEST_FAIL ? 1 : 0,data:{id:'sample'}}));
    } else if (req.method === 'POST' && req.url === '/api/v1/plugins/sample:disable') {
      enabled = false;
      res.end(JSON.stringify({code:0,data:{ok:true}}));
    } else if (req.method === 'GET' && req.url === '/api/v1/plugins') {
      res.end(JSON.stringify({code:0,data:{plugins:[{id:'sample',enabled,state:'ok'}]}}));
    } else process.exit(3);
  });
});
server.listen(0,'127.0.0.1', () => {
  fs.writeFileSync(process.env.SM_NATIVE_TEST_PID, String(process.pid));
  console.log(`Local: http://127.0.0.1:${server.address().port}/#token=fixture-secret-token`);
  console.error('private startup configuration');
});
"##).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    for failed in [false, true] {
        let record = fixture.temp.path().join(format!("requests-{failed}"));
        let pid = fixture.temp.path().join(format!("pid-{failed}"));
        let mut command = std::process::Command::new("node");
        command
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/source-providers/native-plugins.js"),
            )
            .args(["kimi", "https://example.invalid/sample", "sample", "false"])
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("KIMI_CODE_HOME", &kimi_home)
            .env_remove("SM_INSTALL_HOME")
            .env("SM_NATIVE_TEST_RECORD", &record)
            .env("SM_NATIVE_TEST_PID", &pid);
        if failed {
            command.env("SM_NATIVE_TEST_FAIL", "1");
        }
        let output = sm::process::run_command(command, None, std::time::Duration::from_secs(10));
        assert_eq!(output.is_err(), failed);
        let exported = match output {
            Ok(output) => {
                assert!(output.stdout.is_empty());
                String::from_utf8(output.stderr).unwrap()
            }
            Err(error) => error.to_string(),
        };
        assert!(
            !exported.contains("fixture-secret-token") && !exported.contains("private startup")
        );
        let requests: Vec<Value> = fs::read_to_string(record)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(requests[0]["path"], "/api/v1/plugins");
        assert_eq!(requests[1]["path"], "/api/v1/plugins/sample:disable");
        assert_eq!(requests.len(), if failed { 2 } else { 3 });
        let pid = fs::read_to_string(pid).unwrap();
        assert!(
            !Path::new("/proc").join(pid).exists(),
            "adapter must stop its own native server"
        );
    }
}

#[test]
fn codex_adapter_bootstraps_and_updates_only_the_declared_native_marketplace() {
    let fixture = Fixture::new();
    let bin = fixture.temp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let fake = bin.join("codex");
    fs::write(&fake, r##"#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.SM_NATIVE_TEST_RECORD, JSON.stringify({program:'codex',args}) + '\n');
if (args.join(' ') === 'plugin marketplace list --json') console.log(process.env.SM_NATIVE_TEST_MARKETS);
else console.log(JSON.stringify({ok:true}));
"##).unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    let fake_git = bin.join("git");
    fs::write(&fake_git, r##"#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.SM_NATIVE_TEST_RECORD, JSON.stringify({program:'git',args}) + '\n');
if (args.slice(2).join(' ') === 'remote get-url origin') console.log('https://example.invalid/plugins.git');
"##).unwrap();
    fs::set_permissions(&fake_git, fs::Permissions::from_mode(0o700)).unwrap();
    let native_local = tempfile::Builder::new()
        .prefix("sm-native-market-")
        .tempdir_in(std::env::var_os("HOME").unwrap())
        .unwrap();
    let source = "https://example.invalid/plugins.git";
    for (case, selector, marketplace, failed) in [
        ("missing", "sample@market", json!([]), false),
        (
            "git",
            "sample@market",
            json!([{"name":"market","marketplaceSource":{"sourceType":"git","source":source}}]),
            false,
        ),
        (
            "local",
            "sample@market",
            json!([{"name":"market","marketplaceSource":{"sourceType":"local","source":native_local.path()}}]),
            false,
        ),
        ("curated", "sample@openai-api-curated", json!([]), false),
        (
            "mismatch",
            "sample@market",
            json!([{"name":"market","marketplaceSource":{"sourceType":"git","source":"https://example.invalid/unregistered"}}]),
            true,
        ),
    ] {
        let record = fixture.temp.path().join(case);
        let mut command = std::process::Command::new("node");
        command
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/source-providers/native-plugins.js"),
            )
            .args(["codex", source, selector, "false"])
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env_remove("SM_INSTALL_HOME")
            .env("SM_NATIVE_TEST_RECORD", &record)
            .env(
                "SM_NATIVE_TEST_MARKETS",
                json!({"marketplaces":marketplace}).to_string(),
            );
        let output = sm::process::run_command(command, None, std::time::Duration::from_secs(10));
        assert_eq!(output.is_err(), failed, "{case}: {output:?}");
        let calls: Vec<Value> = fs::read_to_string(record)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let expected = match case {
            "missing" => json!([
                {"program":"codex","args":["plugin","marketplace","list","--json"]},
                {"program":"codex","args":["plugin","marketplace","add",source,"--json"]},
                {"program":"codex","args":["plugin","add",selector,"--json"]}
            ]),
            "git" => json!([
                {"program":"codex","args":["plugin","marketplace","list","--json"]},
                {"program":"codex","args":["plugin","marketplace","upgrade","market","--json"]},
                {"program":"codex","args":["plugin","add",selector,"--json"]}
            ]),
            "local" => json!([
                {"program":"codex","args":["plugin","marketplace","list","--json"]},
                {"program":"git","args":["-C",native_local.path(),"remote","get-url","origin"]},
                {"program":"git","args":["-C",native_local.path(),"pull","--ff-only"]},
                {"program":"codex","args":["plugin","add",selector,"--json"]}
            ]),
            "curated" => json!([{ "program":"codex","args":["plugin","add",selector,"--json"]}]),
            _ => json!([{ "program":"codex","args":["plugin","marketplace","list","--json"]}]),
        };
        assert_eq!(serde_json::to_value(calls).unwrap(), expected, "{case}");
    }
}
