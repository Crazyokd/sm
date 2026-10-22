#![forbid(unsafe_code)]
use anyhow::Result;
use clap::Parser;
use sm::{
    cli::{Args, CommandOutcome, Commands},
    service::Runtime,
};
use std::process::ExitCode;
fn run() -> Result<i32> {
    let args = Args::parse();
    if let Commands::Record { agent, state_dir } = &args.command {
        return Ok(sm::cli::record(*agent, state_dir));
    }
    let runtime = Runtime::discover(args.repo.as_deref())?;
    match sm::cli::run(args, &runtime)? {
        CommandOutcome::Exit(code) => Ok(code),
        CommandOutcome::Serve(port) => {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(sm::web::serve(runtime, port))?;
            Ok(0)
        }
    }
}
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "SM: {}",
                sm::privacy::redact_text(&format!("{error:#}"), Some(1200))
            );
            ExitCode::FAILURE
        }
    }
}
