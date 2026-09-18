//! CLI entrypoints for `mint lab ...`.
//! Used by: binary main routing.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use uuid::Uuid;

use crate::lab::console;
use crate::lab::error::{LabError, LabResult};
use crate::lab::inspect::{events_json, inspect_run, inspect_text};
use crate::lab::scenarios::{fixtures_dir, list_scenarios_from, load_scenario_from};
use crate::lab::workflow::LabEngine;

pub fn run(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }
    match args[0].as_str() {
        "start" => cmd_start(&args[1..]),
        "console" => cmd_console(&args[1..]),
        "inspect" => cmd_inspect(&args[1..]),
        "events" => cmd_events(&args[1..]),
        "advance" => cmd_advance(&args[1..]),
        "fault" => cmd_fault(&args[1..]),
        "check" => cmd_check(&args[1..]),
        "run" => cmd_run(&args[1..]),
        "list" => cmd_list(&args[1..]),
        "eval-model" => cmd_eval_model(&args[1..]),
        "mcp-stdio" => cmd_mcp_stdio(&args[1..]),
        "mcp-http" => cmd_mcp_http(&args[1..]),
        other => {
            eprintln!("unknown lab command: {other}");
            print_help();
            Ok(ExitCode::from(2))
        }
    }
}

fn print_help() {
    println!(
        "mint lab — synthetic medical-benefit PA/BV case console (experimental)\n\n\
         Usage:\n\
         \tmint lab start --scenario ID [--json] [--dir PATH]\n\
         \tmint lab console <run-id> [--dir PATH]\n\
         \tmint lab inspect <run-id> [--json] [--dir PATH]\n\
         \tmint lab events <run-id> [--json] [--dir PATH]\n\
         \tmint lab advance <run-id> --by DURATION [--dir PATH]\n\
         \tmint lab fault <run-id> <fault> [--dir PATH]\n\
         \tmint lab check <run-id> [--dir PATH]\n\
         \tmint lab run <scenario> [--json] [--dir PATH]\n\
         \tmint lab list [--json] [--dir PATH]\n\
         \tmint lab eval-model [--scenario ID] [--runner scripted|mcp] [--live] [--json]\n\
         \tmint lab mcp-stdio [--scenario ID | --run-id UUID] [--dir PATH]\n\
         \tmint lab mcp-http [--scenario ID | --run-id UUID] [--bind 127.0.0.1:8787] [--dir PATH]\n\n\
         Agents: MINT_LAB_AGENT=scripted|openai|mcp (default scripted).\n\
         OpenAI: OPENAI_API_KEY + optional MINT_LAB_MODEL (default gpt-4.1-mini).\n\
         MCP agent: MINT_LAB_MCP_TOKEN + MINT_LAB_MCP_URL (fail-closed if missing).\n\
         Live eval: MINT_LAB_MODEL_EVAL=1 with --live (never required for CI).\n\
         MCP: MINT_LAB_MCP_TOKEN required; loopback/stdio only. MINT_LAB_AUTO_PAYER=1 for FakePayer.\n\
         Default data dir: ./lab-data or MINT_LAB_DIR.\n\
         Synthetic CPT 72148 outpatient MRI lumbar spine only. No real patient data.\n\
         No reviewer/appeal/doc/payer LLM agents — those stay human or fixture-driven.\n"
    );
}

fn data_dir(args: &[String]) -> PathBuf {
    if let Some(path) = flag(args, "--dir") {
        return PathBuf::from(path);
    }
    env::var("MINT_LAB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./lab-data"))
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].as_str())
}

fn wants_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json")
}

fn open_engine(args: &[String]) -> LabResult<LabEngine> {
    let dir = data_dir(args);
    LabEngine::open(&dir, &fixtures_dir())
}

fn cmd_start(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let scenario = flag(args, "--scenario").ok_or("missing --scenario")?;
    let engine = open_engine(args)?;
    let run_id = engine.start_run(scenario)?;
    let report = engine.process_pending(run_id)?;
    if wants_json(args) {
        println!(
            "{}",
            serde_json::json!({
                "run_id": run_id,
                "stage": report.stage,
                "happened": report.happened,
            })
        );
    } else {
        println!("started run_id={run_id} stage={:?}", report.stage);
        for h in report.happened {
            println!("  - {h}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_console(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let run_id = args
        .first()
        .ok_or("missing run-id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid run-id: {e}"))?;
    let engine = open_engine(args)?;
    console::run_console(&engine, run_id)?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_inspect(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let run_id = args
        .first()
        .ok_or("missing run-id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid run-id: {e}"))?;
    let engine = open_engine(args)?;
    let report = inspect_run(&engine, run_id)?;
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", inspect_text(&report));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_events(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let run_id = args
        .first()
        .ok_or("missing run-id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid run-id: {e}"))?;
    let engine = open_engine(args)?;
    let events = events_json(&engine, run_id)?;
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&events)?);
    } else {
        for event in events.as_array().cloned().unwrap_or_default() {
            println!(
                "#{} {} {}",
                event.get("seq").and_then(|v| v.as_u64()).unwrap_or(0),
                event.get("kind").and_then(|v| v.as_str()).unwrap_or("?"),
                event
                    .get("payload_json")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_advance(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let run_id = args
        .first()
        .ok_or("missing run-id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid run-id: {e}"))?;
    let by = flag(args, "--by").ok_or("missing --by")?;
    let engine = open_engine(args)?;
    engine.advance_clock(parse_duration(by)?)?;
    let report = engine.process_pending(run_id)?;
    println!("advanced; stage={:?}", report.stage);
    Ok(ExitCode::SUCCESS)
}

fn cmd_fault(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let run_id = args
        .first()
        .ok_or("missing run-id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid run-id: {e}"))?;
    let fault = args.get(1).ok_or("missing fault name")?;
    let engine = open_engine(args)?;
    let _ = engine.require_case(run_id)?;
    engine.inject_fault(fault)?;
    println!("fault {fault} injected for run {run_id}");
    Ok(ExitCode::SUCCESS)
}

fn cmd_check(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let run_id = args
        .first()
        .ok_or("missing run-id")?
        .parse::<Uuid>()
        .map_err(|e| format!("invalid run-id: {e}"))?;
    let engine = open_engine(args)?;
    let failures = engine.check_run(run_id)?;
    if failures.is_empty() {
        println!("check OK");
        Ok(ExitCode::SUCCESS)
    } else {
        for f in failures {
            println!("FAIL: {f}");
        }
        Ok(ExitCode::from(1))
    }
}

fn cmd_run(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let scenario = args.first().ok_or("missing scenario id")?;
    let engine = open_engine(args)?;
    let (run_id, report) = engine.run_scripted(scenario)?;
    let case = engine.require_case(run_id)?;
    if wants_json(args) {
        println!(
            "{}",
            serde_json::json!({
                "run_id": run_id,
                "stage": case.stage,
                "disposition": case.disposition,
                "happened": report.happened,
            })
        );
    } else {
        println!(
            "run_id={run_id} stage={:?} disposition={:?}",
            case.stage, case.disposition
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_list(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let dir = data_dir(args);
    if wants_json(args) {
        if dir.join("lab.db").exists() {
            let engine = LabEngine::open(&dir, &fixtures_dir())?;
            let runs = engine.store.list_runs()?;
            println!("{}", serde_json::to_string_pretty(&runs)?);
        } else {
            let scenarios = list_scenarios_from(&fixtures_dir())?;
            println!("{}", serde_json::to_string_pretty(&scenarios)?);
        }
    } else {
        println!("scenarios:");
        for id in list_scenarios_from(&fixtures_dir())? {
            let fixture = load_scenario_from(&fixtures_dir(), &id)?;
            println!("  {id} — {}", fixture.expected_disposition);
        }
        if dir.join("lab.db").exists() {
            let engine = LabEngine::open(&dir, &fixtures_dir())?;
            println!("runs in {}:", dir.display());
            for (run_id, scenario, stage) in engine.store.list_runs()? {
                println!("  {run_id} scenario={scenario} stage={stage:?}");
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_eval_model(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    use crate::lab::eval::{run_live_openai_eval, run_mcp_eval, run_scripted_eval};

    let live = args.iter().any(|a| a == "--live");
    let scenario = flag(args, "--scenario").unwrap_or("approval");
    let runner = flag(args, "--runner").unwrap_or("scripted");
    let fixtures = fixtures_dir();
    let report = if live {
        run_live_openai_eval(&fixtures, scenario)?
    } else if runner == "mcp" {
        run_mcp_eval(&fixtures, scenario)?
    } else {
        run_scripted_eval(&fixtures, scenario)?
    };
    if wants_json(args) {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "scenario={} model={} runner={} live={} overall={} tools={} repairs={}",
            report.scenario_id,
            report.model_id,
            report.runner,
            report.live,
            report.overall_passed,
            report.tool_call_count,
            report.repair_count
        );
        for score in &report.scores {
            let mark = if score.passed { "PASS" } else { "FAIL" };
            println!("  {mark} {:?} — {}", score.dimension, score.detail);
        }
        for note in &report.notes {
            println!("  note: {note}");
        }
    }
    if report.overall_passed {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn cmd_mcp_stdio(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let state = prepare_mcp_state(args)?;
    let handle = tokio::runtime::Handle::current();
    handle.block_on(crate::lab::mcp::stdio::serve_stdio(state))?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_mcp_http(args: &[String]) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let bind = flag(args, "--bind").unwrap_or("127.0.0.1:8787");
    let addr: std::net::SocketAddr = bind
        .parse()
        .map_err(|err| format!("invalid --bind: {err}"))?;
    let state = prepare_mcp_state(args)?;
    let handle = tokio::runtime::Handle::current();
    handle.block_on(crate::lab::mcp::http::serve_http(state, addr))?;
    Ok(ExitCode::SUCCESS)
}

fn prepare_mcp_state(
    args: &[String],
) -> Result<crate::lab::mcp::McpState, Box<dyn std::error::Error>> {
    let token = crate::lab::mcp::require_mcp_token()?;
    let engine = open_engine(args)?;
    let run_id = if let Some(raw) = flag(args, "--run-id") {
        raw.parse::<Uuid>()
            .map_err(|e| format!("invalid --run-id: {e}"))?
    } else if let Some(scenario) = flag(args, "--scenario") {
        engine.start_run(scenario)?
    } else {
        return Err("mcp requires --scenario or --run-id".into());
    };
    let task_id = engine.prepare_bv_task(run_id)?;
    eprintln!("MCP_RUN_ID={run_id}");
    eprintln!("MCP_TASK_ID={task_id}");
    let apply = !args.iter().any(|a| a == "--observe-only");
    Ok(crate::lab::mcp::McpState {
        engine: std::sync::Arc::new(engine),
        run_id,
        task_id,
        token,
        apply,
        auto_payer: crate::lab::mcp::auto_payer_enabled(),
        trace: std::sync::Mutex::new(crate::lab::tools::ToolTrace::new()),
    })
}

fn parse_duration(raw: &str) -> LabResult<Duration> {
    let raw = raw.trim();
    if let Some(num) = raw.strip_suffix('s') {
        let n: u64 = num
            .parse()
            .map_err(|_| LabError::Invalid(format!("duration {raw}")))?;
        return Ok(Duration::from_secs(n));
    }
    if let Some(num) = raw.strip_suffix('m') {
        let n: u64 = num
            .parse()
            .map_err(|_| LabError::Invalid(format!("duration {raw}")))?;
        return Ok(Duration::from_secs(n * 60));
    }
    if let Some(num) = raw.strip_suffix('h') {
        let n: u64 = num
            .parse()
            .map_err(|_| LabError::Invalid(format!("duration {raw}")))?;
        return Ok(Duration::from_secs(n * 3600));
    }
    let n: u64 = raw
        .parse()
        .map_err(|_| LabError::Invalid(format!("duration {raw}")))?;
    Ok(Duration::from_secs(n))
}

#[allow(dead_code)]
fn ensure_dir(path: &Path) -> LabResult<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}
