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
fn fixture() -> (TempDir, Catalog, Observation, Value) {
    let root = tempdir().unwrap();
    fs::create_dir_all(root.path().join("resources/review")).unwrap();
    fs::write(root.path().join("resources/review/SKILL.md"), BEFORE).unwrap();
    fs::write(root.path().join("resources/review/evals.json"),json!({"required":["Verify result"],"forbidden":["Read config again","Resolve entry again"]}).to_string()).unwrap();
    fs::write(root.path().join("catalog.json"),json!({"projects":{"app":"~/repo/app"},"resources":[{"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"}]}).to_string()).unwrap();
    let catalog = Catalog::load(root.path()).unwrap();
    let observation=serde_json::from_value(json!({"id":"event-1","agent":"codex","session_id":"one-session","cwd":"/repo","event":"PostToolUse","timestamp":"2026-10-21","resource_id":"review","facts":{"tool_input":{"command":"Read config twice; resolve entry twice"}},"coverage":"observed","evidence_kind":"execution"})).unwrap();
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
        upstream: sm::model::UpstreamSource::Git {
            git: "https://example.invalid/upstream".into(),
        },
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
