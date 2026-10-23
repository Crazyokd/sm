use crate::{
    model::{
        ApplyStatus, Catalog, Client, Resource, ResourceContent, ResourceKind, ResourceResult,
        Scope, Source, UpstreamSource,
    },
    observe::{Coverage, Observation, RecordingError},
    privacy::redact_text,
    proposals::{ChangeSet, Proposal},
    service::{Busy, Finding, JobGuard, NativeSubmitter, Runtime, Submitter, synchronize},
    state::{RunRecord, State},
    wire::truncate,
};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path as WebPath, Query, Request, State as WebState,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use minijinja::{AutoEscape, Environment};
use pulldown_cmark::{Event, Parser, Tag, html};
use serde::{
    Deserialize, Serialize,
    de::{DeserializeOwned, Deserializer, value::StringDeserializer},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};
use subtle::ConstantTimeEq;

struct WebContext {
    runtime: Runtime,
    state: State,
    templates: Environment<'static>,
    csrf: String,
    submitter: Arc<dyn Submitter>,
}
type Shared = Arc<WebContext>;
struct WebError {
    status: StatusCode,
    message: String,
}
impl WebError {
    fn bad(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: redact_text(&error.to_string(), Some(600)),
        }
    }
    fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!(
            "SM web error: {}",
            redact_text(&error.to_string(), Some(1200))
        );
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "页面或操作加载失败，请查看服务日志。".into(),
        }
    }
}
impl From<anyhow::Error> for WebError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(format!("{error:#}"))
    }
}
impl From<rusqlite::Error> for WebError {
    fn from(error: rusqlite::Error) -> Self {
        Self::internal(error)
    }
}
impl From<serde_json::Error> for WebError {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(error)
    }
}
impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}
type WebResult<T> = std::result::Result<T, WebError>;
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> WebResult<T> + Send + 'static,
) -> WebResult<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(WebError::internal)?
}
fn templates() -> Result<Environment<'static>> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|name| {
        if name.ends_with(".html") {
            AutoEscape::Html
        } else {
            AutoEscape::None
        }
    });
    env.add_function("inventory_client", |path: String| {
        for (part, label) in [
            ("/.codex/", "Codex"),
            ("/.kimi-code/", "Kimi"),
            ("/.claude/", "Claude"),
            ("/.cursor/", "Cursor"),
        ] {
            if path.contains(part) {
                return label;
            }
        }
        "通用"
    });
    env.add_function("inventory_kind", |kind: String| match kind.as_str() {
        "skill" => "Skill",
        "hook" => "Hook",
        "policy" => "执行策略",
        "rule" => "规则",
        "agent" => "Agent",
        "prompt" => "提示词／命令",
        "mcp" => "MCP",
        "plugin" => "插件",
        "skill-config" => "Skill 启停",
        "config" => "原生配置",
        _ => "关联文件",
    });
    env.add_function("format_bytes", |bytes: u64| {
        if bytes >= 1024 * 1024 {
            format!("{:.1} MiB", bytes as f64 / 1048576.0)
        } else if bytes >= 1024 {
            format!("{:.1} KiB", bytes as f64 / 1024.0)
        } else {
            format!("{bytes} B")
        }
    });
    env.add_function("stage_label", |stage: String| {
        match stage.as_str() {
            "starting" => "准备任务",
            "configure" => "生成配置",
            "register_services" => "注册服务",
            "fetch_approved" => "获取已审核资源",
            "catalog" => "读取清单",
            "fetch_resource" => "获取资源",
            "apply" => "投放文件",
            "apply_existing" => "接管已有资源",
            "cache_metadata" => "更新资源状态",
            "ingest" => "消化本地记录",
            "analysis" => "分析自建资源",
            "validation" => "验证候选修改",
            "proposal" => "保存提案",
            "submit_mr" => "提交 MR",
            "complete_observations" => "标记记录处理完成",
            "retention" => "清理过期明细",
            _ => return stage,
        }
        .to_owned()
    });
    for (name, text) in [
        ("base.html", include_str!("../assets/templates/base.html")),
        (
            "_editor.html",
            include_str!("../assets/templates/_editor.html"),
        ),
        (
            "resources.html",
            include_str!("../assets/templates/resources.html"),
        ),
        (
            "resource.html",
            include_str!("../assets/templates/resource.html"),
        ),
        (
            "projects.html",
            include_str!("../assets/templates/projects.html"),
        ),
        (
            "activity.html",
            include_str!("../assets/templates/activity.html"),
        ),
        ("jobs.html", include_str!("../assets/templates/jobs.html")),
        (
            "inventory.html",
            include_str!("../assets/templates/inventory.html"),
        ),
    ] {
        env.add_template(name, text)?;
    }
    Ok(env)
}
pub fn router(runtime: Runtime, state: State) -> Result<Router> {
    router_with(runtime, state, Arc::new(NativeSubmitter))
}
pub fn router_with(
    runtime: Runtime,
    state: State,
    submitter: Arc<dyn Submitter>,
) -> Result<Router> {
    let context = Arc::new(WebContext {
        runtime,
        state,
        templates: templates()?,
        csrf: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
        submitter,
    });
    Ok(Router::new()
        .route("/", get(|| async { Redirect::temporary("/resources") }))
        .route("/resources", get(resources))
        .route("/resources/{id}", get(resource))
        .route("/projects", get(projects))
        .route("/activity", get(activity))
        .route("/jobs", get(jobs))
        .route("/inventory", get(inventory))
        .route("/inventory.json", get(inventory_json))
        .route("/installations/{id}", post(propose_installation))
        .route("/diagnostics.json", get(download_diagnostics))
        .route("/proposals", post(propose))
        .route(
            "/static/app.js",
            get(|| async {
                (
                    [("Content-Type", "application/javascript; charset=utf-8")],
                    include_str!("../assets/static/app.js"),
                )
            }),
        )
        .route(
            "/static/style.css",
            get(|| async {
                (
                    [("Content-Type", "text/css; charset=utf-8")],
                    include_str!("../assets/static/style.css"),
                )
            }),
        )
        .with_state(context.clone())
        .layer(DefaultBodyLimit::max(1048576))
        .layer(middleware::from_fn_with_state(context, local_boundary)))
}
async fn local_boundary(
    WebState(context): WebState<Shared>,
    request: Request,
    next: Next,
) -> Response {
    let check = (|| -> WebResult<()> {
        let host = request
            .headers()
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let url = url::Url::parse(&format!("http://{host}")).map_err(|_| WebError {
            status: StatusCode::FORBIDDEN,
            message: "Local access only".into(),
        })?;
        let allowed = match url.host() {
            Some(url::Host::Domain("localhost")) => true,
            Some(url::Host::Ipv4(ip)) => ip == std::net::Ipv4Addr::LOCALHOST,
            Some(url::Host::Ipv6(ip)) => ip == std::net::Ipv6Addr::LOCALHOST,
            _ => false,
        };
        if !allowed {
            return Err(WebError {
                status: StatusCode::FORBIDDEN,
                message: "Local access only".into(),
            });
        }
        if request.uri().path() == "/diagnostics.json"
            && (request
                .headers()
                .get("sec-fetch-site")
                .is_some_and(|v| v == "cross-site")
                || request
                    .headers()
                    .get("origin")
                    .is_some_and(|v| v.as_bytes() != format!("http://{host}").as_bytes()))
        {
            return Err(WebError {
                status: StatusCode::FORBIDDEN,
                message: "Origin rejected".into(),
            });
        }
        if !matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS") {
            if request
                .headers()
                .get("origin")
                .and_then(|v| v.to_str().ok())
                != Some(format!("http://{host}").as_str())
            {
                return Err(WebError {
                    status: StatusCode::FORBIDDEN,
                    message: "Origin rejected".into(),
                });
            }
            let token = request
                .headers()
                .get("x-sm-csrf")
                .map(|v| v.as_bytes())
                .unwrap_or_default();
            if !bool::from(token.ct_eq(context.csrf.as_bytes())) {
                return Err(WebError {
                    status: StatusCode::FORBIDDEN,
                    message: "CSRF token required".into(),
                });
            }
            if let Some(length) = request.headers().get("content-length") {
                let length = length
                    .to_str()
                    .map_err(WebError::bad)?
                    .parse::<usize>()
                    .map_err(WebError::bad)?;
                if length > 1048576 {
                    return Err(WebError {
                        status: StatusCode::PAYLOAD_TOO_LARGE,
                        message: "Change is too large".into(),
                    });
                }
            }
        }
        Ok(())
    })();
    let mut response = match check {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    };
    for (name, value) in [
        ("X-Content-Type-Options", "nosniff"),
        ("X-Frame-Options", "DENY"),
        (
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
#[derive(Debug, Clone, Serialize)]
struct ResourceView {
    #[serde(flatten)]
    resource: Resource,
    kind_label: &'static str,
    upstream_path: String,
    source_definition: Option<UpstreamSource>,
    client_label: String,
    supported_clients: BTreeSet<Client>,
    status: ApplyStatus,
    status_label: &'static str,
    status_message: String,
}
fn resource_rows(state: &State) -> Result<Vec<ResourceView>> {
    let catalog = state.get::<Catalog>("catalog")?;
    let resources = catalog.resources;
    let results: BTreeMap<String, ResourceResult> = state.get("resource_results")?;
    Ok(resources
        .into_iter()
        .map(|resource| {
            let result = results.get(&resource.id);
            let status = result.map_or(ApplyStatus::Unknown, |r| r.status);
            let kind = resource.content.kind();
            let targets = match &resource.content {
                ResourceContent::Files { files, .. } => files
                    .entries
                    .iter()
                    .map(|entry| match entry {
                        crate::model::ProjectionEntry::File { target, .. }
                        | crate::model::ProjectionEntry::Tree { target, .. }
                        | crate::model::ProjectionEntry::Fragment { target, .. }
                        | crate::model::ProjectionEntry::Json { target, .. }
                        | crate::model::ProjectionEntry::Toml { target, .. } => target.as_path(),
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            let shared_instruction = |target: &Path| {
                resource.scope == Scope::Project
                    && kind == ResourceKind::Rule
                    && target.file_name().is_some_and(|name| name == "AGENTS.md")
                    && ![".codex", ".kimi-code", ".claude", ".cursor"]
                        .into_iter()
                        .any(|prefix| target.starts_with(prefix))
            };
            let project_mcp = |target: &Path| {
                resource.scope == Scope::Project
                    && kind == ResourceKind::Mcp
                    && target == Path::new(".mcp.json")
            };
            let supported_clients = if kind == ResourceKind::Skill {
                Client::ALL.into_iter().collect()
            } else if let ResourceContent::Installation { installation } = &resource.content {
                BTreeSet::from([installation.agent])
            } else if matches!(resource.content, ResourceContent::Files { .. }) {
                Client::ALL
                    .into_iter()
                    .filter(|client| {
                        targets.iter().any(|target| {
                            target.starts_with(match client {
                                Client::Codex => ".codex",
                                Client::Kimi => ".kimi-code",
                            }) || target.starts_with(".agents")
                                || shared_instruction(target)
                                || (*client == Client::Kimi && project_mcp(target))
                        })
                    })
                    .collect()
            } else {
                resource.targets().into_keys().collect()
            };
            let client_label = if kind == ResourceKind::Skill {
                "通用 Skill".into()
            } else if let ResourceContent::Installation { installation } = &resource.content {
                installation.agent.to_string()
            } else if matches!(resource.content, ResourceContent::Files { .. }) {
                targets
                    .iter()
                    .map(|target| {
                        if shared_instruction(target) {
                            return "Codex / Kimi";
                        }
                        if project_mcp(target) {
                            return "Kimi";
                        }
                        [
                            (".codex", "Codex"),
                            (".kimi-code", "Kimi"),
                            (".claude", "Claude"),
                            (".cursor", "Cursor"),
                        ]
                        .into_iter()
                        .find_map(|(prefix, label)| target.starts_with(prefix).then_some(label))
                        .unwrap_or("通用")
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(" / ")
            } else {
                resource
                    .targets()
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            let upstream_path = resource
                .targets()
                .into_values()
                .next()
                .map_or_else(|| ".".to_owned(), |p| p.to_string_lossy().into_owned());
            let source_definition = match &resource.source {
                Source::Upstream { upstream } => catalog.upstreams.get(upstream).cloned(),
                Source::SelfOwned => None,
            };
            ResourceView {
                resource,
                source_definition,
                upstream_path,
                kind_label: kind.label(),
                client_label,
                supported_clients,
                status,
                status_label: status.label(),
                status_message: result.map_or_else(|| "尚未应用".into(), |r| r.message.clone()),
            }
        })
        .collect())
}
#[derive(Serialize)]
struct BaseContext {
    nav: &'static str,
    catalog: Catalog,
    csrf_token: String,
    kind_labels: Vec<(&'static str, &'static str)>,
    total: usize,
    self_count: usize,
    project_count: usize,
}
fn base(context: &WebContext, nav: &'static str, rows: &[ResourceView]) -> Result<BaseContext> {
    Ok(BaseContext {
        nav,
        catalog: context.state.get("catalog")?,
        csrf_token: context.csrf.clone(),
        kind_labels: vec![
            ("skill", "Skill"),
            ("hook", "Hook"),
            ("rule", "规则"),
            ("agent", "Agent 定义"),
            ("mcp", "MCP 配置"),
            ("prompt", "提示词"),
            ("policy", "执行策略"),
        ],
        total: rows.len(),
        self_count: rows.iter().filter(|r| r.resource.source.is_self()).count(),
        project_count: rows
            .iter()
            .filter(|r| r.resource.scope == Scope::Project)
            .count(),
    })
}
fn render<T: Serialize>(context: &WebContext, name: &str, page: &T) -> WebResult<Html<String>> {
    Ok(Html(
        context
            .templates
            .get_template(name)
            .map_err(WebError::internal)?
            .render(page)
            .map_err(WebError::internal)?,
    ))
}
fn optional<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let text = String::deserialize(deserializer)?;
    if text.is_empty() {
        Ok(None)
    } else {
        T::deserialize(StringDeserializer::<D::Error>::new(text)).map(Some)
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum OriginFilter {
    #[serde(rename = "self")]
    SelfOwned,
    #[serde(rename = "upstream")]
    Upstream,
}
#[derive(Debug, Default, Serialize, Deserialize)]
struct Filters {
    #[serde(default, deserialize_with = "optional")]
    origin: Option<OriginFilter>,
    #[serde(default, deserialize_with = "optional")]
    kind: Option<ResourceKind>,
    #[serde(default, deserialize_with = "optional")]
    scope: Option<Scope>,
    #[serde(default, deserialize_with = "optional")]
    agent: Option<Client>,
    #[serde(default)]
    q: String,
}
#[derive(Serialize)]
struct ResourcePage {
    #[serde(flatten)]
    base: BaseContext,
    resources: Vec<ResourceView>,
    filters: Filters,
}
#[derive(Serialize)]
struct InventoryPage {
    #[serde(flatten)]
    base: BaseContext,
    inventory: crate::inventory::Inventory,
    unmanaged: usize,
    drift: usize,
    native: usize,
    installations: Vec<crate::installations::Installation>,
    results: BTreeMap<String, ResourceResult>,
    entries: Vec<crate::inventory::InventoryEntry>,
    filters: InventoryFilters,
    kinds: BTreeSet<String>,
    owners: Vec<String>,
}
#[derive(Default, Deserialize, Serialize)]
struct InventoryFilters {
    #[serde(default)]
    q: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    kind: String,
}
async fn inventory(
    WebState(context): WebState<Shared>,
    query: std::result::Result<Query<InventoryFilters>, QueryRejection>,
) -> WebResult<Html<String>> {
    let Query(filters) = query.map_err(WebError::bad)?;
    blocking(move || {
        let rows = resource_rows(&context.state)?;
        let catalog = context.state.get("catalog")?;
        let inventory = crate::inventory::scan(&catalog, &context.state, &context.runtime.home)?;
        let unmanaged = inventory
            .entries
            .iter()
            .filter(|e| e.status == "unmanaged")
            .count();
        let drift = inventory
            .entries
            .iter()
            .filter(|e| e.status == "drift")
            .count();
        let native = inventory
            .entries
            .iter()
            .filter(|e| e.status == "native")
            .count();
        let kinds = inventory.entries.iter().map(|e| e.kind.clone()).collect();
        let q = filters.q.to_lowercase();
        let entries = inventory
            .entries
            .iter()
            .filter(|e| {
                (filters.status.is_empty() || e.status == filters.status)
                    && (filters.kind.is_empty() || e.kind == filters.kind)
                    && format!(
                        "{} {} {} {} {}",
                        e.id,
                        e.name,
                        e.project.as_deref().unwrap_or(""),
                        e.owner.as_deref().unwrap_or(""),
                        e.other_owners.join(" ")
                    )
                    .to_lowercase()
                    .contains(&q)
            })
            .cloned()
            .collect();
        let owners = rows.iter().map(|r| r.resource.id.clone()).collect();
        render(
            &context,
            "inventory.html",
            &InventoryPage {
                base: base(&context, "inventory", &rows)?,
                inventory,
                unmanaged,
                drift,
                native,
                installations: context.state.get("installation_manifest")?,
                results: context.state.get("resource_results")?,
                entries,
                filters,
                kinds,
                owners,
            },
        )
    })
    .await
}
async fn inventory_json(WebState(context): WebState<Shared>) -> WebResult<Response> {
    blocking(move || {
        let catalog = context.state.get("catalog")?;
        let inventory = crate::inventory::scan(&catalog, &context.state, &context.runtime.home)?;
        let mut response = Json(inventory).into_response();
        response
            .headers_mut()
            .insert("Cache-Control", HeaderValue::from_static("no-store"));
        Ok(response)
    })
    .await
}
async fn resources(
    WebState(context): WebState<Shared>,
    query: std::result::Result<Query<Filters>, QueryRejection>,
) -> WebResult<Html<String>> {
    let Query(filters) = query.map_err(WebError::bad)?;
    blocking(move || {
        let rows = resource_rows(&context.state)?;
        let base = base(&context, "resources", &rows)?;
        let resources = rows
            .into_iter()
            .filter(|r| {
                filters.origin.is_none_or(|origin| {
                    r.resource.source.is_self() == matches!(origin, OriginFilter::SelfOwned)
                }) && filters
                    .kind
                    .is_none_or(|kind| r.resource.content.kind() == kind)
                    && filters.scope.is_none_or(|scope| r.resource.scope == scope)
                    && filters
                        .agent
                        .is_none_or(|client| r.supported_clients.contains(&client))
                    && r.resource
                        .id
                        .to_lowercase()
                        .contains(&filters.q.to_lowercase())
            })
            .collect();
        render(
            &context,
            "resources.html",
            &ResourcePage {
                base,
                resources,
                filters,
            },
        )
    })
    .await
}
#[derive(Serialize)]
struct Article {
    title: String,
    html: Option<String>,
    text: String,
}
fn safe_url(url: &str) -> bool {
    let normalized: String = url
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .collect();
    if let Some((scheme, _)) = normalized.split_once(':') {
        if !scheme.contains(['/', '#', '?']) {
            return matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "mailto"
            );
        }
    }
    true
}
fn markdown(text: &str) -> String {
    let events = Parser::new(text).map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: if safe_url(&dest_url) {
                dest_url
            } else {
                "#".into()
            },
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: if safe_url(&dest_url) {
                dest_url
            } else {
                "".into()
            },
            title,
            id,
        }),
        event => event,
    });
    let mut output = String::new();
    html::push_html(&mut output, events);
    output
}
fn proposals_for(state: &State, id: &str) -> Result<Vec<Proposal>> {
    let proposals: BTreeMap<String, Proposal> = state.get("proposals")?;
    Ok(proposals
        .into_values()
        .filter(|p| p.change.resource_ids.iter().any(|rid| rid == id))
        .map(|mut p| {
            if p.mr_url.as_ref().is_some_and(|url| {
                url::Url::parse(url).map_or(true, |url| !matches!(url.scheme(), "http" | "https"))
            }) {
                p.mr_url = None;
            }
            p
        })
        .collect())
}
#[derive(Serialize)]
struct SupportingFileView {
    target: String,
    source_path: String,
    source_label: String,
}
#[derive(Serialize)]
struct DetailPage {
    #[serde(flatten)]
    base: BaseContext,
    resource: ResourceView,
    files: BTreeMap<String, String>,
    articles: Vec<Article>,
    supporting_files: Vec<SupportingFileView>,
    proposals: Vec<Proposal>,
}
async fn resource(
    WebState(context): WebState<Shared>,
    WebPath(id): WebPath<String>,
) -> WebResult<Html<String>> {
    blocking(move || {
        let rows = resource_rows(&context.state)?;
        let base = base(&context, "resources", &rows)?;
        let resource = rows
            .into_iter()
            .find(|r| r.resource.id == id)
            .ok_or_else(|| WebError {
                status: StatusCode::NOT_FOUND,
                message: "Resource not found".into(),
            })?;
        let mut contents: BTreeMap<String, BTreeMap<String, String>> =
            context.state.get("resource_contents")?;
        let files = contents.remove(&id).unwrap_or_default();
        let mut supporting_files = Vec::new();
        for name in &resource.resource.dependencies {
            let dependency = base
                .catalog
                .dependencies
                .get(name)
                .context("supporting files missing from catalog")?;
            for entry in &dependency.files.entries {
                if let crate::model::ProjectionEntry::File { target, source } = entry {
                    supporting_files.push(SupportingFileView {
                        target: target.to_string_lossy().into_owned(),
                        source_path: dependency
                            .files
                            .root
                            .join(source)
                            .to_string_lossy()
                            .into_owned(),
                        source_label: match &dependency.source {
                            Source::SelfOwned => "自行维护".into(),
                            Source::Upstream { upstream } => format!("上游拉取 · {upstream}"),
                        },
                    });
                }
            }
        }
        let mut articles = Vec::new();
        for (name, text) in &files {
            let body = if name.ends_with(".md") && text.starts_with("---\n") {
                text.splitn(3, "---").nth(2).unwrap_or(text)
            } else {
                text
            };
            let target = if let ResourceContent::Files { files, .. } = &resource.resource.content {
                files.entries.iter().find_map(|entry| match entry {
                    crate::model::ProjectionEntry::File { target, source }
                    | crate::model::ProjectionEntry::Fragment {
                        target,
                        fragment: source,
                    } if {
                        let path = files.root.join(source);
                        path.strip_prefix(".").unwrap_or(&path) == Path::new(name)
                    } =>
                    {
                        Some(target.to_string_lossy().into_owned())
                    }
                    crate::model::ProjectionEntry::Tree { target, tree } => {
                        let root = files.root.join(tree);
                        let root = root.strip_prefix(".").unwrap_or(&root);
                        Path::new(name)
                            .strip_prefix(root)
                            .ok()
                            .map(|relative| target.join(relative).to_string_lossy().into_owned())
                    }
                    _ => None,
                })
            } else {
                None
            };
            articles.push(Article {
                title: target.unwrap_or_else(|| {
                    Path::new(name)
                        .file_name()
                        .map_or_else(|| name.clone(), |n| n.to_string_lossy().into_owned())
                }),
                html: if name.ends_with(".md") {
                    Some(markdown(body))
                } else {
                    None
                },
                text: text.clone(),
            });
        }
        match &resource.resource.content {
            ResourceContent::Files {
                files: projection, ..
            } => {
                for entry in &projection.entries {
                    let (title, fragment) = match entry {
                        crate::model::ProjectionEntry::File { .. }
                        | crate::model::ProjectionEntry::Tree { .. }
                        | crate::model::ProjectionEntry::Fragment { .. } => continue,
                        crate::model::ProjectionEntry::Json { target, json } => (target, json),
                        crate::model::ProjectionEntry::Toml { target, toml } => (target, toml),
                    };
                    articles.push(Article {
                        title: title.to_string_lossy().into_owned(),
                        html: None,
                        text: serde_json::to_string_pretty(fragment)?,
                    });
                }
            }
            ResourceContent::Installation { installation } => articles.push(Article {
                title: "原生安装策略".into(),
                html: None,
                text: serde_json::to_string_pretty(installation)?,
            }),
            _ => {}
        }
        let proposals = proposals_for(&context.state, &id)?;
        render(
            &context,
            "resource.html",
            &DetailPage {
                base,
                resource,
                files,
                articles,
                supporting_files,
                proposals,
            },
        )
    })
    .await
}
#[derive(Serialize)]
struct ProjectsPage {
    #[serde(flatten)]
    base: BaseContext,
    groups: BTreeMap<String, Vec<ResourceView>>,
}
async fn projects(WebState(context): WebState<Shared>) -> WebResult<Html<String>> {
    blocking(move || {
        let rows = resource_rows(&context.state)?;
        let base = base(&context, "projects", &rows)?;
        let mut groups: BTreeMap<String, Vec<ResourceView>> = BTreeMap::new();
        for row in rows {
            let key = match row.resource.scope {
                Scope::User => "用户级".into(),
                Scope::Project => row.resource.project.clone().context("project missing")?,
            };
            groups.entry(key).or_default().push(row);
        }
        render(&context, "projects.html", &ProjectsPage { base, groups })
    })
    .await
}
fn time_label(timestamp: &str) -> String {
    truncate(timestamp, 19).replace('T', " ")
}
#[derive(Serialize)]
struct EventView {
    id: String,
    time_label: String,
    agent: Client,
    resource_id: Option<String>,
    event: String,
    coverage: Coverage,
    evidence_kind: crate::observe::EvidenceKind,
    outcome: Option<String>,
    duration_ms: Option<f64>,
    excerpt: String,
}
#[derive(Serialize)]
struct Usage {
    events: u64,
    sessions: u64,
    retired_events: u64,
    last_seen_label: String,
}
#[derive(Serialize)]
struct UsageView {
    #[serde(flatten)]
    resource: ResourceView,
    usage: Option<Usage>,
}
#[derive(Serialize)]
struct FindingView {
    #[serde(flatten)]
    finding: Finding,
    proposals: Vec<Proposal>,
}
#[derive(Serialize)]
struct ActivityPage {
    #[serde(flatten)]
    base: BaseContext,
    events: Vec<EventView>,
    findings: Vec<FindingView>,
    usage: Vec<UsageView>,
    recording_errors: Vec<RecordingError>,
    pending_raw_bytes: u64,
    pending_observations: u64,
}
async fn activity(WebState(context): WebState<Shared>) -> WebResult<Html<String>> {
    blocking(move || {
        let rows = resource_rows(&context.state)?;
        let base = base(&context, "activity", &rows)?;
        let db = context.state.connect()?;
        let mut query = db.prepare("SELECT value FROM observations ORDER BY rowid DESC LIMIT 100")?;
        let mut events = Vec::new();
        for text in query.query_map([], |row| row.get::<_, String>(0))? {
            let row: Observation = serde_json::from_str(&text?)?;
            let excerpt = row.facts.get("tool_input")
                .and_then(|i| i.get("command").or_else(|| i.get("path")))
                .and_then(serde_json::Value::as_str).unwrap_or_default();
            events.push(EventView {
                id: row.id, time_label: time_label(&row.timestamp), agent: row.agent,
                resource_id: row.resource_id, event: row.event, coverage: row.coverage,
                evidence_kind: row.evidence_kind,
                outcome: row.facts.get("outcome").and_then(serde_json::Value::as_str).map(str::to_owned),
                duration_ms: row.facts.get("duration_ms").and_then(serde_json::Value::as_f64),
                excerpt: redact_text(excerpt, Some(180)),
            });
        }
        let mut counts = BTreeMap::new();
        let mut query = db.prepare("SELECT json_extract(value,'$.resource_id'),COUNT(*),COUNT(DISTINCT NULLIF(json_extract(value,'$.session_id'),'')),MAX(json_extract(value,'$.timestamp')) FROM observations GROUP BY 1")?;
        for row in query.query_map([], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, u64>(1)?, row.get::<_, u64>(2)?, row.get::<_, String>(3)?)))? {
            let (id, events, sessions, last_seen) = row?;
            if let Some(id) = id { counts.insert(id, Usage { events, sessions, retired_events: 0, last_seen_label: time_label(&last_seen) }); }
        }
        for (id, total) in context.state.get::<BTreeMap<String, crate::retention::UsageTotal>>("usage_totals")? {
            let usage = counts.entry(id).or_insert(Usage { events: 0, sessions: 0, retired_events: 0, last_seen_label: String::new() });
            usage.events = usage.events.saturating_add(total.events);
            usage.retired_events = total.events;
            if let Some(last) = total.last_seen {
                let last = time_label(&last);
                if last > usage.last_seen_label { usage.last_seen_label = last; }
            }
        }
        let usage = rows.into_iter().filter(|r| r.resource.source.is_self()).map(|resource| {
            let usage = counts.remove(&resource.resource.id); UsageView { resource, usage }
        }).collect();
        let mut findings = Vec::new();
        for finding in context.state.get::<Vec<Finding>>("findings")? {
            let proposals = proposals_for(&context.state, &finding.resource_id)?;
            findings.push(FindingView { finding, proposals });
        }
        let health = crate::diagnostics::health(&context.state)?;
        render(&context, "activity.html", &ActivityPage {
            base, events, findings, usage, recording_errors: health.parsing_errors,
            pending_raw_bytes: health.pending_raw_bytes, pending_observations: health.pending_observations,
        })
    }).await
}
#[derive(Serialize)]
struct RunView {
    #[serde(flatten)]
    record: RunRecord,
    label: String,
    time_label: String,
    failed: bool,
}
#[derive(Serialize)]
struct JobsPage {
    #[serde(flatten)]
    base: BaseContext,
    runs: Vec<RunView>,
    collectors: BTreeMap<String, String>,
    health: crate::diagnostics::Health,
    last_success: Option<String>,
}
async fn jobs(WebState(context): WebState<Shared>) -> WebResult<Html<String>> {
    blocking(move || {
        let rows = resource_rows(&context.state)?;
        let base = base(&context, "jobs", &rows)?;
        let records: Vec<RunRecord> = context.state.get("runs")?;
        let runs = records
            .into_iter()
            .rev()
            .map(|record| {
                let label = match record.operation.as_str() {
                    "sync" => "资源同步",
                    "cycle" => "夜间处理",
                    "init" => "初始化",
                    other => other,
                }
                .to_owned();
                let failed = record.result.get("error").is_some()
                    || record
                        .result
                        .get("run")
                        .and_then(|r| r.get("status"))
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|s| matches!(s, "failed" | "interrupted"))
                    || record.result.get("ok") == Some(&serde_json::Value::Bool(false))
                    || record
                        .result
                        .get("results")
                        .and_then(serde_json::Value::as_object)
                        .is_some_and(|results| {
                            results.values().any(|r| {
                                r.get("status")
                                    .and_then(serde_json::Value::as_str)
                                    .is_some_and(|status| matches!(status, "failed" | "conflict"))
                            })
                        });
                let time_label = time_label(&record.time);
                RunView {
                    record,
                    label,
                    time_label,
                    failed,
                }
            })
            .collect();
        let health = crate::diagnostics::health(&context.state)?;
        let last_success = crate::diagnostics::latest_success(&health).map(time_label);
        render(
            &context,
            "jobs.html",
            &JobsPage {
                base,
                runs,
                collectors: context.state.get("collector_status")?,
                health,
                last_success,
            },
        )
    })
    .await
}
async fn download_diagnostics(WebState(context): WebState<Shared>) -> WebResult<Response> {
    blocking(move || {
        let report = crate::diagnostics::collect(&context.runtime, &context.state)?;
        Ok((
            [
                ("content-type", "application/json; charset=utf-8"),
                (
                    "content-disposition",
                    "attachment; filename=sm-diagnostics.json",
                ),
                ("cache-control", "no-store"),
            ],
            crate::diagnostics::bytes(&report)?,
        )
            .into_response())
    })
    .await
}
#[derive(Serialize)]
struct ProposalResponse {
    status: &'static str,
    key: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstallationChange {
    enabled: bool,
    reason: String,
}
async fn propose_installation(
    WebState(context): WebState<Shared>,
    WebPath(id): WebPath<String>,
    body: std::result::Result<Json<InstallationChange>, JsonRejection>,
) -> WebResult<Response> {
    let Json(request) = body.map_err(WebError::bad)?;
    blocking(move || {
        let mut catalog: Catalog = context.state.get("catalog")?;
        let resource = catalog
            .resources
            .iter_mut()
            .find(|resource| resource.id == id)
            .ok_or_else(|| WebError::bad("未知的原生安装"))?;
        let ResourceContent::Installation { installation } = &mut resource.content else {
            return Err(WebError::bad("该资源不是原生安装策略"));
        };
        installation.enabled = request.enabled;
        let change = ChangeSet {
            resource_ids: vec![id],
            reason: request.reason,
            evidence: Vec::new(),
            writes: BTreeMap::from([(
                "catalog.json".into(),
                serde_json::to_string_pretty(&catalog)? + "\n",
            )]),
            deletions: Vec::new(),
            renames: BTreeMap::new(),
            validation: Default::default(),
        };
        submit_change(&context, &change)
    })
    .await
}
async fn propose(
    WebState(context): WebState<Shared>,
    body: std::result::Result<Json<ChangeSet>, JsonRejection>,
) -> WebResult<Response> {
    let Json(change) = body.map_err(|error| WebError {
        status: if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        },
        message: redact_text(&error.body_text(), Some(500)),
    })?;
    blocking(move || submit_change(&context, &change)).await
}
fn submit_change(context: &WebContext, change: &ChangeSet) -> WebResult<Response> {
    let _guard = JobGuard::acquire(context.state.directory()?).map_err(|error| WebError {
        status: if error.is::<Busy>() {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        },
        message: redact_text(&error.to_string(), Some(500)),
    })?;
    let mut proposal =
        crate::proposals::create_proposal(&context.runtime.repo, change, &context.state)
            .map_err(WebError::bad)?;
    match context.submitter.submit(&mut proposal, &context.state) {
        Ok(url) => Ok((
            StatusCode::OK,
            Json(ProposalResponse {
                status: "submitted",
                key: proposal.key,
                message: "MR 已提交，审核合并后将在下次同步应用。".into(),
                url: Some(url),
            }),
        )
            .into_response()),
        Err(error) => Ok((
            StatusCode::ACCEPTED,
            Json(ProposalResponse {
                status: "draft",
                key: proposal.key,
                message: format!(
                    "本地提案已保存，远端提交未完成：{}",
                    redact_text(&format!("{error:#}"), Some(500))
                ),
                url: None,
            }),
        )
            .into_response()),
    }
}
pub async fn serve(runtime: Runtime, port: u16) -> Result<()> {
    let state = runtime.state()?;
    let app = router(runtime.clone(), state.clone())?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("SM: http://{}", listener.local_addr()?);
    let initial = tokio::task::spawn_blocking(move || synchronize(&runtime, &state));
    tokio::spawn(async move {
        match initial.await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => eprintln!(
                "SM initial sync failed: {}",
                redact_text(&error.to_string(), Some(500))
            ),
            Err(error) => eprintln!("SM initial sync worker failed: {error}"),
        }
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("SM shutdown signal unavailable: {error}");
            }
        })
        .await?;
    Ok(())
}
