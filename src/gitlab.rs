use crate::{
    model::ValidationStatus,
    process::run_command,
    proposals::{Proposal, ProposalStatus},
    repository::git,
    state::State,
    wire::truncate,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{fs, path::Path, process::Command, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mr {
    pub iid: u64,
    pub web_url: String,
    pub source_branch: String,
    pub target_branch: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, deserialize_with = "nullable_description")]
    pub description: String,
}
fn nullable_description<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrEdit {
    pub title: String,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMr {
    pub source_branch: String,
    pub target_branch: String,
    pub remove_source_branch: bool,
    #[serde(flatten)]
    pub edit: MrEdit,
}
pub trait MrApi: Send + Sync {
    fn opened(&self, branch: &str) -> Result<Vec<Mr>>;
    fn create(&self, input: &CreateMr) -> Result<Mr>;
    fn update(&self, iid: u64, input: &MrEdit) -> Result<()>;
    fn get(&self, iid: u64) -> Result<Mr>;
}
pub fn project_location(repo: &Path) -> Result<(String, String)> {
    let remote = git(repo, &["remote", "get-url", "origin"])?;
    let (host, project) = if ["http://", "https://", "ssh://"]
        .iter()
        .any(|prefix| remote.starts_with(prefix))
    {
        let url = url::Url::parse(&remote)?;
        let host = url.host_str().context("GitLab hostname missing")?;
        let host = url
            .port()
            .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));
        (host, url.path().trim_start_matches('/').to_owned())
    } else {
        let (host, path) = remote
            .split_once(':')
            .context("MR submission requires a GitLab HTTP or SSH origin")?;
        ensure!(
            !host.contains('/') && !path.is_empty(),
            "invalid GitLab origin"
        );
        (
            host.rsplit('@')
                .next()
                .context("hostname missing")?
                .to_owned(),
            path.to_owned(),
        )
    };
    ensure!(
        !host.is_empty() && !project.is_empty(),
        "invalid GitLab project location"
    );
    Ok((
        host,
        project.strip_suffix(".git").unwrap_or(&project).to_owned(),
    ))
}
fn quoted(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}
pub struct Glab {
    host: String,
    endpoint: String,
}
impl Glab {
    pub fn for_repo(repo: &Path) -> Result<Self> {
        let (host, project) = project_location(repo)?;
        Ok(Self {
            host,
            endpoint: format!("projects/{}/merge_requests", quoted(&project)),
        })
    }
    fn call<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        body: Option<Value>,
        method: &str,
    ) -> Result<T> {
        let temporary = tempfile::tempdir()?;
        let mut command = Command::new("glab");
        command.args(["api", endpoint, "--hostname", &self.host]);
        if let Some(body) = body {
            let path = temporary.path().join("body.json");
            fs::write(&path, serde_json::to_vec(&body)?)?;
            command
                .args(["--method", method, "--input"])
                .arg(path)
                .args(["--header", "Content-Type: application/json"]);
        }
        serde_json::from_slice(&run_command(command, None, Duration::from_secs(120))?.stdout)
            .context("invalid GitLab response")
    }
}
impl MrApi for Glab {
    fn opened(&self, branch: &str) -> Result<Vec<Mr>> {
        self.call(
            &format!(
                "{}?state=opened&source_branch={}",
                self.endpoint,
                quoted(branch)
            ),
            None,
            "GET",
        )
    }
    fn create(&self, input: &CreateMr) -> Result<Mr> {
        self.call(&self.endpoint, Some(serde_json::to_value(input)?), "POST")
    }
    fn update(&self, iid: u64, input: &MrEdit) -> Result<()> {
        let _: Value = self.call(
            &format!("{}/{iid}", self.endpoint),
            Some(serde_json::to_value(input)?),
            "PUT",
        )?;
        Ok(())
    }
    fn get(&self, iid: u64) -> Result<Mr> {
        self.call(&format!("{}/{iid}", self.endpoint), None, "GET")
    }
}
pub fn submit_mr(proposal: &mut Proposal, state: &State) -> Result<String> {
    let api = Glab::for_repo(&proposal.directory)?;
    submit_mr_with(proposal, state, &api)
}
pub fn submit_mr_with(proposal: &mut Proposal, state: &State, api: &dyn MrApi) -> Result<String> {
    let existing = api.opened(&proposal.branch)?;
    git(&proposal.directory, &["push", "origin", &proposal.branch])?;
    let mut title = truncate(
        proposal.change.reason.lines().next().unwrap_or_default(),
        120,
    );
    if proposal.change.validation.status != ValidationStatus::Passed {
        title = format!("Draft: {title}");
    }
    let edit = MrEdit {
        title,
        description: proposal.description()?,
    };
    let mr = if let Some(existing) = existing.first() {
        api.update(existing.iid, &edit)?;
        existing.clone()
    } else {
        api.create(&CreateMr {
            source_branch: proposal.branch.clone(),
            target_branch: "main".into(),
            remove_source_branch: true,
            edit,
        })?
    };
    let confirmed = api.get(mr.iid)?;
    ensure!(
        confirmed.source_branch == proposal.branch && confirmed.target_branch == "main",
        "GitLab MR result does not match proposed branches"
    );
    let url = url::Url::parse(&confirmed.web_url)?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("invalid GitLab MR URL");
    }
    proposal.mr_url = Some(confirmed.web_url.clone());
    proposal.status = ProposalStatus::Submitted;
    proposal.save(state)?;
    Ok(confirmed.web_url)
}
