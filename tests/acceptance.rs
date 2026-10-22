mod common;
use anyhow::Result;
use common::{FakeMrApi, Repo, git};
use serde_json::json;
use sm::{
    analysis::{AgentRunner, AnalysisResponse},
    model::{Catalog, Scope},
    observe::{pending_observations, record_event},
    proposals::{ChangeSet, create_proposal},
    service::{Runtime, Submitter, cycle_with, synchronize},
    state::State,
};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Submit(FakeMrApi);
impl Submitter for Submit {
    fn submit(&self, proposal: &mut sm::proposals::Proposal, state: &State) -> Result<String> {
        sm::gitlab::submit_mr_with(proposal, state, &self.0)
    }
}
fn runtime(repo: &Repo, name: &str) -> Runtime {
    let home = repo.temp.path().join(name);
    fs::create_dir_all(home.join("repo/app")).unwrap();
    Runtime {
        repo: repo.work.clone(),
        home,
        executable: env!("CARGO_BIN_EXE_sm").into(),
    }
}
fn project(repo: &Repo) {
    repo.catalog(json!({"projects":{"app":"~/repo/app"},"resources":[{"id":"review","kind":"skill","origin":"self","scope":"project","project":"app","payload":"resources/review"}]}));
    repo.publish("Project ownership");
}
#[test]
fn two_machine_restore_and_review_merge_gate_keep_project_scope() {
    let repo = Repo::new();
    project(&repo);
    let machines = [runtime(&repo, "one"), runtime(&repo, "two")];
    let relative = "repo/app/.agents/skills/review/SKILL.md";
    for runtime in &machines {
        assert!(
            synchronize(runtime, &runtime.state().unwrap())
                .unwrap()
                .ok()
        );
        assert_eq!(
            fs::read_to_string(runtime.home.join(relative)).unwrap(),
            "Approved"
        );
        assert!(!runtime.home.join(".agents/skills/review").exists());
    }
    let state = machines[0].state().unwrap();
    let change:ChangeSet=serde_json::from_value(json!({"resource_ids":["review"],"reason":"Reviewed update","evidence":["local-reference"],"writes":{"resources/review/SKILL.md":"Reviewed improvement"},"validation":{"status":"partial"}})).unwrap();
    let mut proposal = create_proposal(&repo.work, &change, &state).unwrap();
    sm::gitlab::submit_mr_with(&mut proposal, &state, &FakeMrApi::default()).unwrap();
    synchronize(&machines[0], &state).unwrap();
    assert_eq!(
        fs::read_to_string(machines[0].home.join(relative)).unwrap(),
        "Approved"
    );
    git(&repo.work, &["fetch", "origin", &proposal.branch]);
    git(&repo.work, &["merge", "--no-ff", "--no-edit", "FETCH_HEAD"]);
    git(&repo.work, &["push", "origin", "main"]);
    assert_eq!(
        fs::read_to_string(machines[0].home.join(relative)).unwrap(),
        "Approved"
    );
    for runtime in &machines {
        synchronize(runtime, &runtime.state().unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(runtime.home.join(relative)).unwrap(),
            "Reviewed improvement"
        );
    }
}
#[test]
fn approved_rename_reclassification_and_removal_clean_only_managed_targets() {
    let repo = Repo::new();
    project(&repo);
    let runtime = runtime(&repo, "home");
    let state = runtime.state().unwrap();
    synchronize(&runtime, &state).unwrap();
    let keep = runtime
        .home
        .join("repo/app/.agents/skills/personal/SKILL.md");
    fs::create_dir_all(keep.parent().unwrap()).unwrap();
    fs::write(&keep, "Unmanaged").unwrap();
    let mut catalog = Catalog::load(&repo.work).unwrap();
    git(
        &repo.work,
        &["mv", "resources/review", "resources/review-general"],
    );
    let entry = &mut catalog.resources[0];
    entry.id = "review-general".into();
    entry.scope = Scope::User;
    entry.project = None;
    entry.content = sm::model::ResourceContent::Skill {
        payload: PathBuf::from("resources/review-general"),
    };
    repo.catalog(serde_json::to_value(&catalog).unwrap());
    repo.publish("Approve reorganization");
    assert!(synchronize(&runtime, &state).unwrap().ok());
    assert!(
        !runtime
            .home
            .join("repo/app/.agents/skills/review/SKILL.md")
            .exists()
    );
    assert!(
        runtime
            .home
            .join(".agents/skills/review-general/SKILL.md")
            .exists()
    );
    catalog.resources.clear();
    repo.catalog(serde_json::to_value(catalog).unwrap());
    repo.publish("Retire resource");
    assert!(synchronize(&runtime, &state).unwrap().ok());
    assert!(
        !runtime
            .home
            .join(".agents/skills/review-general/SKILL.md")
            .exists()
    );
    assert_eq!(fs::read_to_string(keep).unwrap(), "Unmanaged");
}
struct NoModel(AtomicUsize);
impl AgentRunner for NoModel {
    fn run(&self, _: &str) -> Result<AnalysisResponse> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(AnalysisResponse {
            candidates: Vec::new(),
        })
    }
}
#[test]
fn upstream_sync_follows_new_commits_without_analysis_or_mr() {
    let repo = Repo::new();
    let upstream = Repo::new();
    repo.catalog(json!({"projects":{},"resources":[{"id":"external","kind":"skill","origin":"upstream","scope":"user","project":null,"payload":"resources/review","upstream":{"git":upstream.remote}}]}));
    repo.publish("Follow upstream");
    let runtime = runtime(&repo, "home");
    let state = runtime.state().unwrap();
    synchronize(&runtime, &state).unwrap();
    fs::write(
        upstream.work.join("resources/review/SKILL.md"),
        "New upstream",
    )
    .unwrap();
    upstream.publish("Upstream updated");
    record_event(
        sm::model::Client::Codex,
        &json!({"resource_id":"external","hook_event_name":"Stop"}),
        state.directory().unwrap(),
    )
    .unwrap();
    let runner = NoModel(AtomicUsize::new(0));
    let submit = Submit(FakeMrApi::default());
    let report = cycle_with(&runtime, &state, Some(&runner), &submit).unwrap();
    assert!(report.ok);
    assert_eq!(runner.0.load(Ordering::SeqCst), 0);
    assert!(submit.0.mrs.lock().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/external/SKILL.md")).unwrap(),
        "New upstream"
    );
}
#[test]
fn one_use_improvement_is_a_draft_and_does_not_change_live_resource() {
    let repo = Repo::new();
    let before = include_str!("fixtures/acceptance/SKILL.md");
    fs::write(repo.work.join("resources/review/SKILL.md"), before).unwrap();
    fs::write(
        repo.work.join("resources/review/evals.json"),
        include_str!("fixtures/acceptance/evals.json"),
    )
    .unwrap();
    repo.publish("Approved behavior checks");
    let runtime = runtime(&repo, "home");
    let state = runtime.state().unwrap();
    synchronize(&runtime, &state).unwrap();
    record_event(sm::model::Client::Codex,&json!({"resource_id":"review","session_id":"single-use","hook_event_name":"PostToolUse","tool_input":{"command":"Read configuration twice; resolve entry twice"}}),state.directory().unwrap()).unwrap();
    struct Runner(State);
    impl AgentRunner for Runner {
        fn run(&self, _: &str) -> Result<AnalysisResponse> {
            let evidence = pending_observations(&self.0, 1)?.remove(0).id;
            Ok(serde_json::from_value(
                json!({"candidates":[{"resource_id":"review","issue":"Remove two redundant steps","evidence":[evidence],"changes":[{"path":"resources/review/SKILL.md","content":include_str!("fixtures/acceptance/SKILL.md").replace("Repeat read configuration.\n","").replace("Repeat resolve entry point.\n","")}]}]}),
            )?)
        }
    }
    let submit = Submit(FakeMrApi::default());
    let report = cycle_with(&runtime, &state, Some(&Runner(state.clone())), &submit).unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!(report.proposals.len(), 1);
    assert!(submit.0.mrs.lock().unwrap()[0].title.starts_with("Draft:"));
    assert!(
        submit.0.mrs.lock().unwrap()[0]
            .description
            .contains("Read configuration twice")
    );
    assert_eq!(
        fs::read_to_string(runtime.home.join(".agents/skills/review/SKILL.md")).unwrap(),
        before
    );
    assert!(pending_observations(&state, 100).unwrap().is_empty());
    assert!(
        cycle_with(
            &runtime,
            &state,
            Some(&NoModel(AtomicUsize::new(0))),
            &submit
        )
        .unwrap()
        .proposals
        .is_empty()
    );
    let proposals: BTreeMap<String, sm::proposals::Proposal> = state.get("proposals").unwrap();
    assert_eq!(proposals.len(), 1);
}
