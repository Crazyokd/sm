mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::{FakeMrApi, Repo};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sm::{
    model::{ApplyStatus, OwnedWrite, ResourceResult, WriteBody},
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
async fn resource_views_group_upstreams_and_keep_filters_and_self_dependencies_independent() {
    let f = Fixture::new();
    let mut catalog: Value =
        serde_json::to_value(f.state.get::<sm::model::Catalog>("catalog").unwrap()).unwrap();
    catalog["projects"] = json!({"app":"~/repo/app"});
    catalog["upstreams"] = json!({
        "alpha-source":{"git":"https://example.invalid/alpha.git"},
        "beta-source":{"git":"https://example.invalid/beta.git"}
    });
    catalog["dependencies"] = json!({"shared":{"origin":"upstream","upstream":"alpha-source","files":{"root":".","entries":[{"target":".codex/hooks/shared.sh","source":"shared.sh"}]}}});
    catalog["resources"].as_array_mut().unwrap().extend([
        json!({"id":"remote-skill","kind":"skill","origin":"upstream","upstream":"alpha-source","scope":"user","project":null,"payload":".agents/skills/remote-skill"}),
        json!({"id":"remote-rule","kind":"rule","origin":"upstream","upstream":"alpha-source","scope":"user","project":null,"agents":["codex"],"payloads":{"codex":"rules"}}),
        json!({"id":"project-skill","kind":"skill","origin":"upstream","upstream":"beta-source","scope":"project","project":"app","payload":".agents/skills/project-skill"}),
        json!({"id":"own-hook","kind":"hook","origin":"self","scope":"user","project":null,"dependencies":["shared"],"agents":["codex"],"payloads":{"codex":"resources/own-hook"}}),
    ]);
    let catalog: sm::model::Catalog = serde_json::from_value(catalog).unwrap();
    f.state.set("catalog", &catalog).unwrap();
    let results = BTreeMap::from([
        (
            "remote-skill",
            ResourceResult::new(ApplyStatus::Applied, "applied"),
        ),
        (
            "remote-rule",
            ResourceResult::new(ApplyStatus::Failed, "application failed"),
        ),
    ]);
    f.state.set("resource_results", &results).unwrap();
    let before = serde_json::to_value(&catalog).unwrap();
    let (status, page) = f.get("/resources").await;
    assert_eq!(status, StatusCode::OK);
    let group = |page: &str, id: &str| -> String {
        page.split(&format!("data-group=\"{id}\""))
            .nth(1)
            .unwrap()
            .split("</details>")
            .next()
            .unwrap()
            .to_owned()
    };
    let alpha = group(&page, "alpha-source");
    assert!(!alpha.split('>').next().unwrap().contains("open"));
    for text in [
        "remote-skill",
        "remote-rule",
        "Skill 1",
        "规则 1",
        "文件已应用 1",
        "应用失败 1",
        "全组应用情况",
    ] {
        assert!(alpha.contains(text), "{text}");
    }
    assert!(!alpha.contains("own-hook") && !alpha.contains("project-skill"));
    assert_eq!(page.matches("data-resource-id=\"own-hook\"").count(), 1);
    assert!(page.contains("查看同步记录"));
    let kinds = f.get("/resources?view=kind&kind=skill").await.1;
    assert_eq!(
        kinds.matches("<details class=\"resource-group\"").count(),
        1
    );
    for id in ["review", "remote-skill", "project-skill"] {
        assert_eq!(
            kinds.matches(&format!("data-resource-id=\"{id}\"")).count(),
            1
        );
    }
    assert!(!kinds.contains("data-resource-id=\"remote-rule\""));
    let filtered = f
        .get("/resources?origin=upstream&kind=skill&scope=project&agent=kimi")
        .await
        .1;
    assert!(
        group(&filtered, "beta-source")
            .split('>')
            .next()
            .unwrap()
            .contains("open")
    );
    assert!(filtered.contains("data-resource-id=\"project-skill\""));
    assert!(!filtered.contains("data-group=\"alpha-source\""));
    for field in [
        "agent=kimi",
        "kind=skill",
        "origin=upstream",
        "scope=project",
        "view=kind",
    ] {
        assert!(filtered.contains(field), "view link lost {field}");
    }
    let partial = f.get("/resources?view=origin&q=remote-skill").await.1;
    let partial = group(&partial, "alpha-source");
    assert!(partial.contains("匹配 1 / 2 项") && partial.contains("应用失败 1"));
    assert!(!partial.contains("data-resource-id=\"remote-rule\""));
    let by_source = f.get("/resources?q=alpha-source").await.1;
    assert!(by_source.contains("data-resource-id=\"remote-rule\""));
    assert!(!by_source.contains("data-resource-id=\"own-hook\""));
    for path in [
        "/resources?view=kind&origin=self",
        "/resources?view=kind&q=space+%26+%E6%B5%8B%E8%AF%95",
    ] {
        let (status, _) = f.get(path).await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(
        f.get("/resources?view=unexpected").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        serde_json::to_value(f.state.get::<sm::model::Catalog>("catalog").unwrap()).unwrap(),
        before
    );
    assert_eq!(
        f.state
            .get::<BTreeMap<String, ResourceResult>>("resource_results")
            .unwrap(),
        results
            .into_iter()
            .map(|(id, value)| (id.to_owned(), value))
            .collect()
    );
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
async fn inventory_shared_file_has_one_row_and_searches_every_resource_owner() {
    let f = Fixture::new();
    let mut catalog: sm::model::Catalog = f.state.get("catalog").unwrap();
    for id in ["alpha-hook", "omega-hook"] {
        catalog.resources.push(
            serde_json::from_value(json!({
                "id":id,"kind":"hook","origin":"self","scope":"user","project":null,
                "agents":["codex"],"payloads":{"codex":format!("resources/{id}")}
            }))
            .unwrap(),
        );
    }
    f.state.set("catalog", &catalog).unwrap();
    let path = f.runtime.home.join(".codex/hooks/shared-runtime.sh");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = b"#!/bin/sh\n# shared support\n";
    fs::write(&path, bytes).unwrap();
    let writes = ["alpha-hook", "omega-hook"].map(|id| {
        OwnedWrite::new(
            id,
            &path,
            WriteBody::File {
                bytes: bytes.to_vec(),
                executable: false,
            },
        )
    });
    f.state.replace_owned(&path, &writes).unwrap();
    for owner in ["alpha-hook", "omega-hook"] {
        let (status, page) = f.get(&format!("/inventory?q={owner}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            page.matches("<strong>shared-runtime.sh</strong>").count(),
            1
        );
        assert!(page.contains("关联文件"));
        assert!(page.contains("<a href=\"/resources/alpha-hook\">alpha-hook</a>"));
        assert!(page.contains("<details><summary>另有1个资源使用</summary><div><a href=\"/resources/omega-hook\">omega-hook</a>"));
    }
}

#[tokio::test]
async fn plugin_toggle_proposes_reviewed_declaration_without_mutating_native_installation() {
    let f = Fixture::new();
    let installation = json!({"agent":"kimi","source":"https://example.invalid/plugin","selector":"example","enabled":true,"paths":["~/.kimi-code/plugins/managed/example"],"update":[["node","resources/source-providers/native-plugins.js","kimi","https://example.invalid/plugin","example","${SM_ENABLED}"]]});
    let mut catalog: Value =
        serde_json::from_slice(&fs::read(f.repo.work.join("catalog.json")).unwrap()).unwrap();
    let mut other = installation.clone();
    other["selector"] = "other".into();
    other["paths"] = json!(["~/.kimi-code/plugins/managed/other"]);
    other["update"][0][4] = "other".into();
    for (id, policy) in [("kimi-example", &installation), ("kimi-other", &other)] {
        catalog["resources"].as_array_mut().unwrap().push(json!({"id":id,"kind":"policy","origin":"self","scope":"user","project":null,"installation":policy}));
    }
    f.repo.catalog(catalog.clone());
    f.repo
        .publish("Register individual native installation policies");
    f.state
        .set("catalog", &sm::model::Catalog::load(&f.repo.work).unwrap())
        .unwrap();
    let (status, page) = f.get("/resources/kimi-example").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("插件内容由上游维护"));
    assert!(page.contains("订阅与启停经 MR 审核后应用"));
    assert!(page.contains("<dt>插件上游</dt>"));
    assert!(f.get("/resources").await.1.contains("订阅声明"));
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
        serde_json::from_slice::<Value>(&fs::read(f.repo.work.join("catalog.json")).unwrap())
            .unwrap(),
        catalog
    );
    let proposals = f
        .state
        .get::<BTreeMap<String, Proposal>>("proposals")
        .unwrap();
    let proposal = proposals.values().next().unwrap();
    assert_eq!(proposal.change.resource_ids, ["kimi-example"]);
    assert_eq!(
        proposal
            .change
            .writes
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["catalog.json"]
    );
    let proposed: Value =
        serde_json::from_slice(&fs::read(proposal.directory.join("catalog.json")).unwrap())
            .unwrap();
    let resources = proposed["resources"].as_array().unwrap();
    assert_eq!(
        resources
            .iter()
            .find(|r| r["id"] == "kimi-example")
            .unwrap()["installation"]["enabled"],
        false
    );
    assert_eq!(
        resources.iter().find(|r| r["id"] == "kimi-other").unwrap()["installation"],
        other
    );
    assert_eq!(
        resources
            .iter()
            .find(|r| r["id"] == "kimi-example")
            .unwrap()["installation"]["update"][0][5],
        "${SM_ENABLED}"
    );
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
async fn resource_types_statuses_and_project_groups_use_same_catalog() {
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
        "提示词",
        "执行策略",
        "暂不支持",
        "存在冲突",
        "应用失败",
        "尚未应用",
    ] {
        assert!(page.contains(label), "{label}");
    }
    assert!(!page.contains("value=\"bundle\"") && !page.contains("资源集合"));
    assert!(!page.contains("value=\"script\""));
    assert_eq!(
        f.get("/resources?kind=script").await.0,
        StatusCode::BAD_REQUEST
    );
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

#[tokio::test]
async fn former_members_have_independent_filter_details_edit_and_retirement_proposals() {
    let f = Fixture::new();
    let mut catalog: Value =
        serde_json::from_slice(&fs::read(f.repo.work.join("catalog.json")).unwrap()).unwrap();
    let first = json!({"id":"first-rule","kind":"rule","origin":"self","scope":"user","project":null,"files":{"root":"resources/first-rule/files","entries":[{"target":".codex/AGENTS.md","source":"AGENTS.md"}]}});
    let second = json!({"id":"second-mcp","kind":"mcp","origin":"self","scope":"user","project":null,"files":{"root":"resources/second-mcp/files","entries":[{"target":".codex/config.toml","fragment":"config.json"}]}});
    catalog["resources"]
        .as_array_mut()
        .unwrap()
        .extend([first.clone(), second.clone()]);
    fs::create_dir_all(f.repo.work.join("resources/first-rule/files")).unwrap();
    fs::write(
        f.repo.work.join("resources/first-rule/files/AGENTS.md"),
        "# Independent rule",
    )
    .unwrap();
    fs::create_dir_all(f.repo.work.join("resources/second-mcp/files")).unwrap();
    let fragment = "{\"mcp_servers\":{\"example\":{\"command\":\"example-server\"}}}";
    fs::write(
        f.repo.work.join("resources/second-mcp/files/config.json"),
        fragment,
    )
    .unwrap();
    f.repo.catalog(catalog.clone());
    f.repo
        .publish("Register independently managed rules and MCP");
    f.state
        .set("catalog", &sm::model::Catalog::load(&f.repo.work).unwrap())
        .unwrap();
    f.state
        .set(
            "resource_contents",
            &BTreeMap::from([
                (
                    "first-rule",
                    BTreeMap::from([(
                        "resources/first-rule/files/AGENTS.md",
                        "# Independent rule",
                    )]),
                ),
                (
                    "second-mcp",
                    BTreeMap::from([("resources/second-mcp/files/config.json", fragment)]),
                ),
            ]),
        )
        .unwrap();
    let page = f.get("/resources?kind=rule").await.1;
    let table = page
        .split("<tbody>")
        .nth(1)
        .unwrap()
        .split("</tbody>")
        .next()
        .unwrap();
    assert!(table.contains("first-rule") && !table.contains("second-mcp"));
    assert!(!page.contains("value=\"bundle\"") && !page.contains("资源集合"));
    let kimi = f.get("/resources?agent=kimi&q=first-rule").await.1;
    let kimi_table = kimi
        .split("<tbody>")
        .nth(1)
        .unwrap()
        .split("</tbody>")
        .next()
        .unwrap();
    assert!(!kimi_table.contains("first-rule"));
    assert!(
        f.get("/resources/first-rule")
            .await
            .1
            .contains("Independent rule")
    );
    let mcp = f.get("/resources/second-mcp").await.1;
    assert!(mcp.contains(".codex/config.toml") && mcp.contains("example-server"));
    let token = f.token().await;
    let headers = [
        ("Content-Type", "application/json"),
        ("X-SM-CSRF", token.as_str()),
        ("Origin", "http://127.0.0.1:8765"),
    ];
    let change = json!({"resource_ids":["first-rule"],"reason":"Edit only this rule","writes":{"resources/first-rule/files/AGENTS.md":"# Reviewed rule"},"validation":{"status":"partial"}});
    let (status, body) = f
        .request("POST", "/proposals", change.to_string(), &headers)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let config_change = json!({"resource_ids":["second-mcp"],"reason":"Edit only this MCP fragment","writes":{"resources/second-mcp/files/config.json":"{\"mcp_servers\":{\"example\":{\"command\":\"reviewed-server\"}}}"},"validation":{"status":"partial"}});
    let (status, body) = f
        .request("POST", "/proposals", config_change.to_string(), &headers)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        fs::read_to_string(f.repo.work.join("resources/second-mcp/files/config.json")).unwrap(),
        fragment
    );
    catalog["resources"]
        .as_array_mut()
        .unwrap()
        .retain(|resource| resource["id"] != "second-mcp");
    let change = json!({"resource_ids":["second-mcp"],"reason":"Retire only this MCP","writes":{"catalog.json":serde_json::to_string_pretty(&catalog).unwrap()+"\n"},"validation":{"status":"partial"}});
    let (status, body) = f
        .request("POST", "/proposals", change.to_string(), &headers)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let proposals: BTreeMap<String, Proposal> = f.state.get("proposals").unwrap();
    assert_eq!(proposals.len(), 3);
    assert!(proposals.values().any(|p| {
        p.change.resource_ids == ["second-mcp"]
            && p.change
                .writes
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                == ["resources/second-mcp/files/config.json"]
    }));
    assert!(
        proposals
            .values()
            .any(|p| p.change.resource_ids == ["first-rule"])
    );
    assert!(
        proposals
            .values()
            .any(|p| p.change.resource_ids == ["second-mcp"])
    );
    let unchanged: Value =
        serde_json::from_slice(&fs::read(f.repo.work.join("catalog.json")).unwrap()).unwrap();
    assert!(unchanged["resources"].as_array().unwrap().contains(&second));
    assert_eq!(
        fs::read_to_string(f.repo.work.join("resources/first-rule/files/AGENTS.md")).unwrap(),
        "# Independent rule"
    );
}

#[tokio::test]
async fn project_instruction_and_mcp_filters_follow_native_scope_semantics() {
    let f = Fixture::new();
    let mut catalog: sm::model::Catalog = f.state.get("catalog").unwrap();
    catalog.projects.insert("app".into(), "~/repo/app".into());
    for (id, target) in [
        ("project-root-rule", "AGENTS.md"),
        ("project-nested-rule", "src/AGENTS.md"),
        ("claude-private-rule", ".claude/AGENTS.md"),
    ] {
        catalog.resources.push(serde_json::from_value(json!({
            "id":id,"kind":"rule","origin":"self","scope":"project","project":"app",
            "files":{"root":format!("resources/{id}/files"),"entries":[{"target":target,"source":"AGENTS.md"}]}
        })).unwrap());
    }
    catalog.resources.push(serde_json::from_value(json!({
        "id":"project-mcp","kind":"mcp","origin":"self","scope":"project","project":"app",
        "files":{"root":"resources/project-mcp/files","entries":[{"target":".mcp.json","json":{"mcpServers":{"example":{"command":"example"}}}}]}
    })).unwrap());
    f.state.set("catalog", &catalog).unwrap();
    for client in ["codex", "kimi"] {
        let (status, page) = f
            .get(&format!(
                "/resources?kind=rule&scope=project&agent={client}"
            ))
            .await;
        assert_eq!(status, StatusCode::OK);
        let table = page
            .split("<tbody>")
            .nth(1)
            .unwrap()
            .split("</tbody>")
            .next()
            .unwrap();
        assert!(table.contains("project-root-rule") && table.contains("project-nested-rule"));
        assert!(!table.contains("claude-private-rule"));
        assert!(table.contains("Codex") && table.contains("Kimi"));
    }
    for (client, visible) in [("kimi", true), ("codex", false)] {
        let page = f
            .get(&format!("/resources?kind=mcp&scope=project&agent={client}"))
            .await
            .1;
        let table = page
            .split("<tbody>")
            .nth(1)
            .unwrap()
            .split("</tbody>")
            .next()
            .unwrap();
        assert_eq!(table.contains("project-mcp"), visible);
    }
}

#[tokio::test]
async fn skill_tree_details_show_nested_helpers_under_their_target_paths() {
    let f = Fixture::new();
    let mut catalog: sm::model::Catalog = f.state.get("catalog").unwrap();
    catalog.resources.push(serde_json::from_value(json!({
        "id":"tree-skill","kind":"skill","origin":"self","scope":"user","project":null,
        "files":{"root":"resources/tree-skill/files","entries":[{"target":".agents/skills/tree-skill","tree":"skill"}]}
    })).unwrap());
    f.state.set("catalog", &catalog).unwrap();
    f.state
        .set(
            "resource_contents",
            &BTreeMap::from([(
                "tree-skill",
                BTreeMap::from([
                    ("resources/tree-skill/files/skill/SKILL.md", "# One Skill"),
                    (
                        "resources/tree-skill/files/skill/references/helper.md",
                        "# Nested helper body",
                    ),
                ]),
            )]),
        )
        .unwrap();
    let (status, page) = f.get("/resources/tree-skill").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("Nested helper body") && page.contains("helper.md"));
    for client in ["codex", "kimi"] {
        let page = f
            .get(&format!("/resources?kind=skill&agent={client}"))
            .await
            .1;
        let table = page
            .split("<tbody>")
            .nth(1)
            .unwrap()
            .split("</tbody>")
            .next()
            .unwrap();
        assert!(table.contains("tree-skill") && table.contains("通用 Skill"));
    }
}

#[tokio::test]
async fn complete_hook_details_show_owned_files_and_referenced_support_without_extra_resources() {
    let f = Fixture::new();
    let mut catalog: Value =
        serde_json::to_value(f.state.get::<sm::model::Catalog>("catalog").unwrap()).unwrap();
    catalog["upstreams"]["runtime-source"] = json!({"git":"https://example.invalid/runtime.git"});
    catalog["dependencies"] = json!({
        "shared-runtime":{"origin":"upstream","upstream":"runtime-source","files":{"root":"hooks","entries":[{"target":".codex/hooks/shared.sh","source":"shared.sh"}]}},
        "owned-runtime":{"origin":"self","files":{"root":"resources/dependencies/owned-runtime/files","entries":[{"target":".codex/hooks/managed.sh","source":"managed.sh"}]}}
    });
    catalog["resources"].as_array_mut().unwrap().push(json!({
        "id":"complete-hook","kind":"hook","origin":"self","scope":"user","project":null,
        "dependencies":["shared-runtime","owned-runtime"],
        "files":{"root":"resources/complete-hook/files","entries":[{"target":".codex/hooks.json","fragment":"hooks.json"},{"target":".codex/hooks/owned.sh","source":"owned.sh"}]}
    }));
    f.state
        .set(
            "catalog",
            &serde_json::from_value::<sm::model::Catalog>(catalog).unwrap(),
        )
        .unwrap();
    f.state.set("resource_contents", &BTreeMap::from([("complete-hook", BTreeMap::from([
        ("resources/complete-hook/files/hooks.json", "{\"hooks\":{\"Stop\":[{\"hooks\":[{\"type\":\"command\",\"command\":\"owned.sh\"}]}]}}"),
        ("resources/complete-hook/files/owned.sh", "#!/bin/sh\necho owned-hook-body\n"),
    ]))])).unwrap();
    let (status, page) = f.get("/resources/complete-hook").await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains(".codex/hooks.json") && page.contains("owned-hook-body"));
    let details = "<details class=\"article-section supporting-files\">";
    assert!(page.find("owned-hook-body").unwrap() < page.find(details).unwrap());
    let supporting = page
        .split(details)
        .nth(1)
        .unwrap()
        .split("</details>")
        .next()
        .unwrap();
    assert!(supporting.starts_with("<summary>支持文件（2）</summary>"));
    for path in [
        ".codex/hooks/shared.sh",
        "hooks/shared.sh",
        ".codex/hooks/managed.sh",
        "resources/dependencies/owned-runtime/files/managed.sh",
    ] {
        assert!(supporting.contains(&path.replace('/', "&#x2f;")), "{path}");
    }
    assert!(supporting.contains("上游拉取 · runtime-source") && supporting.contains("自行维护"));
    let page = f.get("/resources?kind=hook").await.1;
    let table = page
        .split("<tbody>")
        .nth(1)
        .unwrap()
        .split("</tbody>")
        .next()
        .unwrap();
    assert!(table.contains("complete-hook"));
    assert!(!table.contains("shared-runtime") && !table.contains("owned-runtime"));
    assert!(!page.contains("value=\"script\""));
    assert_eq!(
        f.get("/resources/shared-runtime").await.0,
        StatusCode::NOT_FOUND
    );
}
