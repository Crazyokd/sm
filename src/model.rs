use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer},
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use crate::files::{contained, relative_path};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum Client {
    Codex,
    Kimi,
}
impl Client {
    pub const ALL: [Self; 2] = [Self::Codex, Self::Kimi];
}
impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Codex => "codex",
            Self::Kimi => "kimi",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    Skill,
    Bundle,
    Hook,
    Rule,
    Agent,
    Mcp,
}
impl ResourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Skill => "Skill",
            Self::Bundle => "资源包",
            Self::Hook => "Hook",
            Self::Rule => "规则",
            Self::Agent => "Agent 定义",
            Self::Mcp => "MCP 配置",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePayload {
    pub agents: Vec<Client>,
    pub payloads: BTreeMap<Client, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ResourceContent {
    Skill { payload: PathBuf },
    Bundle { payload: PathBuf },
    Hook(NativePayload),
    Rule(NativePayload),
    Agent(NativePayload),
    Mcp(NativePayload),
}
impl ResourceContent {
    pub fn kind(&self) -> ResourceKind {
        match self {
            Self::Skill { .. } => ResourceKind::Skill,
            Self::Bundle { .. } => ResourceKind::Bundle,
            Self::Hook(_) => ResourceKind::Hook,
            Self::Rule(_) => ResourceKind::Rule,
            Self::Agent(_) => ResourceKind::Agent,
            Self::Mcp(_) => ResourceKind::Mcp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum UpstreamSource {
    Git { git: String },
    Generator { argv: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "origin")]
pub enum Source {
    #[serde(rename = "self")]
    SelfOwned,
    #[serde(rename = "upstream")]
    Upstream { upstream: UpstreamSource },
}
impl Source {
    pub fn is_self(&self) -> bool {
        matches!(self, Self::SelfOwned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resource {
    pub id: String,
    #[serde(flatten)]
    pub content: ResourceContent,
    #[serde(flatten)]
    pub source: Source,
    pub scope: Scope,
    pub project: Option<String>,
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Deserialize)]
struct RawResource {
    id: String,
    kind: ResourceKind,
    origin: String,
    scope: Scope,
    project: Option<String>,
    payload: Option<PathBuf>,
    #[serde(default)]
    agents: Vec<Client>,
    #[serde(default)]
    payloads: BTreeMap<Client, PathBuf>,
    upstream: Option<UpstreamSource>,
}
impl<'de> Deserialize<'de> for Resource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = RawResource::deserialize(deserializer)?;
        Self::try_from(raw).map_err(de::Error::custom)
    }
}

pub fn valid_id(id: &str) -> bool {
    static PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]*$").expect("fixed resource ID regex"));
    PATTERN.is_match(id)
}
impl TryFrom<RawResource> for Resource {
    type Error = anyhow::Error;
    fn try_from(raw: RawResource) -> Result<Self> {
        ensure!(valid_id(&raw.id), "invalid resource id");
        ensure!(
            match raw.scope {
                Scope::User => raw.project.is_none(),
                Scope::Project => raw.project.as_deref().is_some_and(|p| !p.is_empty()),
            },
            "scope and project do not match"
        );
        let content = match raw.kind {
            kind @ (ResourceKind::Skill | ResourceKind::Bundle) => {
                ensure!(
                    raw.agents.is_empty() && raw.payloads.is_empty(),
                    "{} cannot have client classification",
                    kind.label()
                );
                let payload = raw
                    .payload
                    .context("shared resource requires one payload")?;
                if kind == ResourceKind::Skill {
                    ResourceContent::Skill { payload }
                } else {
                    ResourceContent::Bundle { payload }
                }
            }
            kind => {
                ensure!(
                    raw.payload.is_none(),
                    "native resource cannot have a shared payload field"
                );
                let agents: BTreeSet<_> = raw.agents.iter().copied().collect();
                ensure!(
                    !agents.is_empty()
                        && agents.len() == raw.agents.len()
                        && agents == raw.payloads.keys().copied().collect(),
                    "payloads must match unique clients"
                );
                let native = NativePayload {
                    agents: raw.agents,
                    payloads: raw.payloads,
                };
                match kind {
                    ResourceKind::Hook => ResourceContent::Hook(native),
                    ResourceKind::Rule => ResourceContent::Rule(native),
                    ResourceKind::Agent => ResourceContent::Agent(native),
                    ResourceKind::Mcp => ResourceContent::Mcp(native),
                    ResourceKind::Skill | ResourceKind::Bundle => unreachable!("handled above"),
                }
            }
        };
        let source = match raw.origin.as_str() {
            "self" => {
                ensure!(
                    raw.upstream.is_none(),
                    "self resource cannot declare upstream"
                );
                Source::SelfOwned
            }
            "upstream" => {
                let upstream = raw.upstream.context("upstream source required")?;
                match &upstream {
                    UpstreamSource::Git { git } => {
                        ensure!(!git.is_empty(), "empty upstream Git source")
                    }
                    UpstreamSource::Generator { argv } => ensure!(
                        !argv.is_empty() && !argv[0].is_empty(),
                        "empty upstream generator"
                    ),
                }
                Source::Upstream { upstream }
            }
            _ => bail!("invalid resource origin"),
        };
        let resource = Self {
            id: raw.id,
            content,
            source,
            scope: raw.scope,
            project: raw.project,
            root: PathBuf::new(),
        };
        for path in resource.targets().values() {
            relative_path(path)?;
            if resource.source.is_self() {
                ensure!(
                    path.starts_with("resources") && path != Path::new("resources"),
                    "self payload must be under resources/"
                );
            }
        }
        Ok(resource)
    }
}
impl Resource {
    pub fn targets(&self) -> BTreeMap<Client, PathBuf> {
        match &self.content {
            ResourceContent::Skill { payload } | ResourceContent::Bundle { payload } => Client::ALL
                .into_iter()
                .map(|c| (c, payload.clone()))
                .collect(),
            ResourceContent::Hook(p)
            | ResourceContent::Rule(p)
            | ResourceContent::Agent(p)
            | ResourceContent::Mcp(p) => p.payloads.clone(),
        }
    }
    pub fn paths(&self) -> BTreeSet<PathBuf> {
        self.targets().into_values().collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub projects: BTreeMap<String, String>,
    pub resources: Vec<Resource>,
}
impl Catalog {
    pub fn load(root: &Path) -> Result<Self> {
        let mut catalog: Self = serde_json::from_slice(&fs::read(root.join("catalog.json"))?)?;
        let mut ids = BTreeSet::new();
        for (name, path) in &catalog.projects {
            ensure!(
                !name.is_empty()
                    && (path.starts_with("~/") || Path::new(path).is_absolute())
                    && !Path::new(path)
                        .components()
                        .any(|c| c == std::path::Component::ParentDir),
                "invalid project directory"
            );
        }
        for resource in &mut catalog.resources {
            ensure!(ids.insert(resource.id.clone()), "duplicate resource id");
            resource.root = root.to_path_buf();
            if resource.scope == Scope::Project {
                ensure!(
                    resource
                        .project
                        .as_ref()
                        .is_some_and(|p| catalog.projects.contains_key(p)),
                    "unknown project"
                );
            }
            if resource.source.is_self() {
                for path in resource.paths() {
                    ensure!(
                        contained(root, &path)?.is_dir(),
                        "missing resource payload: {}",
                        path.display()
                    );
                }
            }
        }
        Ok(catalog)
    }
    pub fn scope_root(&self, resource: &Resource, home: &Path) -> Result<PathBuf> {
        match resource.scope {
            Scope::User => Ok(home.to_path_buf()),
            Scope::Project => {
                let project = resource
                    .project
                    .as_ref()
                    .context("project scope requires project")?;
                let path = self.projects.get(project).context("unknown project")?;
                Ok(path
                    .strip_prefix("~/")
                    .map(|p| home.join(p))
                    .unwrap_or_else(|| PathBuf::from(path)))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "content",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum WriteBody {
    File { bytes: Vec<u8>, executable: bool },
    Markdown(Vec<u8>),
    Json(Map<String, Value>),
    Toml(Map<String, Value>),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedWrite {
    pub resource_id: String,
    pub path: PathBuf,
    pub body: WriteBody,
}
impl OwnedWrite {
    pub fn new(id: impl Into<String>, path: impl AsRef<Path>, body: WriteBody) -> Self {
        Self {
            resource_id: id.into(),
            path: path.as_ref().to_path_buf(),
            body,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "content",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum StoredBody {
    FileFingerprint { sha256: [u8; 32], executable: bool },
    Markdown(Vec<u8>),
    Json(Map<String, Value>),
    Toml(Map<String, Value>),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredWrite {
    pub resource_id: String,
    pub path: PathBuf,
    pub body: StoredBody,
}
impl From<&OwnedWrite> for StoredWrite {
    fn from(write: &OwnedWrite) -> Self {
        Self {
            resource_id: write.resource_id.clone(),
            path: write.path.clone(),
            body: match &write.body {
                WriteBody::File { bytes, executable } => StoredBody::FileFingerprint {
                    sha256: Sha256::digest(bytes).into(),
                    executable: *executable,
                },
                WriteBody::Markdown(bytes) => StoredBody::Markdown(bytes.clone()),
                WriteBody::Json(fragment) => StoredBody::Json(fragment.clone()),
                WriteBody::Toml(fragment) => StoredBody::Toml(fragment.clone()),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Pending,
    Applied,
    Unsupported,
    Conflict,
    Failed,
    Unknown,
}
impl ApplyStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "正在同步",
            Self::Applied => "文件已应用",
            Self::Unsupported => "暂不支持",
            Self::Conflict => "存在冲突",
            Self::Failed => "应用失败",
            Self::Unknown => "尚未应用",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceResult {
    pub status: ApplyStatus,
    pub message: String,
}
impl ResourceResult {
    pub fn new(status: ApplyStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct ApplyPlan {
    pub writes: Vec<OwnedWrite>,
    pub results: BTreeMap<String, ResourceResult>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub results: BTreeMap<String, ResourceResult>,
}
impl ApplyReport {
    pub fn ok(&self) -> bool {
        self.results
            .values()
            .all(|r| !matches!(r.status, ApplyStatus::Failed | ApplyStatus::Conflict))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationStatus {
    Passed,
    Failed,
    #[default]
    Partial,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Static,
    ApprovedFragmentCheck,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCheck {
    pub name: String,
    pub kind: CheckKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<bool>,
    pub after: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationReport {
    #[serde(default)]
    pub status: ValidationStatus,
    #[serde(default)]
    pub checks: Vec<ValidationCheck>,
    #[serde(default)]
    pub unverified: Vec<String>,
}
