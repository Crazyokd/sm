use crate::{
    model::{ApplyStatus, Catalog, Client, Resource, ResourceKind, ResourceResult, Scope},
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
use std::{collections::BTreeMap, path::Path, sync::Arc};
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
    client_label: String,
    status: ApplyStatus,
    status_label: &'static str,
    status_message: String,
}
fn resource_rows(state: &State) -> Result<Vec<ResourceView>> {
    let resources = state.get::<Catalog>("catalog")?.resources;
    let results: BTreeMap<String, ResourceResult> = state.get("resource_results")?;
    Ok(resources
        .into_iter()
        .map(|resource| {
            let result = results.get(&resource.id);
            let status = result.map_or(ApplyStatus::Unknown, |r| r.status);
            let kind = resource.content.kind();
            let client_label = if kind == ResourceKind::Skill {
                "通用 Skill".into()
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
            ResourceView {
                resource,
                upstream_path,
                kind_label: kind.label(),
                client_label,
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
                        .is_none_or(|client| r.resource.targets().contains_key(&client))
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
struct DetailPage {
    #[serde(flatten)]
    base: BaseContext,
    resource: ResourceView,
    files: BTreeMap<String, String>,
    articles: Vec<Article>,
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
        let mut articles = Vec::new();
        for (name, text) in &files {
            let body = if name.ends_with(".md") && text.starts_with("---\n") {
                text.splitn(3, "---").nth(2).unwrap_or(text)
            } else {
                text
            };
            articles.push(Article {
                title: Path::new(name)
                    .file_name()
                    .map_or_else(|| name.clone(), |n| n.to_string_lossy().into_owned()),
                html: if name.ends_with(".md") {
                    Some(markdown(body))
                } else {
                    None
                },
                text: text.clone(),
            });
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
    excerpt: String,
}
#[derive(Serialize)]
struct Usage {
    events: u64,
    sessions: u64,
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
}
async fn activity(WebState(context): WebState<Shared>) -> WebResult<Html<String>> {
    blocking(move||{
    let rows=resource_rows(&context.state)?;let base=base(&context,"activity",&rows)?;let db=context.state.connect()?;let mut query=db.prepare("SELECT value FROM observations ORDER BY rowid DESC LIMIT 100")?;let mut events=Vec::new();
    for text in query.query_map([],|row|row.get::<_,String>(0))?{let row:Observation=serde_json::from_str(&text?)?;let input=row.facts.get("tool_input");let excerpt=input.and_then(|i|i.get("command").or_else(||i.get("path"))).and_then(serde_json::Value::as_str).unwrap_or_default();events.push(EventView{id:row.id,time_label:time_label(&row.timestamp),agent:row.agent,resource_id:row.resource_id,event:row.event,coverage:row.coverage,excerpt:redact_text(excerpt,Some(180))});}
    let mut counts=BTreeMap::new();let mut query=db.prepare("SELECT json_extract(value,'$.resource_id'),COUNT(*),COUNT(DISTINCT NULLIF(json_extract(value,'$.session_id'),'')),MAX(json_extract(value,'$.timestamp')) FROM observations GROUP BY 1")?;
    for row in query.query_map([],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,u64>(1)?,row.get::<_,u64>(2)?,row.get::<_,String>(3)?)))?{let(id,events,sessions,last_seen)=row?;if let Some(id)=id{counts.insert(id,Usage{events,sessions,last_seen_label:time_label(&last_seen)});}}
    let usage=rows.into_iter().filter(|r|r.resource.source.is_self()).map(|resource|{let usage=counts.remove(&resource.resource.id);UsageView{resource,usage}}).collect();
    let mut findings=Vec::new();for finding in context.state.get::<Vec<Finding>>("findings")?{let proposals=proposals_for(&context.state,&finding.resource_id)?;findings.push(FindingView{finding,proposals});}
    render(&context,"activity.html",&ActivityPage{base,events,findings,usage,recording_errors:context.state.get("recording_errors")?})
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
        render(
            &context,
            "jobs.html",
            &JobsPage {
                base,
                runs,
                collectors: context.state.get("collector_status")?,
            },
        )
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
    blocking(move || {
        let _guard = JobGuard::acquire(context.state.directory()?).map_err(|error| WebError {
            status: if error.is::<Busy>() {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            },
            message: redact_text(&error.to_string(), Some(500)),
        })?;
        let mut proposal =
            crate::proposals::create_proposal(&context.runtime.repo, &change, &context.state)
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
    })
    .await
}
pub async fn serve(runtime: Runtime, port: u16) -> Result<()> {
    let state = runtime.state()?;
    let app = router(runtime.clone(), state.clone())?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    eprintln!("SM: http://{}", listener.local_addr()?);
    let initial = tokio::task::spawn_blocking(move || synchronize(&runtime, &state));
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("SM shutdown signal unavailable: {error}");
            }
        })
        .await?;
    match initial.await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => eprintln!(
            "SM initial sync failed: {}",
            redact_text(&error.to_string(), Some(500))
        ),
        Err(error) => eprintln!("SM initial sync worker failed: {error}"),
    };
    Ok(())
}
