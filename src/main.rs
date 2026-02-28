//! AgentMint: cryptographic proof of human authorization for AI agent actions.

pub mod audit;
pub mod console;
pub mod error;
pub mod handlers;
pub mod jti;
pub mod oidc;
pub mod policy;
pub mod ratelimit;
pub mod server;
pub mod state;
pub mod telemetry;
pub mod token;
pub mod webauthn;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    console::print_banner();

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "agentmint starting");

    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let state = state::build_state("agentmint.db")?;

    tracing::info!(bind = %addr, jti_capacity = 100_000, max_ttl = 300, "config");
    console::print_startup(&addr);

    server::run(state, &addr).await?;
    Ok(())
}
