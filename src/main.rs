//! AgentMint: cryptographic proof of human authorization for AI agent actions.
//! Used by: binary entrypoint.

pub mod audit;
pub mod error;
pub mod handlers;
pub mod jti;
pub mod server;
pub mod state;
pub mod token;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let state = state::build_state("agentmint.db")?;
    let addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    tracing::info!("starting agentmint on {}", addr);

    server::run(state, &addr).await?;
    Ok(())
}
