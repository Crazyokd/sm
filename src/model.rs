use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer},
    ser::SerializeMap,
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
    Hook,
    Rule,
    Agent,
    Mcp,
    Prompt,
    Policy,
}
impl ResourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Skill => "Skill",
            Self::Hook => "Hook",
            Self::Rule => "规则",
            Self::Agent => "Agent 定义",
            Self::Mcp => "MCP 配置",
            Self::Prompt => "提示词",
            Self::Policy => "执行策略",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePayload {
    pub agents: Vec<Client>,
    pub payloads: BTreeMap<Client, PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileProjection {
    pub root: PathBuf,
    pub entries: Vec<ProjectionEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ProjectionEntry {
    File {
        target: PathBuf,
        source: PathBuf,
    },
    Fragment {
        target: PathBuf,
        fragment: PathBuf,
    },
    Tree {
        target: PathBuf,
        tree: PathBuf,
    },
    Json {
        target: PathBuf,
        json: Map<String, Value>,
    },
    Toml {
        target: PathBuf,
        toml: Map<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationPolicy {
    pub agent: Client,
    pub source: String,
    pub selector: String,
    pub enabled: bool,
    pub paths: Vec<PathBuf>,
    pub update: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceContent {
    Skill {
        payload: PathBuf,
    },
    Files {
        kind: ResourceKind,
        files: FileProjection,
    },
    Installation {
        installation: InstallationPolicy,
    },
    Hook(NativePayload),
    Rule(NativePayload),
    Agent(NativePayload),
    Mcp(NativePayload),
}
impl ResourceContent {
    pub fn kind(&self) -> ResourceKind {
        match self {
            Self::Skill { .. } => ResourceKind::Skill,
            Self::Files { kind, .. } => *kind,
            Self::Installation { .. } => ResourceKind::Policy,
            Self::Hook(_) => ResourceKind::Hook,
            Self::Rule(_) => ResourceKind::Rule,
            Self::Agent(_) => ResourceKind::Agent,
            Self::Mcp(_) => ResourceKind::Mcp,
        }
    }
}
impl Serialize for ResourceContent {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut fields = serializer.serialize_map(None)?;
        fields.serialize_entry("kind", &self.kind())?;
        match self {
            Self::Skill { payload } => fields.serialize_entry("payload", payload)?,
            Self::Files { files, .. } => fields.serialize_entry("files", files)?,
            Self::Installation { installation } => {
                fields.serialize_entry("installation", installation)?
            }
            Self::Hook(native) | Self::Rule(native) | Self::Agent(native) | Self::Mcp(native) => {
                fields.serialize_entry("agents", &native.agents)?;
                fields.serialize_entry("payloads", &native.payloads)?;
            }
        }
        fields.end()
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
    Upstream { upstream: String },
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResource {
    id: String,
    kind: ResourceKind,
    origin: String,
    scope: Scope,
    project: Option<String>,
    payload: Option<PathBuf>,
    files: Option<FileProjection>,
    installation: Option<InstallationPolicy>,
    #[serde(default)]
    agents: Vec<Client>,
    #[serde(default)]
    payloads: BTreeMap<Client, PathBuf>,
    upstream: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
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
            raw.id != "source-providers" && raw.id != "dependencies",
            "resource id is reserved for shared infrastructure"
        );
        let dependencies: BTreeSet<_> = raw.dependencies.iter().collect();
        ensure!(
            dependencies.len() == raw.dependencies.len()
                && dependencies.iter().all(|name| valid_id(name)),
            "invalid or duplicate resource dependency"
        );
        ensure!(
            match raw.scope {
                Scope::User => raw.project.is_none(),
                Scope::Project => raw.project.as_deref().is_some_and(|p| !p.is_empty()),
            },
            "scope and project do not match"
        );
        let content = if let Some(files) = raw.files {
            ensure!(
                raw.payload.is_none()
                    && raw.installation.is_none()
                    && raw.agents.is_empty()
                    && raw.payloads.is_empty(),
                "file projection cannot declare native payloads or installation"
            );
            crate::projections::validate(raw.kind, &files, raw.scope)?;
            if raw.origin == "self" {
                let directory = Path::new("resources").join(&raw.id);
                ensure!(
                    files.root.starts_with(&directory),
                    "self projection root must be resource specific"
                );
                let payloads = directory.join("files");
                for entry in &files.entries {
                    match entry {
                        ProjectionEntry::File { source, .. } => {
                            let path = files.root.join(source);
                            ensure!(
                                path.starts_with(&payloads) && path != payloads,
                                "self projection files must be under resources/<id>/files"
                            );
                        }
                        ProjectionEntry::Tree { tree, .. } => ensure!(
                            files.root.join(tree).starts_with(&payloads),
                            "self Skill tree must be under resources/<id>/files"
                        ),
                        _ => {}
                    }
                }
            }
            ResourceContent::Files {
                kind: raw.kind,
                files,
            }
        } else if let Some(installation) = raw.installation {
            ensure!(
                raw.dependencies.is_empty(),
                "native installation policy cannot declare supporting files"
            );
            ensure!(
                raw.kind == ResourceKind::Policy
                    && raw.origin == "self"
                    && raw.scope == Scope::User,
                "installation must be a self-owned user policy"
            );
            ensure!(
                raw.payload.is_none() && raw.agents.is_empty() && raw.payloads.is_empty(),
                "installation cannot declare payloads"
            );
            ResourceContent::Installation { installation }
        } else if raw.kind == ResourceKind::Skill {
            ensure!(
                raw.agents.is_empty() && raw.payloads.is_empty(),
                "Skill cannot have client classification"
            );
            ResourceContent::Skill {
                payload: raw.payload.context("Skill requires one payload")?,
            }
        } else {
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
            match raw.kind {
                ResourceKind::Hook => ResourceContent::Hook(native),
                ResourceKind::Rule => ResourceContent::Rule(native),
                ResourceKind::Agent => ResourceContent::Agent(native),
                ResourceKind::Mcp => ResourceContent::Mcp(native),
                ResourceKind::Prompt | ResourceKind::Policy => {
                    bail!("resource requires a file projection or installation policy")
                }
                ResourceKind::Skill => unreachable!("handled above"),
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
                ensure!(valid_id(&upstream), "invalid upstream source id");
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
            dependencies: raw.dependencies,
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
            ResourceContent::Skill { payload } => Client::ALL
                .into_iter()
                .map(|c| (c, payload.clone()))
                .collect(),
            ResourceContent::Files { files, .. } => Client::ALL
                .into_iter()
                .map(|c| (c, files.root.clone()))
                .collect(),
            ResourceContent::Installation { .. } => BTreeMap::new(),
            ResourceContent::Hook(p)
            | ResourceContent::Rule(p)
            | ResourceContent::Agent(p)
            | ResourceContent::Mcp(p) => p.payloads.clone(),
        }
    }
    pub fn paths(&self) -> BTreeSet<PathBuf> {
        match &self.content {
            ResourceContent::Files { files, .. } => files
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    ProjectionEntry::File { source, .. }
                    | ProjectionEntry::Tree { tree: source, .. }
                    | ProjectionEntry::Fragment {
                        fragment: source, ..
                    } => Some(files.root.join(source)),
                    _ => None,
                })
                .collect(),
            _ => self.targets().into_values().collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SupportingFiles {
    #[serde(flatten)]
    pub source: Source,
    pub files: FileProjection,
}
impl<'de> Deserialize<'de> for SupportingFiles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            origin: String,
            upstream: Option<String>,
            files: FileProjection,
        }
        let raw = Raw::deserialize(deserializer)?;
        let source = match (raw.origin.as_str(), raw.upstream) {
            ("self", None) => Source::SelfOwned,
            ("upstream", Some(upstream)) if valid_id(&upstream) => Source::Upstream { upstream },
            _ => return Err(de::Error::custom("invalid supporting file source")),
        };
        crate::projections::validate_support(&raw.files).map_err(de::Error::custom)?;
        Ok(Self {
            source,
            files: raw.files,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub projects: BTreeMap<String, String>,
    #[serde(default)]
    pub upstreams: BTreeMap<String, UpstreamSource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, SupportingFiles>,
    pub resources: Vec<Resource>,
}
impl Catalog {
    pub fn load(root: &Path) -> Result<Self> {
        let mut catalog: Self = serde_json::from_slice(&fs::read(root.join("catalog.json"))?)?;
        let mut ids = BTreeSet::new();
        for (name, upstream) in &catalog.upstreams {
            ensure!(valid_id(name), "invalid upstream source id");
            match upstream {
                UpstreamSource::Git { git } => ensure!(
                    !git.is_empty() && !git.contains('\0'),
                    "empty upstream Git source"
                ),
                UpstreamSource::Generator { argv } => ensure!(
                    !argv.is_empty()
                        && !argv[0].is_empty()
                        && argv.iter().all(|arg| !arg.contains('\0')),
                    "empty upstream generator"
                ),
            }
        }
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
        let mut referenced = BTreeSet::new();
        for resource in &catalog.resources {
            for dependency in &resource.dependencies {
                ensure!(
                    catalog.dependencies.contains_key(dependency),
                    "unknown resource dependency: {dependency}"
                );
                referenced.insert(dependency.clone());
            }
        }
        ensure!(
            referenced.len() == catalog.dependencies.len(),
            "unreferenced supporting file dependency"
        );
        for (name, dependency) in &catalog.dependencies {
            ensure!(valid_id(name), "invalid dependency id");
            crate::projections::validate_support(&dependency.files)?;
            match &dependency.source {
                Source::SelfOwned => ensure!(
                    dependency
                        .files
                        .root
                        .starts_with(Path::new("resources/dependencies").join(name).join("files")),
                    "self dependency root must be resources/dependencies/<name>/files"
                ),
                Source::Upstream { upstream } => ensure!(
                    catalog.upstreams.contains_key(upstream),
                    "unknown dependency upstream source: {upstream}"
                ),
            }
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
            if let Source::Upstream { upstream } = &resource.source {
                ensure!(
                    catalog.upstreams.contains_key(upstream),
                    "unknown upstream source: {upstream}"
                );
            }
            if resource.source.is_self() {
                for path in resource.paths() {
                    let payload = contained(root, &path)?;
                    let exists = match &resource.content {
                        ResourceContent::Files { files, .. } => {
                            crate::files::reject_symlink_path(&payload)?;
                            if files.entries.iter().any(|entry| matches!(entry, ProjectionEntry::Tree { tree, .. } if files.root.join(tree) == path)) {
                                payload.is_dir()
                            } else {
                                payload.is_file()
                            }
                        }
                        _ => payload.is_dir(),
                    };
                    ensure!(exists, "missing resource payload: {}", path.display());
                }
            }
        }
        crate::installations::load(root)?;
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
