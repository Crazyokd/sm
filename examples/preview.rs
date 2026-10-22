//! Isolated browser verification: preview <repo> <temporary-home> <port> [state.sqlite3].
use anyhow::{Context, Result};
use axum::{response::Html, routing::get};
use sm::{
    proposals::{Proposal, ProposalStatus},
    service::{Runtime, Submitter},
    state::State,
};
use std::{path::PathBuf, sync::Arc};
struct PreviewSubmitter(String);
impl Submitter for PreviewSubmitter {
    fn submit(&self, proposal: &mut Proposal, state: &State) -> Result<String> {
        let url = format!("{}/preview-mr/{}", self.0, proposal.key);
        proposal.mr_url = Some(url.clone());
        proposal.status = ProposalStatus::Submitted;
        proposal.save(state)?;
        Ok(url)
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let repo = PathBuf::from(args.first().context("repo required")?);
    let home = PathBuf::from(args.get(1).context("temporary home required")?);
    let port: u16 = args
        .get(2)
        .context("port required")?
        .to_str()
        .context("invalid port")?
        .parse()?;
    let executable = std::env::current_exe()?
        .parent()
        .and_then(|p| p.parent())
        .context("binary directory unavailable")?
        .join("sm");
    let runtime = Runtime {
        repo,
        home,
        executable,
    };
    let state = match args.get(3) {
        Some(path) => State::open(PathBuf::from(path))?,
        None => runtime.state()?,
    };
    sm::service::synchronize(&runtime, &state)?;
    let app = sm::web::router_with(
        runtime,
        state,
        Arc::new(PreviewSubmitter(format!("http://127.0.0.1:{port}"))),
    )?
    .route(
        "/preview-mr/{key}",
        get(|| async {
            Html("<h1>演示 MR · 未提交远端</h1><p>此页面仅供隔离浏览器验收；资源尚未应用。</p>")
        }),
    );
    eprintln!("Isolated preview: http://127.0.0.1:{port}");
    axum::serve(
        tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?,
        app,
    )
    .await?;
    Ok(())
}
