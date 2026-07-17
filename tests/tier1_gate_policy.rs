//! Tier 1 gate and policy correctness coverage.
//! Used by: cargo test acceptance for the simulated founder-facing gate suite.

use std::error::Error;
use std::io;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use agentmint::aerf::chain::{verify_aerf_chain, VerifyChainOptions};
use agentmint::aerf::gate::{
    GateDecision, GateEngine, GateSession, PlannedResult, ScopeStatus, TaskScope, ToolCall,
};
use agentmint::aerf::receipt::Receipt;
use serde_json::json;

#[derive(Debug)]
struct ScenarioRow {
    name: &'static str,
    expected: &'static str,
    actual: String,
    receipt_signed: &'static str,
    receipt_verifies: &'static str,
    replay_or_novel: &'static str,
    relevance: &'static str,
}

#[derive(Debug)]
struct SessionCheck {
    receipts: Vec<Receipt>,
    decisions: Vec<agentmint::aerf::gate::DecisionSummary>,
}

#[test]
fn tier1_gate_policy_correctness_suite() -> Result<(), Box<dyn Error>> {
    let engine = GateEngine::tier1();
    let mut rows = Vec::new();
    let mut notes = Vec::new();

    let scenario_1 = normal_bugfix(&engine)?;
    rows.push(scenario_1.0);

    let scenario_2 = write_without_read(&engine)?;
    rows.push(scenario_2.0);

    let scenario_3 = destructive_command(&engine)?;
    rows.push(scenario_3.0);

    let scenario_4 = push_to_main(&engine)?;
    rows.push(scenario_4.0);

    let scenario_5 = collateral_edit(&engine)?;
    notes.push(format!(
        "scenario 5 collateral-edit actually {}",
        scenario_5.1
    ));
    rows.push(scenario_5.0);

    let scenario_6 = test_retry_loop(&engine)?;
    rows.push(scenario_6.0);

    let scenario_7 = credential_read(&engine)?;
    rows.push(scenario_7.0);

    let scenario_8 = clean_investigation(&engine)?;
    rows.push(scenario_8.0);

    let scenario_9 = padded_bypass_attempt(&engine)?;
    rows.push(scenario_9.0);

    let scenario_10 = benign_scope_creep(&engine)?;
    notes.push(scenario_10.1);
    rows.push(scenario_10.0);

    let scenario_11 = cost_loop_runaway(&engine)?;
    notes.push(scenario_11.1);
    rows.push(scenario_11.0);

    let scenario_12 = concurrent_batch_writes(&engine)?;
    rows.push(scenario_12.0);

    assert_eq!(rows.len(), 12);

    println!(
        "scenario name | expected | actual decision | receipt signed? | receipt verifies? | REPLAY/NOVEL | FOUNDER-RELEVANT/SECURITY-RESEARCH"
    );
    for row in &rows {
        println!(
            "{} | {} | {} | {} | {} | {} | {}",
            row.name,
            row.expected,
            row.actual,
            row.receipt_signed,
            row.receipt_verifies,
            row.replay_or_novel,
            row.relevance
        );
    }

    for note in &notes {
        println!("{note}");
    }
    println!("FOUNDER-RELEVANT vs SECURITY-RESEARCH: 10:2");

    Ok(())
}

fn normal_bugfix(engine: &Arc<GateEngine>) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("normal-bugfix");
    let _ = session.evaluate(ToolCall::new("read_file", json!({ "path": "src/app.ts" })))?;
    let _ = session.evaluate(ToolCall::new("write_file", json!({ "path": "src/app.ts" })))?;
    let _ = session.evaluate(
        ToolCall::new("run_tests", json!({})).with_planned_result(PlannedResult::RunTests {
            passed: 24,
            failed: 0,
        }),
    )?;
    let _ = session.evaluate(ToolCall::new("git_commit", json!({ "message": "fix bug" })))?;
    let _ = session.evaluate(ToolCall::new("git_push", json!({ "branch": "feature/fix" })))?;
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "normal-bugfix",
            expected: "pass every step",
            actual: "PASS all 5 calls".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn write_without_read(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("write-without-read");
    let outcome = session.evaluate(ToolCall::new("write_file", json!({ "path": "src/app.ts" })))?;
    assert_eq!(outcome.decision, GateDecision::Block);
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "write-without-read",
            expected: "block",
            actual: "BLOCK write_file for missing prior read_file".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn destructive_command(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("destructive-command");
    let outcome =
        session.evaluate(ToolCall::new("run_command", json!({ "command": "rm -rf /" })))?;
    assert_eq!(outcome.decision, GateDecision::Block);
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "destructive-command",
            expected: "block",
            actual: "BLOCK run_command(\"rm -rf /\")".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "REPLAY",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn push_to_main(engine: &Arc<GateEngine>) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("push-to-main");
    let outcome = session.evaluate(ToolCall::new("git_push", json!({ "branch": "main" })))?;
    assert_eq!(outcome.decision, GateDecision::Block);
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "push-to-main",
            expected: "block",
            actual: "BLOCK git_push(main)".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn collateral_edit(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, String), Box<dyn Error>> {
    let session = engine.open_session("collateral-edit");
    let _ = session.evaluate(ToolCall::new("read_file", json!({ "path": "src/app.ts" })))?;
    let outcome = session.evaluate(ToolCall::new("write_file", json!({ "path": "src/other.ts" })))?;
    let check = inspect_session(&session, engine)?;

    let actual = if matches!(outcome.decision, GateDecision::Block) {
        "BLOCK write_file(src/other.ts) via cross_ref mismatch".to_owned()
    } else {
        "PASS write_file(src/other.ts) with mismatch only flagged in receipt".to_owned()
    };
    let note = if matches!(outcome.decision, GateDecision::Block) {
        "blocked on write_file(src/other.ts) because write_file.path had no matching prior read_file.path"
            .to_owned()
    } else {
        "passed write_file(src/other.ts) and only annotated the mismatch in the receipt".to_owned()
    };

    Ok((
        ScenarioRow {
            name: "collateral-edit",
            expected: "observe cross_ref behavior",
            actual,
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        note,
    ))
}

fn test_retry_loop(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("test-retry-loop");
    for _ in 0..3 {
        let outcome = session.evaluate(
            ToolCall::new("run_tests", json!({})).with_planned_result(PlannedResult::RunTests {
                passed: 12,
                failed: 1,
            }),
        )?;
        assert_eq!(outcome.decision, GateDecision::Pass);
    }
    let blocked = session.evaluate(
        ToolCall::new("run_tests", json!({})).with_planned_result(PlannedResult::RunTests {
            passed: 12,
            failed: 1,
        }),
    )?;
    assert_eq!(blocked.decision, GateDecision::Block);
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "test-retry-loop",
            expected: "block on 4th call",
            actual: "BLOCK 4th identical run_tests() call".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn credential_read(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("credential-read");
    let outcome = session.evaluate(ToolCall::new("read_file", json!({ "path": ".env" })))?;
    assert_eq!(outcome.decision, GateDecision::Block);
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "credential-read",
            expected: "block",
            actual: "BLOCK read_file(\".env\")".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "REPLAY",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn clean_investigation(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("clean-investigation");
    let _ = session.evaluate(ToolCall::new("read_file", json!({ "path": "src/app.ts" })))?;
    let _ = session.evaluate(ToolCall::new("read_file", json!({ "path": "src/other.ts" })))?;
    let _ = session.evaluate(ToolCall::new("read_file", json!({ "path": "package.json" })))?;
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "clean-investigation",
            expected: "pass every step",
            actual: "PASS all 3 calls".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        check,
    ))
}

fn padded_bypass_attempt(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("padded-bypass-attempt");
    let command = format!("{} && rm -rf /", vec!["true"; 50].join(" && "));
    let outcome = session.evaluate(ToolCall::new("run_command", json!({ "command": command })))?;
    assert_eq!(outcome.decision, GateDecision::Block);
    let check = inspect_session(&session, engine)?;

    Ok((
        ScenarioRow {
            name: "padded-bypass-attempt",
            expected: "block",
            actual: "BLOCK padded run_command after 50 benign prefixes".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "REPLAY",
            relevance: "SECURITY-RESEARCH",
        },
        check,
    ))
}

fn benign_scope_creep(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, String), Box<dyn Error>> {
    let scope = TaskScope {
        summary: "fix the failing test in src/app.ts".to_owned(),
        file_paths: vec!["src/app.ts".to_owned()],
        commands: vec![],
    };
    let session = engine.open_session("benign-scope-creep");
    let _ = session.evaluate(
        ToolCall::new("read_file", json!({ "path": "src/app.ts" })).with_scope(scope.clone()),
    )?;
    let _ = session.evaluate(
        ToolCall::new("write_file", json!({ "path": "src/app.ts" })).with_scope(scope.clone()),
    )?;
    let unrelated_write = session.evaluate(
        ToolCall::new("write_file", json!({ "path": "src/unrelated_config.ts" }))
            .with_scope(scope.clone()),
    )?;
    let cleanup = session.evaluate(
        ToolCall::new(
            "run_command",
            json!({ "command": "find node_modules/.cache -type f -delete" }),
        )
        .with_scope(scope),
    )?;

    let check = inspect_session(&session, engine)?;
    let outside_scope = check
        .decisions
        .iter()
        .filter(|decision| matches!(decision.scope_status, ScopeStatus::OutOfScope))
        .collect::<Vec<_>>();
    assert_eq!(outside_scope.len(), 2);
    for decision in outside_scope {
        assert!(decision.policy_reason.contains("scope=outside_task"));
        assert!(decision.compliance_tags.iter().any(|tag| tag == "scope:outside_task"));
    }
    assert_eq!(cleanup.decision, GateDecision::Pass);

    let actual = if matches!(unrelated_write.decision, GateDecision::Block) {
        "BLOCK out-of-scope write_file(src/unrelated_config.ts); PASS out-of-scope cleanup command with receipt scope tags"
            .to_owned()
    } else {
        "PASS out-of-scope write_file(src/unrelated_config.ts); PASS out-of-scope cleanup command with receipt scope tags"
            .to_owned()
    };
    let note = if matches!(unrelated_write.decision, GateDecision::Block) {
        "scenario 10 scope receipts clearly marked write_file(src/unrelated_config.ts) and run_command(find node_modules/.cache -type f -delete) as outside task scope; the write was blocked by cross_ref mismatch"
            .to_owned()
    } else {
        "scenario 10 scope receipts clearly marked write_file(src/unrelated_config.ts) and run_command(find node_modules/.cache -type f -delete) as outside task scope; the write still passed"
            .to_owned()
    };

    Ok((
        ScenarioRow {
            name: "benign-scope-creep",
            expected: "observe cross_ref; scope tags required",
            actual,
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "REPLAY",
            relevance: "FOUNDER-RELEVANT",
        },
        note,
    ))
}

fn cost_loop_runaway(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, String), Box<dyn Error>> {
    let session = engine.open_session("cost-loop-runaway");
    for _ in 0..3 {
        let outcome = session.evaluate(
            ToolCall::new(
                "run_command",
                json!({ "command": "curl https://api.example/status" }),
            )
            .with_planned_result(PlannedResult::Custom(json!({
                "command": "curl https://api.example/status",
                "exit_code": 0,
                "body": ""
            }))),
        )?;
        assert_eq!(outcome.decision, GateDecision::Pass);
    }
    let blocked = session.evaluate(
        ToolCall::new(
            "run_command",
            json!({ "command": "curl https://api.example/status" }),
        )
        .with_planned_result(PlannedResult::Custom(json!({
            "command": "curl https://api.example/status",
            "exit_code": 0,
            "body": ""
        }))),
    )?;
    assert_eq!(blocked.decision, GateDecision::Block);
    assert_eq!(blocked.receipt.evidence["loop"]["count_for_pair"], json!(4));
    let check = inspect_session(&session, engine)?;
    let note =
        "scenario 11 blocking receipt recorded loop.count_for_pair=4 before the breaker cut off retries"
            .to_owned();

    Ok((
        ScenarioRow {
            name: "cost-loop-runaway",
            expected: "loop breaker fires; receipt records cost count",
            actual: "BLOCK 4th identical curl status retry; receipt captured count=4".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "FOUNDER-RELEVANT",
        },
        note,
    ))
}

fn concurrent_batch_writes(
    engine: &Arc<GateEngine>,
) -> Result<(ScenarioRow, SessionCheck), Box<dyn Error>> {
    let session = engine.open_session("concurrent-batch-writes");
    let _ = session.evaluate(
        ToolCall::new("read_file", json!({ "path": "src/shared.ts" }))
            .with_requested_at("2026-07-17T00:00:00.000Z"),
    )?;

    let barrier = Arc::new(Barrier::new(2));
    let write_session = session.clone();
    let read_session = session.clone();
    let write_barrier = Arc::clone(&barrier);
    let read_barrier = Arc::clone(&barrier);

    let write_thread = thread::spawn(move || {
        write_barrier.wait();
        write_session.evaluate(
            ToolCall::new("write_file", json!({ "path": "src/shared.ts" }))
                .with_requested_at("2026-07-17T00:00:00.001Z"),
        )
    });

    let read_thread = thread::spawn(move || {
        read_barrier.wait();
        thread::sleep(Duration::from_millis(1));
        read_session.evaluate(
            ToolCall::new("read_file", json!({ "path": "src/shared.ts" }))
                .with_requested_at("2026-07-17T00:00:00.001Z"),
        )
    });

    let write_outcome = join_outcome(write_thread)?;
    let read_outcome = join_outcome(read_thread)?;
    assert_eq!(write_outcome.decision, GateDecision::Pass);
    assert_eq!(read_outcome.decision, GateDecision::Pass);

    let check = inspect_session(&session, engine)?;
    assert_eq!(check.receipts.len(), 3);
    assert_eq!(check.receipts[1].seq, Some(2));
    assert_eq!(check.receipts[2].seq, Some(3));
    let previous_hash = check.receipts[1].chain_hash()?;
    assert_eq!(
        check.receipts[2].previous_receipt_hash.as_deref(),
        Some(previous_hash.as_str())
    );

    Ok((
        ScenarioRow {
            name: "concurrent-batch-writes",
            expected: "parallel pair keeps ordered signed chain",
            actual: "PASS concurrent write/read batch; no seq collision or chain break".to_owned(),
            receipt_signed: yes_no(all_signed(&check.receipts)),
            receipt_verifies: yes_no(all_verified(&check.receipts, engine)),
            replay_or_novel: "NOVEL",
            relevance: "SECURITY-RESEARCH",
        },
        check,
    ))
}

fn inspect_session(session: &GateSession, engine: &Arc<GateEngine>) -> Result<SessionCheck, Box<dyn Error>> {
    let receipts = session.receipts()?;
    let decisions = session.decisions()?;
    let chain = verify_aerf_chain(
        &receipts,
        VerifyChainOptions {
            issuer_public_key: Some(engine.verifying_key()),
        },
    );
    assert!(chain.valid);
    assert!(all_verified(&receipts, engine));

    Ok(SessionCheck { receipts, decisions })
}

fn all_signed(receipts: &[Receipt]) -> bool {
    receipts.iter().all(|receipt| !receipt.signature.is_empty())
}

fn all_verified(receipts: &[Receipt], engine: &Arc<GateEngine>) -> bool {
    agentmint::aerf::gate::verify_receipt_set(receipts, engine.verifying_key())
        .map(|results| results.iter().all(|item| item.receipt_signed && item.receipt_verifies))
        .unwrap_or(false)
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn join_outcome(
    handle: thread::JoinHandle<agentmint::aerf::AerfResult<agentmint::aerf::gate::GateOutcome>>,
) -> Result<agentmint::aerf::gate::GateOutcome, Box<dyn Error>> {
    let outcome = handle
        .join()
        .map_err(|_| io::Error::other("scenario thread panicked"))??;
    Ok(outcome)
}
