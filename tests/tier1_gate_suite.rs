//! Tier 1 gate/policy correctness suite: 12 scenarios over a signed receipt chain.
//! Used by: manual verification of gate.rs against the AERF crypto-core.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

use agentmint::aerf::chain::{verify_aerf_chain, VerifyChainOptions};
use agentmint::aerf::gate::{BlockRule, CrossRef, Spec, ToolCall, ToolRules, Verdict};
use agentmint::aerf::receipt::{verify_aerf_receipt, Receipt, VerifyOptions};
use agentmint::aerf::session::{ScopeTag, Session};

const SIGNING_SEED: [u8; 32] = [42; 32];

fn tier1_spec() -> Spec {
    let mut tools = HashMap::new();

    let mut read_file = ToolRules::default();
    read_file
        .blocked_patterns
        .insert("path".into(), vec!["*.env".into()]);
    tools.insert("read_file".into(), read_file);

    let mut write_file = ToolRules::default();
    write_file.requires = vec!["read_file".into()];
    write_file.cross_ref = vec![CrossRef {
        field: "path".into(),
        other_tool: "read_file".into(),
        other_field: "path".into(),
    }];
    tools.insert("write_file".into(), write_file);

    let mut run_command = ToolRules::default();
    run_command
        .blocked_patterns
        .insert("command".into(), vec!["rm -rf".into()]);
    tools.insert("run_command".into(), run_command);

    let mut git_push = ToolRules::default();
    git_push
        .blocked_values
        .insert("branch".into(), vec!["main".into(), "master".into()]);
    tools.insert("git_push".into(), git_push);

    Spec {
        tools,
        max_identical_calls: 3,
    }
}

fn read_file(path: &str) -> Value {
    json!({ "path": path, "contents": format!("// contents of {path}") })
}

fn write_file(path: &str) -> Value {
    json!({ "path": path, "written": true })
}

fn run_command(command: &str) -> Value {
    json!({ "command": command, "exit_code": 0 })
}

fn git_commit(message: &str) -> Value {
    json!({ "committed": true, "message": message })
}

fn git_push(branch: &str) -> Value {
    json!({ "pushed": true, "branch": branch })
}

fn run_tests(passed: u32, failed: u32) -> Value {
    json!({ "passed": passed, "failed": failed })
}

fn execute(call: &ToolCall) -> Value {
    let field = |name: &str| call.field(name).and_then(Value::as_str).unwrap_or_default();
    match call.tool.as_str() {
        "read_file" => read_file(field("path")),
        "write_file" => write_file(field("path")),
        "run_command" => run_command(field("command")),
        "git_commit" => git_commit(field("message")),
        "git_push" => git_push(field("branch")),
        "run_tests" => run_tests(0, 0),
        _ => Value::Null,
    }
}

fn step(session: &mut Session, call: ToolCall, scope: ScopeTag) -> Verdict {
    step_with_result(session, call, None, scope)
}

fn step_with_result(
    session: &mut Session,
    call: ToolCall,
    result_override: Option<Value>,
    scope: ScopeTag,
) -> Verdict {
    let verdict = session.evaluate(&call);
    let result = if verdict.is_pass() {
        Some(result_override.unwrap_or_else(|| execute(&call)))
    } else {
        None
    };
    session
        .commit(call, verdict.clone(), result, scope)
        .expect("commit receipt");
    verdict
}

fn new_session(plan_id: &str) -> Session {
    Session::new(SigningKey::from_bytes(&SIGNING_SEED), tier1_spec(), plan_id)
}

struct ReceiptAudit {
    all_signed: bool,
    all_verify: bool,
}

fn audit_receipts(session: &Session) -> ReceiptAudit {
    let public_key = session.verifying_key();
    let all_signed = session
        .receipts()
        .iter()
        .all(|receipt| !receipt.signature.is_empty());
    let all_verify = session.receipts().iter().all(|receipt| {
        verify_aerf_receipt(receipt, &public_key, VerifyOptions::default()).ok
    });
    ReceiptAudit {
        all_signed,
        all_verify,
    }
}

struct Row {
    name: &'static str,
    expected: &'static str,
    actual: String,
    signed: bool,
    verifies: bool,
    incident: &'static str,
    relevance: &'static str,
}

fn scenario_1() -> Row {
    let mut session = new_session("normal-bugfix");
    let scope = ScopeTag::in_scope();
    let v1 = step(&mut session, ToolCall::new("read_file", json!({ "path": "src/app.ts" })), scope.clone());
    let v2 = step(&mut session, ToolCall::new("write_file", json!({ "path": "src/app.ts" })), scope.clone());
    let v3 = step_with_result(&mut session, ToolCall::new("run_tests", json!({})), Some(run_tests(3, 0)), scope.clone());
    let v4 = step(&mut session, ToolCall::new("git_commit", json!({ "message": "fix bug" })), scope.clone());
    let v5 = step(&mut session, ToolCall::new("git_push", json!({ "branch": "feature/fix" })), scope);

    for verdict in [&v1, &v2, &v3, &v4, &v5] {
        assert!(verdict.is_pass(), "every step of normal-bugfix must pass");
    }
    let audit = audit_receipts(&session);
    assert!(audit.all_signed && audit.all_verify);
    Row {
        name: "normal-bugfix",
        expected: "pass (all steps)",
        actual: "pass (all 5 steps)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_2() -> Row {
    let mut session = new_session("write-without-read");
    let verdict = step(&mut session, ToolCall::new("write_file", json!({ "path": "src/app.ts" })), ScopeTag::none());
    assert_eq!(verdict.rule(), Some(BlockRule::MissingRequirement));
    let audit = audit_receipts(&session);
    Row {
        name: "write-without-read",
        expected: "block",
        actual: "block (requires)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_3() -> Row {
    let mut session = new_session("destructive-command");
    let verdict = step(&mut session, ToolCall::new("run_command", json!({ "command": "rm -rf /" })), ScopeTag::none());
    assert_eq!(verdict.rule(), Some(BlockRule::BlockedPattern));
    let audit = audit_receipts(&session);
    Row {
        name: "destructive-command",
        expected: "block",
        actual: "block (blocked_patterns)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_4() -> Row {
    let mut session = new_session("push-to-main");
    let verdict = step(&mut session, ToolCall::new("git_push", json!({ "branch": "main" })), ScopeTag::none());
    assert_eq!(verdict.rule(), Some(BlockRule::BlockedValue));
    let audit = audit_receipts(&session);
    Row {
        name: "push-to-main",
        expected: "block",
        actual: "block (blocked_values)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_5() -> (Row, String) {
    let mut session = new_session("collateral-edit");
    step(&mut session, ToolCall::new("read_file", json!({ "path": "src/app.ts" })), ScopeTag::in_scope());
    let verdict = step(
        &mut session,
        ToolCall::new("write_file", json!({ "path": "src/other.ts" })),
        ScopeTag::out_of_scope("wrote a file that was never read in this session"),
    );

    let (actual, note) = match &verdict {
        Verdict::Block { rule: BlockRule::CrossRefMismatch, .. } => (
            "block (cross_ref)".to_owned(),
            "cross_ref treats a write to a never-read file as a HARD BLOCK".to_owned(),
        ),
        Verdict::Pass => (
            "pass (receipt flagged)".to_owned(),
            "cross_ref treats it as pass-with-receipt-flagged-mismatch".to_owned(),
        ),
        other => (format!("block ({other:?})"), "unexpected rule".to_owned()),
    };
    assert!(verdict.is_block(), "collateral-edit resolved to: {actual}");
    let audit = audit_receipts(&session);
    (
        Row {
            name: "collateral-edit",
            expected: "unspecified (report actual)",
            actual,
            signed: audit.all_signed,
            verifies: audit.all_verify,
            incident: "REPLAY",
            relevance: "FOUNDER-RELEVANT",
        },
        note,
    )
}

fn scenario_6() -> Row {
    let mut session = new_session("test-retry-loop");
    let call = ToolCall::new("run_tests", json!({}));
    let mut verdicts = Vec::new();
    for _ in 0..4 {
        verdicts.push(step_with_result(&mut session, call.clone(), Some(run_tests(1, 2)), ScopeTag::none()));
    }
    assert!(verdicts[0].is_pass() && verdicts[1].is_pass() && verdicts[2].is_pass());
    assert_eq!(verdicts[3].rule(), Some(BlockRule::LoopBreaker));
    let audit = audit_receipts(&session);
    Row {
        name: "test-retry-loop",
        expected: "block on 4th",
        actual: "block on 4th (loop_breaker)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_7() -> Row {
    let mut session = new_session("credential-read");
    let verdict = step(&mut session, ToolCall::new("read_file", json!({ "path": ".env" })), ScopeTag::none());
    assert_eq!(verdict.rule(), Some(BlockRule::BlockedPattern));
    let audit = audit_receipts(&session);
    Row {
        name: "credential-read",
        expected: "block",
        actual: "block (blocked_patterns)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_8() -> Row {
    let mut session = new_session("clean-investigation");
    let scope = ScopeTag::in_scope();
    let v1 = step(&mut session, ToolCall::new("read_file", json!({ "path": "src/app.ts" })), scope.clone());
    let v2 = step(&mut session, ToolCall::new("read_file", json!({ "path": "src/other.ts" })), scope.clone());
    let v3 = step(&mut session, ToolCall::new("read_file", json!({ "path": "package.json" })), scope);
    for verdict in [&v1, &v2, &v3] {
        assert!(verdict.is_pass(), "read-only investigation must pass");
    }
    let audit = audit_receipts(&session);
    Row {
        name: "clean-investigation",
        expected: "pass (all steps)",
        actual: "pass (all 3 reads)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_9() -> Row {
    let mut session = new_session("padded-bypass-attempt");
    let padded = format!("{}rm -rf /", "true && ".repeat(50));
    let verdict = step(&mut session, ToolCall::new("run_command", json!({ "command": padded })), ScopeTag::none());
    assert_eq!(
        verdict.rule(),
        Some(BlockRule::BlockedPattern),
        "matcher must scan the full untruncated string"
    );
    let audit = audit_receipts(&session);
    Row {
        name: "padded-bypass-attempt",
        expected: "block",
        actual: "block (blocked_patterns)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "SECURITY-RESEARCH",
    }
}

fn scenario_10() -> (Row, String) {
    let mut session = new_session("benign-scope-creep");
    step(&mut session, ToolCall::new("read_file", json!({ "path": "src/app.ts" })), ScopeTag::in_scope());
    step(&mut session, ToolCall::new("write_file", json!({ "path": "src/app.ts" })), ScopeTag::in_scope());
    let unrelated = step(
        &mut session,
        ToolCall::new("write_file", json!({ "path": "src/unrelated_config.ts" })),
        ScopeTag::out_of_scope("second file, not the file named in the task"),
    );
    let cleanup = step(
        &mut session,
        ToolCall::new("run_command", json!({ "command": "npm cache clean --force" })),
        ScopeTag::out_of_scope("unrequested cleanup command, passes the destructive rule"),
    );

    assert!(unrelated.is_block(), "unrelated write blocked by cross_ref");
    assert!(cleanup.is_pass(), "cleanup passes policy but is out of task scope");

    let flagged: Vec<String> = session
        .receipts()
        .iter()
        .filter(|receipt| receipt.evidence.get("in_task_scope") == Some(&json!(false)))
        .map(|receipt| receipt.action.clone())
        .collect();
    assert_eq!(
        flagged.len(),
        2,
        "receipt trail must flag both out-of-scope actions"
    );
    let cleanup_flagged = session.receipts().iter().any(|receipt| {
        receipt.in_policy && receipt.evidence.get("in_task_scope") == Some(&json!(false))
    });
    assert!(
        cleanup_flagged,
        "a policy-passing action must still be flagged out-of-scope"
    );

    let audit = audit_receipts(&session);
    (
        Row {
            name: "benign-scope-creep",
            expected: "cross_ref blocks 2nd write; cleanup passes",
            actual: "block write(unrelated) + pass cleanup, both scope-flagged".to_owned(),
            signed: audit.all_signed,
            verifies: audit.all_verify,
            incident: "REPLAY",
            relevance: "FOUNDER-RELEVANT",
        },
        "scope-out actions flagged in receipt trail: write_file(src/unrelated_config.ts) [blocked] + run_command(npm cache clean) [passed policy]".to_owned(),
    )
}

fn scenario_11() -> Row {
    let mut session = new_session("cost-loop-runaway");
    let call = ToolCall::new("run_command", json!({ "command": "curl https://api.example/status" }));
    let mut verdicts = Vec::new();
    for _ in 0..8 {
        verdicts.push(step_with_result(
            &mut session,
            call.clone(),
            Some(json!({ "command": "curl https://api.example/status", "body": "" })),
            ScopeTag::none(),
        ));
    }
    assert!(verdicts[0].is_pass() && verdicts[1].is_pass() && verdicts[2].is_pass());
    assert_eq!(verdicts[3].rule(), Some(BlockRule::LoopBreaker));

    let blocking = &session.receipts()[3];
    assert_eq!(
        blocking.evidence.get("identical_call_count"),
        Some(&json!(4)),
        "blocking receipt records how many identical calls occurred"
    );
    let audit = audit_receipts(&session);
    Row {
        name: "cost-loop-runaway",
        expected: "block on 4th",
        actual: "block on 4th (loop_breaker, count=4 recorded)".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "FOUNDER-RELEVANT",
    }
}

fn scenario_12() -> Row {
    let session = Arc::new(Mutex::new(new_session("concurrent-batch-writes")));

    let write = ToolCall::new("write_file", json!({ "path": "src/shared.ts" }));
    let read = ToolCall::new("read_file", json!({ "path": "src/shared.ts" }));

    let handles: Vec<_> = [write, read]
        .into_iter()
        .map(|call| {
            let session = Arc::clone(&session);
            thread::spawn(move || {
                let mut guard = session.lock().expect("session lock");
                let verdict = guard.evaluate(&call);
                let result = if verdict.is_pass() {
                    Some(execute(&call))
                } else {
                    None
                };
                guard
                    .commit(call, verdict, result, ScopeTag::none())
                    .expect("concurrent commit");
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("thread join");
    }

    let guard = session.lock().expect("session lock");
    let receipts: Vec<Receipt> = guard.receipts().to_vec();
    assert_eq!(receipts.len(), 2, "no lost receipt under concurrency");

    let mut seqs: Vec<u64> = receipts.iter().filter_map(|receipt| receipt.seq).collect();
    seqs.sort_unstable();
    assert_eq!(seqs, vec![1, 2], "no seq collision under concurrency");

    let public_key = guard.verifying_key();
    let chain = verify_aerf_chain(
        &receipts,
        VerifyChainOptions {
            issuer_public_key: Some(&public_key),
        },
    );
    assert!(chain.valid, "receipt chain must not break under concurrency");
    let audit = audit_receipts(&guard);
    assert!(audit.all_signed && audit.all_verify);

    Row {
        name: "concurrent-batch-writes",
        expected: "chain intact, no collision",
        actual: "2 receipts, seq [1,2], chain valid".to_owned(),
        signed: audit.all_signed,
        verifies: audit.all_verify,
        incident: "REPLAY",
        relevance: "SECURITY-RESEARCH",
    }
}

fn all_rows() -> (Vec<Row>, String, String) {
    let (row5, note5) = scenario_5();
    let (row10, note10) = scenario_10();
    let rows = vec![
        scenario_1(),
        scenario_2(),
        scenario_3(),
        scenario_4(),
        row5,
        scenario_6(),
        scenario_7(),
        scenario_8(),
        scenario_9(),
        row10,
        scenario_11(),
        scenario_12(),
    ];
    (rows, note5, note10)
}

#[test]
fn scenario_1_normal_bugfix_passes() {
    scenario_1();
}

#[test]
fn scenario_2_write_without_read_blocks() {
    scenario_2();
}

#[test]
fn scenario_3_destructive_command_blocks() {
    scenario_3();
}

#[test]
fn scenario_4_push_to_main_blocks() {
    scenario_4();
}

#[test]
fn scenario_5_collateral_edit_reported() {
    scenario_5();
}

#[test]
fn scenario_6_test_retry_loop_breaks() {
    scenario_6();
}

#[test]
fn scenario_7_credential_read_blocks() {
    scenario_7();
}

#[test]
fn scenario_8_clean_investigation_passes() {
    scenario_8();
}

#[test]
fn scenario_9_padded_bypass_blocks() {
    scenario_9();
}

#[test]
fn scenario_10_benign_scope_creep_flagged() {
    scenario_10();
}

#[test]
fn scenario_11_cost_loop_runaway_breaks() {
    scenario_11();
}

#[test]
fn scenario_12_concurrent_batch_writes_durable() {
    scenario_12();
}

#[test]
fn tier1_summary_table() {
    let (rows, note5, note10) = all_rows();

    println!();
    println!("TIER 1 GATE/POLICY CORRECTNESS SUITE");
    println!(
        "{:<24} | {:<34} | {:<48} | {:<7} | {:<9} | {:<7} | {}",
        "scenario", "expected", "actual decision", "signed?", "verifies?", "incident", "relevance"
    );
    println!("{}", "-".repeat(170));
    for row in &rows {
        println!(
            "{:<24} | {:<34} | {:<48} | {:<7} | {:<9} | {:<7} | {}",
            row.name,
            row.expected,
            row.actual,
            yes_no(row.signed),
            yes_no(row.verifies),
            row.incident,
            row.relevance
        );
    }
    println!();
    println!("Scenario 5 (collateral-edit) actual behaviour: {note5}.");
    println!("Scenario 10 (benign-scope-creep) receipt trail: {note10}.");

    let founder = rows
        .iter()
        .filter(|row| row.relevance == "FOUNDER-RELEVANT")
        .count();
    let security = rows
        .iter()
        .filter(|row| row.relevance == "SECURITY-RESEARCH")
        .count();
    let novel = rows.iter().filter(|row| row.incident == "NOVEL").count();
    println!("FOUNDER-RELEVANT vs SECURITY-RESEARCH: {founder} : {security}");
    println!("NOVEL findings: {novel}");

    assert_eq!(rows.len(), 12);
    assert!(rows.iter().all(|row| row.signed && row.verifies));
}

fn yes_no(flag: bool) -> &'static str {
    if flag {
        "yes"
    } else {
        "no"
    }
}
