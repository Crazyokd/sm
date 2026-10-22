mod common;
use common::{FakeMrApi, Repo, git};
use serde_json::{Value, json};
use sm::{
    gitlab::{project_location, submit_mr_with},
    model::{Source, UpstreamSource},
    process::run_command,
    proposals::{ChangeSet, NoChanges, Proposal, create_proposal},
    repository::{fetch_approved, fetch_upstream},
};
use std::{
    fs,
    process::Command,
    time::{Duration, Instant},
};

fn change() -> ChangeSet {
    serde_json::from_value(json!({"resource_ids":["review"],"reason":"Remove duplicate steps","evidence":["event-1"],"writes":{"resources/review/SKILL.md":"Improved"},"deletions":[],"validation":{"status":"partial","unverified":["behavior pending"]}})).unwrap()
}

#[test]
fn paths_and_credentials_are_rejected_at_proposal_boundary() {
    for path in [
        "../outside",
        "/absolute",
        ".git/config",
        "resources/other/file",
    ] {
        let mut input = change();
        input.writes.clear();
        input.writes.insert(path.into(), "bad".into());
        assert!(input.validate().is_err(), "{path}");
    }
    for secret in [
        "OPENAI_API_KEY=actual-sensitive-value",
        "api_key=\"actual-sensitive-value\"",
        "glpat-abcdefghijklmnop",
        "https://user:secret@example.invalid",
    ] {
        let mut input = change();
        input.evidence = vec![secret.into()];
        assert!(input.validate().is_err(), "{secret}");
    }
    let mut input = change();
    input.writes.insert(
        "resources/review/config.toml".into(),
        "api_key=\"$LOCAL_API_KEY\"".into(),
    );
    assert!(input.validate().is_ok());
}
#[test]
fn repeated_proposals_reuse_the_same_key_and_saved_draft() {
    let repo = Repo::new();
    let state = repo.state();
    let first = create_proposal(&repo.work, &change(), &state).unwrap();
    let restored: std::collections::BTreeMap<String, Proposal> = state.get("proposals").unwrap();
    assert_eq!(restored[&first.key].directory, first.directory);
    let second = create_proposal(&repo.work, &change(), &state).unwrap();
    assert_eq!(first.branch, second.branch);
    assert_eq!(
        fs::read_to_string(repo.work.join("resources/review/SKILL.md")).unwrap(),
        "Approved"
    );
}
#[test]
fn proposal_identity_ignores_resource_order_and_preserves_unicode_reasons() {
    let mut first = change();
    first.resource_ids = vec!["review".into(), "other".into()];
    first.reason = "减少重复步骤 🚀".into();
    let mut second = first.clone();
    second.resource_ids.reverse();
    assert_eq!(first.key().unwrap(), second.key().unwrap());
    second.reason.push_str("，保留校验");
    assert_ne!(first.key().unwrap(), second.key().unwrap());
}
#[test]
fn gitlab_nullable_description_is_accepted_at_the_external_boundary() {
    let mr:sm::gitlab::Mr=serde_json::from_value(json!({"iid":1,"web_url":"https://git.example.invalid/mr/1","source_branch":"sm/one","target_branch":"main","title":"Draft: update","description":null})).unwrap();
    assert!(mr.description.is_empty());
}
#[test]
fn approved_export_ignores_dirty_checkout_and_unmerged_proposals() {
    let repo = Repo::new();
    fs::write(repo.work.join("resources/review/SKILL.md"), "Unreviewed").unwrap();
    let output = repo.temp.path().join("approved");
    fetch_approved(&repo.work, &output).unwrap();
    assert_eq!(
        fs::read_to_string(output.join("resources/review/SKILL.md")).unwrap(),
        "Approved"
    );
    assert!(!output.join(".git").exists());
}
#[test]
fn invalid_catalog_does_not_poison_retry_or_previous_draft() {
    let repo = Repo::new();
    let state = repo.state();
    let mut bad = change();
    bad.writes.clear();
    let mut catalog: Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"][0]["scope"] = json!("system");
    bad.writes
        .insert("catalog.json".into(), catalog.to_string());
    assert!(create_proposal(&repo.work, &bad, &state).is_err());
    let first = create_proposal(&repo.work, &change(), &state).unwrap();
    assert!(create_proposal(&repo.work, &bad, &state).is_err());
    assert_eq!(
        fs::read_to_string(first.directory.join("resources/review/SKILL.md")).unwrap(),
        "Improved"
    );
}
#[test]
fn no_change_does_not_create_or_publish_empty_draft() {
    let repo = Repo::new();
    let state = repo.state();
    let mut same = change();
    same.writes
        .insert("resources/review/SKILL.md".into(), "Approved".into());
    let error = create_proposal(&repo.work, &same, &state).unwrap_err();
    assert!(error.is::<NoChanges>());
    assert!(
        state
            .get::<std::collections::BTreeMap<String, Proposal>>("proposals")
            .unwrap()
            .is_empty()
    );
}
#[test]
fn upstream_content_cannot_be_locally_modified() {
    let repo = Repo::new();
    let mut catalog: Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"][0]["origin"] = json!("upstream");
    catalog["resources"][0]["upstream"] = json!({"git":"https://example.invalid/upstream"});
    repo.catalog(catalog);
    repo.publish("Use upstream");
    assert!(create_proposal(&repo.work, &change(), &repo.state()).is_err());
}
#[test]
fn rename_preserves_binary_companions_and_live_source() {
    let repo = Repo::new();
    fs::write(
        repo.work.join("resources/review/asset.bin"),
        b"\0\xffpayload",
    )
    .unwrap();
    repo.publish("Companion");
    let mut catalog: Value =
        serde_json::from_slice(&fs::read(repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"][0]["id"] = json!("renamed");
    catalog["resources"][0]["payload"] = json!("resources/renamed");
    let mut input = change();
    input.resource_ids.push("renamed".into());
    input.renames.insert("review".into(), "renamed".into());
    input.writes.clear();
    input
        .writes
        .insert("catalog.json".into(), catalog.to_string());
    let proposal = create_proposal(&repo.work, &input, &repo.state()).unwrap();
    assert_eq!(
        fs::read(proposal.directory.join("resources/renamed/asset.bin")).unwrap(),
        b"\0\xffpayload"
    );
    assert!(!proposal.directory.join("resources/review").exists());
    assert!(repo.work.join("resources/review/asset.bin").exists());
}
#[test]
fn project_map_cannot_move_an_unselected_resource() {
    let repo = Repo::new();
    fs::create_dir(repo.work.join("resources/other")).unwrap();
    fs::write(repo.work.join("resources/other/SKILL.md"), "Other").unwrap();
    let mut catalog = json!({"projects":{"app":"~/repo/app"},"resources":[{"id":"review","kind":"skill","origin":"self","scope":"project","project":"app","payload":"resources/review"},{"id":"other","kind":"skill","origin":"self","scope":"project","project":"app","payload":"resources/other"}]});
    repo.catalog(catalog.clone());
    repo.publish("Shared project");
    catalog["projects"]["app"] = json!("~/repo/elsewhere");
    let mut input = change();
    input.writes.clear();
    input
        .writes
        .insert("catalog.json".into(), catalog.to_string());
    assert!(create_proposal(&repo.work, &input, &repo.state()).is_err());
}
#[test]
fn existing_mr_survives_main_advancement_and_refreshes_evidence() {
    let repo = Repo::new();
    let state = repo.state();
    let api = FakeMrApi::default();
    let mut input = change();
    input.validation.status = sm::model::ValidationStatus::Passed;
    let mut proposal = create_proposal(&repo.work, &input, &state).unwrap();
    let first = submit_mr_with(&mut proposal, &state, &api).unwrap();
    fs::write(repo.work.join("unrelated.txt"), "New main content").unwrap();
    repo.publish("Main advanced");
    input.evidence = vec!["New checked evidence".into()];
    input.validation.status = sm::model::ValidationStatus::Partial;
    input
        .writes
        .insert("resources/review/SKILL.md".into(), "Improved again".into());
    let mut proposal = create_proposal(&repo.work, &input, &state).unwrap();
    assert_eq!(submit_mr_with(&mut proposal, &state, &api).unwrap(), first);
    let mrs = api.mrs.lock().unwrap();
    assert_eq!(mrs.len(), 1);
    assert!(mrs[0].description.contains("New checked evidence"));
    assert!(mrs[0].title.starts_with("Draft:"));
    assert_eq!(
        git(
            &repo.remote,
            &["rev-parse", &format!("refs/heads/{}", proposal.branch)]
        ),
        git(&proposal.directory, &["rev-parse", "HEAD"])
    );
}
#[test]
fn failed_mr_submission_leaves_retriable_local_proposal() {
    let repo = Repo::new();
    let state = repo.state();
    let mut proposal = create_proposal(&repo.work, &change(), &state).unwrap();
    assert!(
        submit_mr_with(
            &mut proposal,
            &state,
            &FakeMrApi {
                offline: true,
                ..Default::default()
            }
        )
        .is_err()
    );
    let restored: std::collections::BTreeMap<String, Proposal> = state.get("proposals").unwrap();
    assert!(restored[&proposal.key].directory.exists());
    assert!(restored[&proposal.key].mr_url.is_none());
}
#[test]
fn upstream_git_follows_new_content_and_ignores_its_git_directory() {
    let repo = Repo::new();
    let entry = sm::model::Resource {
        id: "external".into(),
        source: Source::Upstream {
            upstream: UpstreamSource::Git {
                git: repo.remote.to_string_lossy().into(),
            },
        },
        ..sm::model::Catalog::load(&repo.work)
            .unwrap()
            .resources
            .remove(0)
    };
    let output = repo.temp.path().join("upstream");
    fetch_upstream(&entry, &output).unwrap();
    assert_eq!(
        fs::read_to_string(output.join("resources/review/SKILL.md")).unwrap(),
        "Approved"
    );
}
#[test]
fn subprocess_timeout_is_bounded_and_failure_is_visible() {
    let started = Instant::now();
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 5"]);
    assert!(run_command(command, None, Duration::from_millis(50)).is_err());
    assert!(started.elapsed() < Duration::from_secs(2));
    let mut command = Command::new("sh");
    command.args(["-c", "printf bad >&2; exit 3"]);
    assert!(run_command(command, None, Duration::from_secs(1)).is_err());
}
#[test]
fn subprocess_deadline_includes_pipes_inherited_after_parent_exit() {
    for input in [None, Some(vec![b'x'; 1024 * 1024])] {
        let mut command = Command::new("sh");
        command.args(["-c", "exec 3<&0; sleep 2 <&3 & exit 0"]);
        let started = Instant::now();
        let result = run_command(command, input.as_deref(), Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(1));
        let error = result.unwrap_err().to_string();
        assert!(error.contains("timed out"), "{error}");
    }
}
#[test]
fn subprocess_drains_large_output_while_writing_input() {
    let input = vec![b'x'; 1024 * 1024];
    let mut command = Command::new("sh");
    command.args(["-c", "printf diagnostic >&2; cat"]);
    let output = run_command(command, Some(&input), Duration::from_secs(5)).unwrap();
    assert_eq!(output.stdout, input);
    assert_eq!(output.stderr, b"diagnostic");
}
#[test]
fn gitlab_location_supports_http_and_scp_without_leaking_credentials() {
    let repo = Repo::new();
    git(
        &repo.work,
        &[
            "remote",
            "set-url",
            "origin",
            "http://git.example/group/sm.git",
        ],
    );
    assert_eq!(
        project_location(&repo.work).unwrap(),
        ("git.example".into(), "group/sm".into())
    );
    git(
        &repo.work,
        &[
            "remote",
            "set-url",
            "origin",
            "git@git.example:group/sm.git",
        ],
    );
    assert_eq!(
        project_location(&repo.work).unwrap(),
        ("git.example".into(), "group/sm".into())
    );
}
