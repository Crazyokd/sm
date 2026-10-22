use crate::{
    model::{Catalog, Client, Resource, ResourceResult},
    observe::record_event,
    privacy::redact_text,
    proposals::{ChangeSet, create_proposal},
    service::{Runtime, cycle, initialize, synchronize},
    state::RunRecord,
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
};
#[derive(Debug, Parser)]
#[command(name = "sm", version, about = "Local AI resource management")]
pub struct Args {
    #[arg(long, global = true)]
    pub repo: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}
#[derive(Debug, Subcommand)]
pub enum Commands {
    Init,
    Serve {
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
    Sync,
    Cycle,
    Status,
    Propose {
        change: PathBuf,
    },
    Record {
        #[arg(long, value_enum)]
        agent: Client,
        #[arg(long)]
        state_dir: PathBuf,
    },
}
#[derive(Debug)]
pub enum CommandOutcome {
    Exit(i32),
    Serve(u16),
}
fn print<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
pub fn record(agent: Client, directory: &std::path::Path) -> i32 {
    let result = || -> Result<()> {
        let mut raw = String::new();
        io::stdin().take(1048577).read_to_string(&mut raw)?;
        let payload = if raw.len() > 1048576 {
            json!({"hook_event_name":"CaptureGap","reason":"payload exceeds recorder limit"})
        } else {
            serde_json::from_str(&raw)?
        };
        record_event(agent, &payload, directory)
    };
    if let Err(error) = result() {
        eprintln!(
            "SM recording unavailable: {}",
            redact_text(&format!("{error:#}"), Some(400))
        );
    }
    0
}
pub fn run(args: Args, runtime: &Runtime) -> Result<CommandOutcome> {
    if let Commands::Record { agent, state_dir } = args.command {
        return Ok(CommandOutcome::Exit(record(agent, &state_dir)));
    }
    if let Commands::Serve { port } = args.command {
        return Ok(CommandOutcome::Serve(port));
    }
    let state = runtime.state()?;
    let exit = match args.command {
        Commands::Init => {
            let report = initialize(runtime)?;
            print(&report)?;
            i32::from(!report.ok)
        }
        Commands::Sync => {
            let report = synchronize(runtime, &state)?;
            print(&report)?;
            i32::from(!report.ok())
        }
        Commands::Cycle => {
            let report = cycle(runtime, &state)?;
            print(&report)?;
            i32::from(!report.ok)
        }
        Commands::Status => {
            #[derive(Serialize)]
            struct Status {
                resources: Vec<Resource>,
                results: std::collections::BTreeMap<String, ResourceResult>,
                runs: Vec<RunRecord>,
            }
            let mut runs: Vec<RunRecord> = state.get("runs")?;
            if runs.len() > 5 {
                runs.drain(..runs.len() - 5);
            }
            print(&Status {
                resources: state.get::<Catalog>("catalog")?.resources,
                results: state.get("resource_results")?,
                runs,
            })?;
            0
        }
        Commands::Propose { change } => {
            let change: ChangeSet = serde_json::from_slice(&fs::read(change)?)?;
            let _guard = crate::service::JobGuard::acquire(state.directory()?)?;
            let mut proposal = create_proposal(&runtime.repo, &change, &state)?;
            println!("{}", crate::gitlab::submit_mr(&mut proposal, &state)?);
            0
        }
        Commands::Serve { .. } | Commands::Record { .. } => unreachable!("handled above"),
    };
    Ok(CommandOutcome::Exit(exit))
}
