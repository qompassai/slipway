//! slipway — the estate's Android build / sign / verify /
//! install / release MCP server. rmcp, modern era only
//! (2026-07-28), from birth: no legacy era has ever existed in
//! this server. Configuration comes from `--config <path>` or
//! the `SLIPWAY_CONFIG` environment variable (a TOML document;
//! see `config.example.toml` in the build tree).
//!
//! Wiring (fleet template): the era guard owns stdio, the SDK
//! service sits behind an in-process duplex, and a pump copies
//! its responses back to stdout through the same lock the guard
//! writes with — the ordering discipline of the W1 amendment.

use std::process::ExitCode;
use std::sync::Arc;

use tokio::sync::Mutex;

use slipway::{config::Config, guard, server::SlipwayServer};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let config_path = args
        .windows(2)
        .find(|w| w[0] == "--config")
        .map(|w| w[1].clone())
        .or_else(|| std::env::var("SLIPWAY_CONFIG").ok());
    let Some(config_path) = config_path else {
        eprintln!("slipway: no config — pass --config <path> or set SLIPWAY_CONFIG");
        return ExitCode::from(2);
    };
    let config = match Config::load(std::path::Path::new(&config_path)) {
        Ok(config) => config,
        Err(failure) => {
            eprintln!(
                "slipway: config refused: [{}] {}",
                failure.code, failure.message
            );
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("slipway: cannot start runtime: {e}");
            return ExitCode::from(2);
        }
    };
    match runtime.block_on(run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("slipway: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run(config: Config) -> std::io::Result<()> {
    let (client_side, server_side) = tokio::io::duplex(64 * 1024);
    let (server_read, server_write) = tokio::io::split(server_side);
    let (client_read, client_write) = tokio::io::split(client_side);
    let stdout: guard::SharedStdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let ordering = guard::Ordering::new();
    let server = SlipwayServer::new(config);
    let serve = tokio::spawn(async move {
        server
            .run(server_read, server_write)
            .await
            .map_err(|e| std::io::Error::other(e.to_string()))
    });
    let mut pump = tokio::spawn(guard::pump(
        client_read,
        Arc::clone(&stdout),
        Arc::clone(&ordering),
    ));
    let guard_result = guard::run(tokio::io::stdin(), client_write, stdout, ordering).await;
    let serve_result = match serve.await {
        Ok(result) => result,
        Err(e) => Err(std::io::Error::other(e.to_string())),
    };
    if tokio::time::timeout(std::time::Duration::from_secs(2), &mut pump)
        .await
        .is_err()
    {
        pump.abort();
    }
    guard_result.and(serve_result)
}
