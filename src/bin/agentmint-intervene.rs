//! Loopback-only viewer for one recorded intervention run.
//! Used by: local inspection of a direct recorded run without a project picker.

use std::path::PathBuf;

use agentmint::aerf::intervention::Run;
use agentmint::intervene::{generate_url_token, run_with_listener_for_path};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/interventions/retry-after-v1-v2.json")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(fixture_path);
    let run = Run::from_slice(&std::fs::read(&path)?)?;
    let token = generate_url_token();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let repo_root = std::env::current_dir()?;

    println!("Serving recorded run from {}", path.display());
    println!("Open http://{addr}/{token}/run");

    run_with_listener_for_path(listener, run, &token, &path, &repo_root).await?;
    Ok(())
}
