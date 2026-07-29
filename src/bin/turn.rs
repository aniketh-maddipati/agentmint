//! turn CLI entrypoint.
//! Used by: local belief-aware agent loop runs.

use std::path::PathBuf;

use agentmint::turnrt::loop_::{run, RunOptions};
use agentmint::turnrt::tape::Tape;
use agentmint::turnrt::{engine::EngineConfig, tape::EventKind};

#[tokio::main]
async fn main() {
    if let Err(error) = dispatch().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let Some(command) = args.first().cloned() else {
        print_usage();
        return Ok(());
    };
    args.remove(0);

    match command.as_str() {
        "run" => run_command(args).await?,
        "show" => show_command(args)?,
        "export-run" => export_command(args)?,
        _ => print_usage(),
    }

    Ok(())
}

async fn run_command(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let repo = required_flag(&args, "--repo")?;
    let task = required_flag(&args, "--task")?;
    let model =
        optional_flag(&args, "--model").unwrap_or_else(|| "qwen/qwen3-coder-30b".to_owned());
    let base_url = optional_flag(&args, "--base-url")
        .or_else(|| std::env::var("TURN_BASE_URL").ok())
        .unwrap_or_else(|| "http://localhost:1234".to_owned());
    let tape = optional_flag(&args, "--tape")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("turn-tape.jsonl"));
    let max_gears = optional_flag(&args, "--max-gears")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20);

    let summary = run(RunOptions {
        repo: PathBuf::from(repo),
        task,
        tape_path: tape,
        max_gears,
        engine: EngineConfig {
            model,
            base_url,
            temperature: Some(0.2),
            top_p: Some(0.95),
            seed: None,
        },
    })
    .await?;
    let total = summary.beliefs_parsed + summary.beliefs_failed;
    let percent = if total == 0 {
        0usize
    } else {
        (summary.beliefs_parsed * 100) / total
    };
    println!(
        "beliefs parsed {}/{} ({}%)",
        summary.beliefs_parsed, total, percent
    );
    for (policy, count) in summary.policy_hits {
        println!("policy {} fired {}", policy, count);
    }
    for (trigger, count) in summary.pauses_by_trigger {
        println!("pause {} {}", trigger, count);
    }
    Ok(())
}

fn show_command(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let path = args.first().ok_or("missing tape path")?;
    let events = Tape::load(path)?;
    for event in events {
        if matches!(event.kind, EventKind::Belief) {
            println!(
                "{}\t{}\t{}\t{}",
                event.seq,
                event
                    .body
                    .get("claim")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
                event
                    .body
                    .get("said")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or_default(),
                event
                    .body
                    .get("logit")
                    .and_then(serde_json::Value::as_f64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned())
            );
        }
    }
    Ok(())
}

fn export_command(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let input = args.first().ok_or("missing tape path")?;
    let output = required_flag(&args, "-o")?;
    let events = Tape::load(input)?;
    let run = Tape::export_run(&events, "turn export");
    std::fs::write(output, serde_json::to_vec_pretty(&run)?)?;
    Ok(())
}

fn required_flag(args: &[String], name: &str) -> Result<String, Box<dyn std::error::Error>> {
    optional_flag(args, name).ok_or_else(|| format!("missing required flag {}", name).into())
}

fn optional_flag(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].clone())
}

fn print_usage() {
    eprintln!("turn run --repo <path> --task <text> [--model <id>] [--base-url <url>] [--tape <out.jsonl>] [--max-gears N]");
    eprintln!("turn show <tape.jsonl>");
    eprintln!("turn export-run <tape.jsonl> -o run.json");
}
