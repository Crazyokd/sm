mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{FakeMrApi, Repo};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sm::{
    model::{ApplyStatus, ResourceResult},
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
#[tokio::test]
async fn inventory_page_detects_resources_added_since_last_sync_without_exposing_config() {
    let f = Fixture::new();
    let extra = f.runtime.home.join(".agents/skills/outside");
    fs::create_dir_all(&extra).unwrap();
    fs::write(extra.join("SKILL.md"), "Private resource body").unwrap();
    let (status, page) = f.get("/inventory").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("纳管覆盖") && page.contains("outside") && page.contains("未纳管"));
    assert!(!page.contains("Private resource body"));
    let (status, data) = f.get("/inventory.json").await;
    assert_eq!(status, StatusCode::OK);
    let data: Value = serde_json::from_str(&data).unwrap();
    assert!(data["entries"].as_array().unwrap().iter().any(|e| {
        e["path"].as_str().is_some_and(|p| p.ends_with("/outside"))
            && e["status"] == "unmanaged"
            && e["name"] == "outside"
    }));
    let (_, filtered) = f.get("/inventory?status=managed").await;
    assert!(!filtered.contains("<strong>outside</strong>"));
    fs::write(
        f.runtime.home.join(".codex/config.toml"),
        "[[skills.config]]\nname='cavecrew'\nenabled=false\n",
    )
    .unwrap();
    let (status, named) = f.get("/inventory?kind=skill-config&q=cavecrew").await;
    assert_eq!(status, StatusCode::OK);
    assert!(named.contains("<strong>cavecrew</strong>"));
    assert!(!named.contains("<strong>review</strong>"));
}

#[tokio::test]
async fn plugin_toggle_proposes_reviewed_declaration_without_mutating_native_installation() {
    let f = Fixture::new();
    let path = f.repo.work.join("resources/native-installations");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("bundle.json"), "{\"entries\":[]}").unwrap();
    let declaration = json!([{"id":"kimi-example","agent":"kimi","source":"https://example.invalid/plugin","selector":"example","enabled":true,"paths":["~/.kimi-code/plugins/managed/example"],"update":[["node","resources/source-providers/native-plugins.js","kimi","https://example.invalid/plugin","example","${SM_ENABLED}"]]}]);
    fs::write(path.join("installations.json"), declaration.to_string()).unwrap();
    let mut catalog: Value =
        serde_json::from_slice(&fs::read(f.repo.work.join("catalog.json")).unwrap()).unwrap();
    catalog["resources"].as_array_mut().unwrap().push(json!({"id":"native-installations","kind":"bundle","origin":"self","scope":"user","project":null,"payload":"resources/native-installations"}));
    f.repo.catalog(catalog);
    f.repo.publish("Register native installation declaration");
    f.state
        .set("catalog", &sm::model::Catalog::load(&f.repo.work).unwrap())
        .unwrap();
    f.state.set("installation_manifest", &declaration).unwrap();
    let token = f.token().await;
    let (status, body) = f
        .request(
            "POST",
            "/installations/kimi-example",
            json!({"enabled":false,"reason":"Disable the unused plugin after review"}).to_string(),
            &[
                ("Content-Type", "application/json"),
                ("X-SM-CSRF", &token),
                ("Origin", "http://127.0.0.1:8765"),
            ],
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("审核合并后"));
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(path.join("installations.json")).unwrap())
            .unwrap()[0]["enabled"],
        true
    );
    assert_eq!(
        f.state.get::<Value>("installation_manifest").unwrap()[0]["enabled"],
        true
    );
    let proposals = f
        .state
        .get::<BTreeMap<String, Proposal>>("proposals")
        .unwrap();
    let proposed: Value = serde_json::from_slice(
        &fs::read(
            proposals
                .values()
                .next()
                .unwrap()
                .directory
                .join("resources/native-installations/installations.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(proposed[0]["enabled"], false);
    assert_eq!(proposed[0]["update"][0][5], "${SM_ENABLED}");
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
async fn interrupted_runs_are_marked_as_needing_attention() {
    let f = Fixture::new();
    let guard = sm::service::JobGuard::acquire(f.state.directory().unwrap()).unwrap();
    {
        let _run = sm::telemetry::RunTracker::start(&f.state, "cycle").unwrap();
    }
    drop(guard);
    let (status, page) = f.get("/jobs").await;
    assert_eq!(status, StatusCode::OK);
    let history = page.split("<h2>执行记录</h2>").nth(1).unwrap();
    assert!(history.contains("需处理"));
}

#[tokio::test]
async fn activity_keeps_historical_totals_and_labels_current_evidence() {
    let f = Fixture::new();
    record_event(
        sm::model::Client::Codex,
        &json!({
            "hook_event_name":"PostToolUse", "tool_name":"Read", "resource_id":"review",
            "tool_input":{"path":f.runtime.home.join(".agents/skills/review/SKILL.md")},
            "tool_response":{"status":"failed","duration_ms":12.5}
        }),
        f.state.directory().unwrap(),
    )
    .unwrap();
    ingest_events(f.state.directory().unwrap(), &f.state).unwrap();
    f.state
        .set(
            "usage_totals",
            &json!({"review":{"events":5,"last_seen":"2026-01-01T00:00:00Z"}}),
        )
        .unwrap();
    let (status, page) = f.get("/activity").await;
    assert_eq!(status, StatusCode::OK);
    for text in [
        "6 条",
        "历史摘要 5 条",
        "保留明细中的会话",
        "文件读取",
        "失败",
        "12.5 ms",
    ] {
        assert!(page.contains(text), "{text}");
    }
    assert!(!page.contains("完整执行了"));
}

#[tokio::test]
async fn diagnostic_download_is_private_and_shares_health_with_jobs() {
    let f = Fixture::new();
    let (status, page) = f.get("/jobs").await;
    assert_eq!(status, StatusCode::OK);
    for label in ["采集健康", "待消化", "存储与留存", "下载脱敏诊断"] {
        assert!(page.contains(label), "{label}");
    }
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/diagnostics.json")
                .header("Host", "127.0.0.1:8765")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains("attachment")
    );
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["manager_version"], env!("CARGO_PKG_VERSION"));
    assert!(body["health"]["storage"]["database_bytes"].is_number());
    let (status, _) = f
        .request(
            "GET",
            "/diagnostics.json",
            String::new(),
            &[("Origin", "https://foreign.invalid")],
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
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
    let mut catalog: sm::model::Catalog = f.state.get("catalog").unwrap();
    let kinds = ["hook", "rule", "agent", "mcp"];
    let statuses = [
        ApplyStatus::Unsupported,
        ApplyStatus::Conflict,
        ApplyStatus::Failed,
        ApplyStatus::Unknown,
    ];
    let mut results = BTreeMap::new();
    for (kind, status) in kinds.into_iter().zip(statuses) {
        catalog.resources.push(serde_json::from_value(json!({"id":kind,"kind":kind,"origin":"self","scope":"project","project":"app","agents":["codex"],"payloads":{"codex":format!("resources/{kind}")}})).unwrap());
        results.insert(kind, ResourceResult::new(status, "status evidence"));
    }
    f.state.set("resource_results", &results).unwrap();
    catalog.projects.insert("app".into(), "~/repo/app".into());
    f.state.set("catalog", &catalog).unwrap();
    let page = f.get("/resources").await.1;
    for label in [
        "Skill",
        "Hook",
        "规则",
        "Agent 定义",
        "MCP 配置",
        "暂不支持",
        "存在冲突",
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
    let validation=serde_json::from_value(json!({"status":"partial","checks":[{"name":"required","kind":"approved_fragment_check","before":true,"after":true}],"unverified":["No business replay"]})).unwrap();
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
