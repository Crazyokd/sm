use anyhow::Result;
use serde_json::{Value, json};
use sm::{
    analysis::{
        AgentRunner, AnalysisResponse, Candidate, CliRunner, analyze_observations, content_hash,
        to_changeset,
    },
    evaluation::evaluate_candidate,
    files::copy_tree,
    model::{Catalog, Client, ValidationStatus},
    observe::Observation,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tempfile::{TempDir, tempdir};
struct Runner {
    reply: AnalysisResponse,
    calls: AtomicUsize,
    prompt: Mutex<String>,
}
impl Runner {
    fn new(reply: Value) -> Self {
        Self {
            reply: serde_json::from_value(reply).unwrap(),
            calls: AtomicUsize::new(0),
            prompt: Mutex::new(String::new()),
        }
    }
}
impl AgentRunner for Runner {
    fn run(&self, prompt: &str) -> Result<AnalysisResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.prompt.lock().unwrap() = prompt.into();
        Ok(self.reply.clone())
    }
}
const BEFORE: &str = "---\nname: review\ndescription: Review code\n---\nRead config\nRead config again\nResolve entry\nResolve entry again\nVerify result\n";
const AFTER: &str =
    "---\nname: review\ndescription: Review code\n---\nRead config\nResolve entry\nVerify result\n";

#[test]
fn individual_configuration_scope_and_source_file_modes_change_the_evidence_hash() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    let definition = json!({"id":"config","kind":"mcp","origin":"self","scope":"user","project":null,"files":{"root":"resources/config","entries":[{"target":".mcp.json","fragment":"config.json"}]}});
    let mut resource: sm::model::Resource = serde_json::from_value(definition).unwrap();
    resource.root = root.path().to_path_buf();
    fs::create_dir_all(root.path().join("resources/config")).unwrap();
    let config = root.path().join("resources/config/config.json");
    fs::write(
        &config,
        json!({"mcpServers":{"demo":{"command":"before"}}}).to_string(),
    )
    .unwrap();
    let before = content_hash(&resource).unwrap();
    fs::write(
        &config,
        json!({"mcpServers":{"demo":{"command":"after"}}}).to_string(),
    )
    .unwrap();
    assert_ne!(content_hash(&resource).unwrap(), before);
    let before = content_hash(&resource).unwrap();
    resource.scope = sm::model::Scope::Project;
    resource.project = Some("app".into());
    assert_ne!(content_hash(&resource).unwrap(), before);
    let before = content_hash(&resource).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
    assert_ne!(content_hash(&resource).unwrap(), before);
}

#[test]
fn configuration_fragment_can_be_optimized_without_changing_another_resource() {
    let (root, _, mut observation, mut reply) = fixture();
    for id in ["config", "other"] {
        fs::create_dir_all(root.path().join("resources").join(id)).unwrap();
        fs::write(
            root.path().join("resources").join(id).join("config.json"),
            json!({"mcpServers":{id:{"command":"before"}}}).to_string(),
        )
        .unwrap();
    }
    let resources:Vec<_>=["config","other"].into_iter().map(|id|json!({"id":id,"kind":"mcp","origin":"self","scope":"user","project":null,"files":{"root":format!("resources/{id}"),"entries":[{"target":".mcp.json","fragment":"config.json"}]}})).collect();
    fs::write(
        root.path().join("catalog.json"),
        json!({"projects":{},"resources":resources}).to_string(),
    )
    .unwrap();
    let catalog = Catalog::load(root.path()).unwrap();
    observation.resource_id = Some("config".into());
    reply["candidates"][0]["resource_id"] = json!("config");
    let after = json!({"mcpServers":{"config":{"command":"after"}}}).to_string();
    reply["candidates"][0]["changes"] =
        json!([{"path":"resources/config/config.json","content":after}]);
    let runner = Runner::new(reply);
    let candidate = analyze_observations(&catalog, &[observation], &runner)
        .unwrap()
        .remove(0);
    assert!(
        runner
            .prompt
            .lock()
            .unwrap()
            .contains("resources/config/config.json")
    );
    let proposed = stage(&root, &candidate);
    let report = evaluate_candidate(&candidate, root.path(), &proposed).unwrap();
    assert_eq!(report.status, ValidationStatus::Partial);
    let change = to_changeset(&candidate, &report).unwrap();
    assert_eq!(change.resource_ids, vec!["config"]);
    assert!(change.writes.contains_key("resources/config/config.json"));
    assert_eq!(
        fs::read(proposed.join("resources/other/config.json")).unwrap(),
        fs::read(root.path().join("resources/other/config.json")).unwrap()
    );
    assert_ne!(
        fs::read(proposed.join("resources/config/config.json")).unwrap(),
        fs::read(root.path().join("resources/config/config.json")).unwrap()
    );
}
fn fixture() -> (TempDir, Catalog, Observation, Value) {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("resources/review")).unwrap();
    fs::write(root.path().join("resources/review/SKILL.md"), BEFORE).unwrap();
    fs::write(root.path().join("resources/review/evals.json"),json!({"required":["Verify result"],"forbidden":["Read config again","Resolve entry again"]}).to_string()).unwrap();
    fs::write(root.path().join("catalog.json"),json!({"projects":{"app":"~/repo/app"},"resources":[{"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"}]}).to_string()).unwrap();
    let catalog = Catalog::load(root.path()).unwrap();
    let observation=serde_json::from_value(json!({"id":"event-1","agent":"codex","session_id":"one-session","cwd":"/repo","event":"PostToolUse","timestamp":"2026-10-21T00:00:00Z","resource_id":"review","resource_content_hash":null,"evidence_kind":"tool_execution","signal":"tool_execution","detected_from":"explicit","confidence":"medium","turn_id":null,"invocation_id":null,"parent_invocation_id":null,"facts":{"tool_input":{"command":"Read config twice; resolve entry twice"}},"coverage":"observed"})).unwrap();
    let reply = json!({"candidates":[{"resource_id":"review","issue":"Two redundant steps","evidence":["event-1"],"changes":[{"path":"resources/review/SKILL.md","content":AFTER}],"expected_checks":["Keep verification"]}]});
    (root, catalog, observation, reply)
}
fn stage(root: &TempDir, candidate: &Candidate) -> PathBuf {
    let proposed = root.path().join("proposed");
    fs::create_dir_all(&proposed).unwrap();
    fs::copy(
        root.path().join("catalog.json"),
        proposed.join("catalog.json"),
    )
    .unwrap();
    copy_tree(&root.path().join("resources"), &proposed.join("resources")).unwrap();
    for (path, text) in &candidate.changes {
        let path = proposed.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    proposed
}
#[test]
fn one_use_can_remove_two_redundant_steps_with_comparison_and_partial_validation() {
    let (root, catalog, observation, reply) = fixture();
    let runner = Runner::new(reply);
    let candidate = analyze_observations(&catalog, &[observation], &runner)
        .unwrap()
        .remove(0);
    let proposed = stage(&root, &candidate);
    let report = evaluate_candidate(&candidate, root.path(), &proposed).unwrap();
    assert_eq!(report.status, ValidationStatus::Partial);
    assert!(
        report
            .checks
            .iter()
            .any(|c| c.name == "forbidden:Read config again" && c.before == Some(false) && c.after)
    );
    assert!(!report.unverified.is_empty());
    let change = to_changeset(&candidate, &report).unwrap();
    assert!(change.evidence[0].contains("Read config twice"));
    assert_eq!(
        fs::read_to_string(root.path().join("resources/review/SKILL.md")).unwrap(),
        BEFORE
    );
}
#[test]
fn removal_of_required_verification_fails_even_if_candidate_weakens_evals() {
    let (root, catalog, observation, mut reply) = fixture();
    reply["candidates"][0]["changes"] = json!([{"path":"resources/review/SKILL.md","content":AFTER.replace("Verify result\n","")},{"path":"resources/review/evals.json","content":"{\"required\":[],\"forbidden\":[]}"}]);
    let candidate = analyze_observations(&catalog, &[observation], &Runner::new(reply))
        .unwrap()
        .remove(0);
    assert_eq!(
        evaluate_candidate(&candidate, root.path(), &stage(&root, &candidate))
            .unwrap()
            .status,
        ValidationStatus::Failed
    );
}
#[test]
fn missing_evidence_unknown_resource_and_outside_paths_are_rejected() {
    for mutation in ["missing", "invented", "outside", "unknown"] {
        let (_root, catalog, observation, mut reply) = fixture();
        match mutation {
            "missing" => reply["candidates"][0]["evidence"] = json!([]),
            "invented" => reply["candidates"][0]["evidence"] = json!(["fake"]),
            "outside" => reply["candidates"][0]["changes"][0]["path"] = json!("../outside"),
            "unknown" => reply["candidates"][0]["resource_id"] = json!("other"),
            _ => unreachable!(),
        }
        assert!(
            analyze_observations(&catalog, &[observation], &Runner::new(reply)).is_err(),
            "{mutation}"
        );
    }
}
#[test]
fn omitted_or_stale_source_is_not_rewritten() {
    let (root, catalog, mut observation, reply) = fixture();
    fs::write(
        root.path().join("resources/review/SKILL.md"),
        "x".repeat(20000),
    )
    .unwrap();
    assert!(
        analyze_observations(
            &catalog,
            &[observation.clone()],
            &Runner::new(reply.clone())
        )
        .is_err()
    );
    observation.resource_content_hash = Some("old-content".into());
    assert!(
        analyze_observations(&catalog, &[observation], &Runner::new(reply))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn no_records_or_only_upstream_never_calls_the_model() {
    let (_root, mut catalog, observation, _) = fixture();
    let runner = Runner::new(json!({"candidates":[]}));
    assert!(
        analyze_observations(&catalog, &[], &runner)
            .unwrap()
            .is_empty()
    );
    catalog.resources[0].source = sm::model::Source::Upstream {
        upstream: "external".into(),
    };
    assert!(
        analyze_observations(&catalog, &[observation], &runner)
            .unwrap()
            .is_empty()
    );
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}
#[test]
fn full_supplied_content_is_retained_but_sensitive_sources_are_omitted() {
    let (root, catalog, observation, _) = fixture();
    fs::write(
        root.path().join("resources/review/SKILL.md"),
        format!("{}SOURCE_TAIL", "x".repeat(8000)),
    )
    .unwrap();
    fs::write(
        root.path().join("resources/review/private.txt"),
        "OPENAI_API_KEY=source-secret",
    )
    .unwrap();
    let runner = Runner::new(json!({"candidates":[]}));
    analyze_observations(&catalog, &[observation], &runner).unwrap();
    let prompt = runner.prompt.lock().unwrap();
    assert!(prompt.contains("SOURCE_TAIL"));
    assert!(!prompt.contains("source-secret"));
}

#[test]
fn serialized_analysis_batches_keep_all_evidence_and_source_within_the_input_limit() {
    struct BoundedRunner(Mutex<Vec<Value>>);
    impl AgentRunner for BoundedRunner {
        fn run(&self, prompt: &str) -> Result<AnalysisResponse> {
            assert!(prompt.len() <= 512_000, "{} prompt bytes", prompt.len());
            let input: Value = serde_json::from_str(prompt.lines().last().unwrap())?;
            assert_eq!(
                input["resources"][0]["files"]["resources/review/SKILL.md"],
                BEFORE
            );
            self.0.lock().unwrap().push(input);
            Ok(AnalysisResponse {
                candidates: Vec::new(),
            })
        }
    }
    let (_root, mut catalog, observation, _) = fixture();
    for index in 0..292 {
        let mut resource = catalog.resources[0].clone();
        resource.id = format!("resource-{index}");
        catalog.resources.push(resource);
    }
    let observations: Vec<_> = (0..1000)
        .map(|index| {
            let mut row = observation.clone();
            row.id = format!("event-{index}");
            row.resource_id = None;
            row.facts.insert(
                "text".into(),
                json!(format!("{}tail-{index}", "\"\\\n".repeat(400))),
            );
            row
        })
        .collect();
    let runner = BoundedRunner(Mutex::new(Vec::new()));
    assert!(
        analyze_observations(&catalog, &observations, &runner)
            .unwrap()
            .is_empty()
    );
    let inputs = runner.0.lock().unwrap();
    assert!(inputs.len() > 1);
    let ids: Vec<_> = inputs
        .iter()
        .flat_map(|input| input["observations"].as_array().unwrap())
        .map(|row| {
            let id = row["id"].as_str().unwrap();
            assert!(
                row["facts"]["text"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!("tail-{}", id.strip_prefix("event-").unwrap()))
            );
            id.to_owned()
        })
        .collect();
    assert_eq!(
        ids,
        observations
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn oversized_single_evidence_is_not_silently_truncated_or_sent() {
    let (_root, catalog, mut observation, _) = fixture();
    observation
        .facts
        .insert("text".into(), json!("x".repeat(1_100_000)));
    let runner = Runner::new(json!({"candidates":[]}));
    let error = analyze_observations(&catalog, &[observation], &runner).unwrap_err();
    assert!(format!("{error:#}").contains("event-1"));
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn resource_and_project_context_is_also_part_of_the_serialized_limit() {
    let (_root, mut catalog, observation, _) = fixture();
    catalog.projects.insert("large".into(), "x".repeat(600_000));
    let runner = Runner::new(json!({"candidates":[]}));
    let error = analyze_observations(&catalog, &[observation], &runner).unwrap_err();
    assert!(format!("{error:#}").contains("resource context exceeds"));
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn each_batch_keeps_its_candidates_and_requires_evidence_from_that_batch() {
    struct EvidenceRunner {
        reply: AnalysisResponse,
        reuse_first_evidence: bool,
    }
    impl AgentRunner for EvidenceRunner {
        fn run(&self, prompt: &str) -> Result<AnalysisResponse> {
            let input: Value = serde_json::from_str(prompt.lines().last().unwrap())?;
            let mut reply = self.reply.clone();
            if !self.reuse_first_evidence {
                reply.candidates[0].evidence =
                    vec![input["observations"][0]["id"].as_str().unwrap().to_owned()];
            }
            Ok(reply)
        }
    }
    let (_root, catalog, observation, reply) = fixture();
    let observations: Vec<_> = (0..3)
        .map(|index| {
            let mut row = observation.clone();
            row.id = format!("event-{}", index + 1);
            row.facts.insert("text".into(), json!("x".repeat(240_000)));
            row
        })
        .collect();
    let runner = EvidenceRunner {
        reply: serde_json::from_value(reply).unwrap(),
        reuse_first_evidence: false,
    };
    let candidates = analyze_observations(&catalog, &observations, &runner).unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].evidence, vec!["event-1"]);
    assert_eq!(candidates[1].evidence, vec!["event-3"]);
    let runner = EvidenceRunner {
        reuse_first_evidence: true,
        ..runner
    };
    let error = analyze_observations(&catalog, &observations, &runner).unwrap_err();
    assert!(format!("{error:#}").contains("real evidence"));
}
#[test]
fn retirement_and_scope_changes_are_catalog_proposals_only() {
    for operation in ["retire", "reclassify"] {
        let (root, catalog, observation, mut reply) = fixture();
        reply["candidates"][0]["operation"] = json!(operation);
        reply["candidates"][0]["changes"] = json!([]);
        if operation == "reclassify" {
            reply["candidates"][0]["scope"] = json!("project");
            reply["candidates"][0]["project"] = json!("app");
        }
        let candidate = analyze_observations(&catalog, &[observation], &Runner::new(reply))
            .unwrap()
            .remove(0);
        let report =
            evaluate_candidate(&candidate, root.path(), &stage(&root, &candidate)).unwrap();
        let change = to_changeset(&candidate, &report).unwrap();
        let proposed: Value = serde_json::from_str(&change.writes["catalog.json"]).unwrap();
        if operation == "retire" {
            assert!(proposed["resources"].as_array().unwrap().is_empty());
        } else {
            assert_eq!(proposed["resources"][0]["scope"], "project");
        }
        assert!(root.path().join("resources/review/SKILL.md").exists());
    }
}
#[test]
fn python_and_shell_syntax_checks_use_native_parsing_without_execution() {
    let (root, catalog, observation, mut reply) = fixture();
    reply["candidates"][0]["changes"] = json!([{"path":"resources/review/check.py","content":"def broken(:\n pass\n"},{"path":"resources/review/check.sh","content":"if then\n"}]);
    let candidate = analyze_observations(&catalog, &[observation], &Runner::new(reply))
        .unwrap()
        .remove(0);
    let report = evaluate_candidate(&candidate, root.path(), &stage(&root, &candidate)).unwrap();
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(
        report
            .checks
            .iter()
            .filter(|c| c.kind == sm::model::CheckKind::Static)
            .all(|c| !c.after)
    );
}
#[test]
fn resource_hash_tracks_paths_and_content() {
    let (root, catalog, _, _) = fixture();
    let resource = &catalog.resources[0];
    let first = content_hash(resource).unwrap();
    assert_eq!(first, content_hash(resource).unwrap());
    let path = root.path().join("resources/review/SKILL.md");
    fs::write(&path, "Changed content").unwrap();
    let changed = content_hash(resource).unwrap();
    assert_ne!(first, changed);
    fs::rename(&path, path.with_file_name("RENAMED.md")).unwrap();
    assert_ne!(changed, content_hash(resource).unwrap());
}
#[test]
fn native_analysis_runners_have_no_business_tools_and_keep_large_input_off_argv() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempdir().unwrap();
    for client in [Client::Codex, Client::Kimi] {
        let file = root.path().join(client.to_string());
        let script = if client == Client::Codex {
            r#"#!/bin/sh
case " $* " in *' mcp list '*) printf '[{"name":"production"}]'; exit 0;; esac
out=''
while [ "$#" -gt 0 ]; do printf '%s\n' "$1" >> "__ARGS__"; if [ "$1" = '--output-last-message' ]; then shift; out=$1; fi; shift; done
cat >/dev/null
printf '{"candidates":[]}' > "$out"
"#
        } else {
            r#"#!/bin/sh
printf '%s\n' "$@" >> "__ARGS__"
while [ "$#" -gt 0 ]; do if [ "$1" = '--agent-file' ]; then shift; cp "$1" "__PROFILE__"; fi; shift; done
printf '{"message":{"content":[{"type":"text","text":"{\"candidates\":[]}"}]}}\n'
"#
        };
        let args = root.path().join(format!("{client}-args"));
        let profile = root.path().join("profile");
        fs::write(
            &file,
            script
                .replace("__ARGS__", args.to_str().unwrap())
                .replace("__PROFILE__", profile.to_str().unwrap()),
        )
        .unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
        let runner = CliRunner::new(client, file);
        assert!(
            runner
                .run(&format!(
                    "${{KIMI_WORK_DIR}}\n{}",
                    "evidence ".repeat(20000)
                ))
                .unwrap()
                .candidates
                .is_empty()
        );
        let args = fs::read_to_string(args).unwrap();
        if client == Client::Codex {
            assert!(
                args.contains("read-only")
                    && args.contains("mcp_servers.production.enabled=false")
                    && args.contains("hooks")
                    && args.contains("shell_tool")
            );
        } else {
            let profile = fs::read_to_string(profile).unwrap();
            assert!(profile.contains("tools: []") && profile.contains("subagents: []"));
            assert!(!profile.contains("${"));
            assert!(args.lines().all(|l| l.len() < 8192));
        }
    }
}
