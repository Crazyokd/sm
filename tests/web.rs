mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{FakeMrApi, Repo};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sm::{
    model::{ApplyStatus, Resource, ResourceResult},
    observe::{ingest_events, record_event},
    proposals::Proposal,
    service::{Finding, Runtime, Submitter, synchronize},
    state::State,
};
use std::{collections::BTreeMap, fs, sync::Arc};
use tower::ServiceExt;
struct Submit(FakeMrApi);
impl Submitter for Submit {
    fn submit(&self, proposal: &mut Proposal, state: &State) -> anyhow::Result<String> {
        sm::gitlab::submit_mr_with(proposal, state, &self.0)
    }
}
struct Fixture {
    repo: Repo,
    runtime: Runtime,
    state: State,
    app: axum::Router,
}
impl Fixture {
    fn new() -> Self {
        let repo = Repo::new();
        fs::write(repo.work.join("resources/review/SKILL.md"),"---\nname: review\ndescription: Review\n---\n# Review\n<script>alert('xss')</script>\n[jump](javascript:alert%281%29)\n").unwrap();
        repo.publish("Readable skill");
        let runtime = Runtime {
            repo: repo.work.clone(),
            home: repo.temp.path().join("home"),
            executable: env!("CARGO_BIN_EXE_sm").into(),
        };
        let state = runtime.state().unwrap();
        synchronize(&runtime, &state).unwrap();
        let app = sm::web::router_with(
            runtime.clone(),
            state.clone(),
            Arc::new(Submit(FakeMrApi::default())),
        )
        .unwrap();
        Self {
            repo,
            runtime,
            state,
            app,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: String,
        headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("Host", "127.0.0.1:8765");
        for (k, v) in headers {
            request = request.header(*k, *v);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        (status, text)
    }
    async fn get(&self, path: &str) -> (StatusCode, String) {
        self.request("GET", path, String::new(), &[]).await
    }
    async fn token(&self) -> String {
        self.get("/resources")
            .await
            .1
            .split("name=\"sm-csrf\" content=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_owned()
    }
}
#[tokio::test]
async fn embedded_views_and_portable_skill_filter_are_readable() {
    let f = Fixture::new();
    for path in [
        "/resources",
        "/resources/review",
        "/projects",
        "/activity",
        "/jobs",
        "/static/app.js",
        "/static/style.css",
    ] {
        let (status, body) = f.get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
    }
    for client in ["codex", "kimi"] {
        let page = f
            .get(&format!("/resources?agent={client}&kind=skill&scope=user"))
            .await
            .1;
        let table = page
            .split("<tbody>")
            .nth(1)
            .unwrap()
            .split("</tbody>")
            .next()
            .unwrap();
        assert!(table.contains("review") && table.contains("通用 Skill"));
    }
    assert_eq!(f.get("/resources/unknown").await.0, StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn markdown_and_embedded_json_cannot_execute_scripts_or_unsafe_links() {
    let f = Fixture::new();
    fs::write(
        f.runtime.home.join(".codex/private.txt"),
        "NATIVE_SECRET_VALUE",
    )
    .unwrap();
    let page = f.get("/resources/review").await.1;
    assert!(page.contains("&lt;script&gt;"));
    assert!(!page.contains("<script>alert('xss')</script>"));
    assert!(!page.contains("href=\"javascript:"));
    assert!(!page.contains("NATIVE_SECRET_VALUE"));
    assert!(!page.contains("</script>alert"));
}
#[tokio::test]
async fn foreign_host_origin_and_missing_token_cannot_mutate() {
    let f = Fixture::new();
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/resources")
                .header("Host", "evil.invalid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    for headers in [
        vec![("Origin", "https://evil.invalid")],
        vec![("Origin", "http://127.0.0.1:8765")],
        vec![],
    ] {
        assert_eq!(
            f.request("POST", "/proposals", "{}".into(), &headers)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
}
#[tokio::test]
async fn malformed_body_and_paths_return_errors_without_escaping() {
    let f = Fixture::new();
    let token = f.token().await;
    let headers = [
        ("Origin", "http://127.0.0.1:8765"),
        ("X-SM-CSRF", token.as_str()),
        ("Content-Type", "application/json"),
    ];
    for body in ["[]".to_owned(),"{broken".to_owned(),json!({"resource_ids":["review"],"reason":"Invalid path","writes":{"../outside":"bad"},"deletions":[],"evidence":[],"validation":{}}).to_string()]{let(status,text)=f.request("POST","/proposals",body,&headers).await;assert_eq!(status,StatusCode::BAD_REQUEST,"{text}");assert!(serde_json::from_str::<Value>(&text).unwrap().get("error").is_some());}
    assert!(!f.repo.temp.path().join("outside").exists());
}
#[tokio::test]
async fn actual_body_limit_applies_without_content_length_header() {
    let f = Fixture::new();
    let token = f.token().await;
    let headers = [
        ("Origin", "http://127.0.0.1:8765"),
        ("X-SM-CSRF", token.as_str()),
        ("Content-Type", "application/json"),
    ];
    assert_eq!(
        f.request("POST", "/proposals", "x".repeat(1048577), &headers)
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
#[tokio::test]
async fn editing_only_creates_mr_and_keeps_live_content() {
    let f = Fixture::new();
    let token = f.token().await;
    let before = fs::read(f.runtime.home.join(".agents/skills/review/SKILL.md")).unwrap();
    let input = json!({"resource_ids":["review"],"reason":"Web edit","evidence":[],"writes":{"resources/review/SKILL.md":"Updated only in proposal"},"deletions":[],"validation":{"status":"partial","unverified":["manual review"]}});
    let (status, text) = f
        .request(
            "POST",
            "/proposals",
            input.to_string(),
            &[
                ("Origin", "http://127.0.0.1:8765"),
                ("X-SM-CSRF", &token),
                ("Content-Type", "application/json"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(text.contains("submitted") && text.contains("https://git.example.invalid/mr/1"));
    assert_eq!(
        before,
        fs::read(f.runtime.home.join(".agents/skills/review/SKILL.md")).unwrap()
    );
    let page = f.get("/resources/review").await.1;
    assert!(page.contains("打开 MR"));
}
#[tokio::test]
async fn five_types_statuses_and_project_groups_use_same_catalog() {
    let f = Fixture::new();
    let mut resources: Vec<Resource> = f.state.get("resources").unwrap();
    let kinds = ["hook", "rule", "agent", "mcp"];
    let statuses = [
        ApplyStatus::Unsupported,
        ApplyStatus::NeedsAgentTrust,
        ApplyStatus::Failed,
        ApplyStatus::Unknown,
    ];
    let mut results = BTreeMap::new();
    for (kind, status) in kinds.into_iter().zip(statuses) {
        resources.push(serde_json::from_value(json!({"id":kind,"kind":kind,"origin":"self","scope":"project","project":"app","agents":["codex"],"payloads":{"codex":format!("resources/{kind}")}})).unwrap());
        results.insert(kind, ResourceResult::new(status, "status evidence"));
    }
    f.state.set("resources", &resources).unwrap();
    f.state.set("resource_results", &results).unwrap();
    let mut catalog: sm::model::Catalog = f.state.get("catalog").unwrap();
    catalog.projects.insert("app".into(), "~/repo/app".into());
    catalog.resources = resources;
    f.state.set("catalog", &catalog).unwrap();
    let page = f.get("/resources").await.1;
    for label in [
        "Skill",
        "Hook",
        "规则",
        "Agent 定义",
        "MCP 配置",
        "暂不支持",
        "需原生信任",
        "应用失败",
        "尚未应用",
    ] {
        assert!(page.contains(label), "{label}");
    }
    let page = f.get("/projects").await.1;
    assert!(page.contains("用户级") && page.contains("app"));
}
#[tokio::test]
async fn observation_counts_are_not_reported_as_usage_frequency_or_retirement() {
    let f = Fixture::new();
    let empty = f.get("/activity").await.1;
    assert!(empty.contains("未观测") && !empty.contains("建议淘汰"));
    for _ in 0..2 {
        record_event(
            sm::model::Client::Codex,
            &json!({"resource_id":"review","session_id":"one","hook_event_name":"PostToolUse"}),
            f.state.directory().unwrap(),
        )
        .unwrap();
    }
    ingest_events(f.state.directory().unwrap(), &f.state).unwrap();
    let page = f.get("/activity").await.1;
    assert!(page.contains("2 条") && page.contains("1 个"));
    assert!(page.contains("不等于调用次数"));
}
#[tokio::test]
async fn findings_link_actual_evidence_and_related_mr() {
    let f = Fixture::new();
    record_event(
        sm::model::Client::Codex,
        &json!({"resource_id":"review","hook_event_name":"Stop"}),
        f.state.directory().unwrap(),
    )
    .unwrap();
    let event = ingest_events(f.state.directory().unwrap(), &f.state)
        .unwrap()
        .remove(0);
    let validation=serde_json::from_value(json!({"status":"partial","checks":[{"name":"required","before":true,"after":true}],"unverified":["No business replay"]})).unwrap();
    f.state
        .set(
            "findings",
            &vec![Finding {
                resource_id: "review".into(),
                issue: "Repeated reads".into(),
                evidence: vec![event.id.clone()],
                validation,
            }],
        )
        .unwrap();
    let token = f.token().await;
    let change = json!({"resource_ids":["review"],"reason":"Link MR","writes":{"resources/review/SKILL.md":"Draft"},"validation":{"status":"partial"}});
    assert_eq!(
        f.request(
            "POST",
            "/proposals",
            change.to_string(),
            &[
                ("Origin", "http://127.0.0.1:8765"),
                ("X-SM-CSRF", &token),
                ("Content-Type", "application/json")
            ]
        )
        .await
        .0,
        StatusCode::OK
    );
    let page = f.get("/activity").await.1;
    assert!(page.contains(&format!("href=\"#event-{}\"", event.id)));
    assert!(
        page.contains("href=\"https:")
            && page.contains("git.example.invalid")
            && page.contains("打开 MR")
    );
    assert!(page.contains("原有") && page.contains("修改后"));
}
