//! mint.run CLI: init, serve, verify, doctor, lab, and local overhead bench.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use mint_run::bench;
use mint_run::config::Config;
use mint_run::doctor;
use mint_run::domain::SignedReceipt;
use mint_run::keys::KeyRing;
use mint_run::lab;
use mint_run::receipt::{verify_receipt, verify_with_public_key, verifying_key_from_jwk_x};
use mint_run::server::{self, build_state};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }
    match args[0].as_str() {
        "init" => {
            cmd_init(&args[1..])?;
            Ok(ExitCode::SUCCESS)
        }
        "serve" => {
            cmd_serve().await?;
            Ok(ExitCode::SUCCESS)
        }
        "verify" => {
            cmd_verify(&args[1..]).await?;
            Ok(ExitCode::SUCCESS)
        }
        "doctor" => {
            let config = Config::from_env()?;
            let results = doctor::run_doctor(&config).await;
            doctor::print_results(&results);
            Ok(ExitCode::from(doctor::exit_code(&results) as u8))
        }
        "bench" => {
            init_tracing(false);
            let iterations = parse_flag(&args[1..], "--iterations")
                .and_then(|v| v.parse().ok())
                .unwrap_or(200);
            print!("{}", bench::run_bench(iterations).await?);
            Ok(ExitCode::SUCCESS)
        }
        "lab" => lab::cli::run(&args[1..]),
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            Ok(ExitCode::from(2))
        }
    }
}

fn print_help() {
    println!(
        "mint.run — consequential agent actions that are safe to retry\n\n\
         Usage:\n\
         \tmint init [--key-file PATH]\n\
         \tmint doctor\n\
         \tmint serve\n\
         \tmint verify --receipt PATH [--key-file PATH | --keys-url URL]\n\
         \tmint bench [--iterations N]\n\
         \tmint lab <start|console|inspect|events|advance|fault|check|run|list> ...\n\n\
         Environment variables use the MINT_ prefix. Local identity and the fake\n\
         provider are development-only. Live Stripe secrets are refused.\n\
         `mint lab` is an experimental synthetic PA/BV case console (no real PHI).\n"
    );
}

fn cmd_init(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let path = parse_flag(args, "--key-file")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("mint.ed25519.pem"));
    let mut ring = KeyRing::generate();
    ring.kid = env::var("MINT_KID").unwrap_or_else(|_| "mint-local-1".into());
    ring.write_pkcs8_pem(&path)?;
    println!("wrote signing key {}", path.display());
    println!("kid={}", ring.kid);
    println!("set MINT_SIGNING_KEY_FILE to this path before `mint serve` or `mint doctor`");
    Ok(())
}

async fn cmd_serve() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env()?;
    config.validate_relationships()?;
    init_tracing(config.log_format_json);
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        product = "mint.run",
        mode = ?config.mode,
        provider = ?config.provider,
        "starting"
    );
    let addr = config.bind_addr.clone();
    let state = build_state(config)?;
    state.identity.warmup().await?;
    server::run(state, &addr).await?;
    Ok(())
}

async fn cmd_verify(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let receipt_path = parse_flag(args, "--receipt").ok_or("missing --receipt")?;
    let raw = fs::read_to_string(receipt_path)?;
    let receipt: SignedReceipt = serde_json::from_str(&raw)?;
    if let Some(url) = parse_flag(args, "--keys-url") {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let keys: serde_json::Value = http.get(url).send().await?.json().await?;
        let Some(list) = keys.get("keys").and_then(|v| v.as_array()) else {
            return Err("keys url did not return keys[]".into());
        };
        let jwk = list
            .iter()
            .find(|k| k.get("kid").and_then(|v| v.as_str()) == Some(receipt.kid.as_str()))
            .ok_or("unknown key id")?;
        let x = jwk.get("x").and_then(|v| v.as_str()).ok_or("missing x")?;
        let verifying = verifying_key_from_jwk_x(x)?;
        verify_with_public_key(&receipt, &receipt.kid, &verifying)?;
    } else {
        let key_file = parse_flag(args, "--key-file")
            .map(PathBuf::from)
            .or_else(|| env::var("MINT_SIGNING_KEY_FILE").ok().map(PathBuf::from))
            .ok_or("missing --key-file or --keys-url")?;
        let kid = env::var("MINT_KID").unwrap_or_else(|_| receipt.kid.clone());
        let keys = KeyRing::from_config(&kid, Some(Path::new(&key_file)), None)?;
        verify_receipt(&receipt, &keys)?;
    }
    println!(
        "valid receipt action_id={} status={:?}",
        receipt.payload.action_id, receipt.payload.status
    );
    Ok(())
}

fn parse_flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn init_tracing(json: bool) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .flatten_event(true)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}
