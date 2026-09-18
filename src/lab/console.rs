//! Interactive PA/BV lab console.
//! Used by: `mint lab console <run-id>`.

use std::io::{self, BufRead, Write};

use uuid::Uuid;

use crate::lab::clock::parse_duration;
use crate::lab::domain::{CaseStage, ReviewDecision, Role};
use crate::lab::error::{LabError, LabResult};
use crate::lab::inspect::{inspect_run, inspect_text};
use crate::lab::workflow::{LabEngine, TickReport};

pub fn run_console(engine: &LabEngine, run_id: Uuid) -> LabResult<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut role = Role::Operator;
    let mut line_buf = String::new();

    writeln!(
        stdout,
        "PA/BV lab console run={run_id} role={role:?}. Commands start with /. EOF or /quit to exit."
    )?;
    print_status_block(
        engine,
        run_id,
        &TickReport {
            stage: engine.require_case(run_id)?.stage,
            happened: vec!["console attached".into()],
            next_owner: Some(role),
            next_action: Some("issue command".into()),
            evidence: vec![],
            blockers: vec![],
        },
    )?;

    let mut reader = stdin.lock();
    loop {
        write!(stdout, "[{role:?}]> ")?;
        stdout.flush()?;
        line_buf.clear();
        let n = reader.read_line(&mut line_buf)?;
        if n == 0 {
            writeln!(stdout, "\nEOF — leaving console (state persisted).")?;
            break;
        }
        let trimmed = line_buf.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }

        let mut input = trimmed.to_string();
        if !input.starts_with('/') && role == Role::Payer {
            while input.ends_with('\\') {
                input.pop();
                write!(stdout, "... ")?;
                stdout.flush()?;
                let mut cont = String::new();
                let n = reader.read_line(&mut cont)?;
                if n == 0 {
                    break;
                }
                input.push_str(cont.trim_end_matches(['\r', '\n']));
            }
            if !input.ends_with('\\') {
                let blank_check = input.clone();
                if blank_check.is_empty() {
                    continue;
                }
            }
            match engine.submit_payer_answer(run_id, &input) {
                Ok(report) => print_status_block(engine, run_id, &report)?,
                Err(err) => writeln!(stdout, "error: {err}")?,
            }
            continue;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        match parts[0] {
            "/quit" | "/exit" => break,
            "/status" => {
                let report = engine
                    .process_pending(run_id)
                    .unwrap_or_else(|_| TickReport {
                        stage: engine
                            .require_case(run_id)
                            .map(|c| c.stage)
                            .unwrap_or(CaseStage::Paused),
                        happened: vec![],
                        next_owner: None,
                        next_action: None,
                        evidence: vec![],
                        blockers: vec![],
                    });
                print_status_block(engine, run_id, &report)?;
            }
            "/history" => {
                let snap = engine.snapshot(run_id)?;
                for event in snap.events {
                    writeln!(
                        stdout,
                        "  #{} {} {}",
                        event.seq,
                        sanitize(&event.kind),
                        sanitize(&event.payload_json)
                    )?;
                }
            }
            "/evidence" => {
                if parts.len() < 2 {
                    writeln!(stdout, "usage: /evidence <id>")?;
                    continue;
                }
                let report = inspect_run(engine, run_id)?;
                let id = parts[1];
                let mut found = false;
                for item in report.items {
                    if item.id.starts_with(id) || item.evidence_refs.iter().any(|e| e.contains(id))
                    {
                        writeln!(
                            stdout,
                            "[{:?}] {} — {}",
                            item.class,
                            sanitize(&item.kind),
                            sanitize(&item.summary)
                        )?;
                        found = true;
                    }
                }
                if !found {
                    writeln!(stdout, "no evidence matching {id}")?;
                }
            }
            "/role" => {
                if parts.len() < 2 {
                    writeln!(stdout, "usage: /role payer|customer|reviewer|operator")?;
                    continue;
                }
                role = match parts[1] {
                    "payer" => Role::Payer,
                    "customer" => Role::Customer,
                    "reviewer" => Role::Reviewer,
                    "operator" => Role::Operator,
                    other => {
                        writeln!(stdout, "unknown role {other}")?;
                        continue;
                    }
                };
                writeln!(stdout, "role set to {role:?}")?;
            }
            "/supply" => {
                if parts.len() < 3 {
                    writeln!(stdout, "usage: /supply <req> <doc>")?;
                    continue;
                }
                match engine.supply_document(run_id, parts[1], parts[2]) {
                    Ok(report) => print_status_block(engine, run_id, &report)?,
                    Err(err) => writeln!(stdout, "error: {err}")?,
                }
            }
            "/review" => {
                if parts.len() < 3 {
                    writeln!(stdout, "usage: /review <packet> approve|decline|changes")?;
                    continue;
                }
                let packet_id =
                    Uuid::parse_str(parts[1]).map_err(|e| LabError::Invalid(e.to_string()))?;
                let decision = match parts[2] {
                    "approve" => ReviewDecision::Approve,
                    "decline" => ReviewDecision::Decline,
                    "changes" => ReviewDecision::Changes,
                    other => {
                        writeln!(stdout, "unknown decision {other}")?;
                        continue;
                    }
                };
                match engine.review_packet(run_id, packet_id, decision, "console-reviewer") {
                    Ok(report) => print_status_block(engine, run_id, &report)?,
                    Err(err) => writeln!(stdout, "error: {err}")?,
                }
            }
            "/advance" => {
                if parts.len() < 2 {
                    writeln!(stdout, "usage: /advance <duration> (e.g. 1h, 30m, 60s)")?;
                    continue;
                }
                let dur = parse_duration(parts[1])?;
                engine.advance_clock(dur)?;
                let report = engine.process_pending(run_id)?;
                print_status_block(engine, run_id, &report)?;
            }
            "/fault" => {
                if parts.len() < 2 {
                    writeln!(stdout, "usage: /fault <name>")?;
                    continue;
                }
                engine.inject_fault(parts[1])?;
                writeln!(stdout, "fault injected: {}", parts[1])?;
            }
            "/check" => {
                let failures = engine.check_run(run_id)?;
                if failures.is_empty() {
                    writeln!(stdout, "check: OK")?;
                } else {
                    for f in failures {
                        writeln!(stdout, "check FAIL: {}", sanitize(&f))?;
                    }
                }
            }
            "/appeal" => match engine.initiate_appeal(run_id) {
                Ok(report) => print_status_block(engine, run_id, &report)?,
                Err(err) => writeln!(stdout, "error: {err}")?,
            },
            "/cancel" => {
                let report = engine.cancel_case(run_id)?;
                print_status_block(engine, run_id, &report)?;
            }
            "/inspect" => {
                let report = inspect_run(engine, run_id)?;
                write!(stdout, "{}", sanitize(&inspect_text(&report)))?;
            }
            "/tick" => {
                let report = engine.process_pending(run_id)?;
                print_status_block(engine, run_id, &report)?;
            }
            other if other.starts_with('/') => {
                writeln!(stdout, "unknown command {other}")?;
            }
            _ => {
                writeln!(
                    stdout,
                    "non-slash input is payer speech only when role=payer; current role={role:?}"
                )?;
            }
        }
    }
    Ok(())
}

fn print_status_block(engine: &LabEngine, run_id: Uuid, report: &TickReport) -> LabResult<()> {
    let mut stdout = io::stdout();
    let case = engine.require_case(run_id)?;
    writeln!(stdout, "----")?;
    writeln!(stdout, "stage: {:?}", report.stage)?;
    writeln!(
        stdout,
        "happened: {}",
        sanitize(&report.happened.join("; "))
    )?;
    writeln!(
        stdout,
        "next: {:?} / {}",
        report.next_owner,
        report.next_action.as_deref().unwrap_or("-")
    )?;
    writeln!(
        stdout,
        "evidence: {}",
        sanitize(&report.evidence.join(", "))
    )?;
    writeln!(
        stdout,
        "blockers: {}",
        sanitize(&report.blockers.join(" | "))
    )?;
    writeln!(stdout, "disposition: {:?}", case.disposition)?;
    writeln!(stdout, "----")?;
    Ok(())
}

fn sanitize(input: &str) -> String {
    input
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect()
}
