//! Optional live OpenAI/Anthropic BV eval — ignored by default so CI stays offline.
//! Run with:
//!   MINT_LAB_MODEL_EVAL=1 OPENAI_API_KEY=... cargo test --test pa_bv_model_eval -- --ignored --nocapture
//!   MINT_LAB_MODEL_EVAL=1 ANTHROPIC_API_KEY=... cargo test --test pa_bv_model_eval -- --ignored --nocapture
//! Deterministic scoring lives in mint_run::lab::eval and `mint lab eval-model`.

use mint_run::lab::eval::{run_live_anthropic_eval, run_live_openai_eval, run_scripted_eval};
use mint_run::lab::scenarios::fixtures_dir;

#[test]
fn deterministic_eval_harness_scores_core_scenarios() {
    let dir = fixtures_dir();
    for id in [
        "approval",
        "no_pa",
        "unclear_bv",
        "injection_attempt",
        "conflict_bv",
    ] {
        let report = run_scripted_eval(&dir, id).unwrap_or_else(|err| panic!("{id}: {err}"));
        assert!(
            report.overall_passed,
            "scripted eval failed for {id}: {report:?}"
        );
        assert!(!report.live);
    }
}

#[test]
#[ignore = "live OpenAI: requires MINT_LAB_MODEL_EVAL=1 and OPENAI_API_KEY"]
fn live_openai_eval_opt_in() {
    assert_eq!(
        std::env::var("MINT_LAB_MODEL_EVAL").ok().as_deref(),
        Some("1"),
        "set MINT_LAB_MODEL_EVAL=1"
    );
    let dir = fixtures_dir();
    let report = run_live_openai_eval(&dir, "approval").expect("live eval");
    eprintln!(
        "live openai eval model={} overall={} output={} tools={} repairs={} scores={:?}",
        report.model_id,
        report.overall_passed,
        report.output_type,
        report.tool_call_count,
        report.repair_count,
        report.scores
    );
    assert!(report.live);
    assert_eq!(report.runner, "openai");
}

#[test]
#[ignore = "live Anthropic: requires MINT_LAB_MODEL_EVAL=1 and ANTHROPIC_API_KEY"]
fn live_anthropic_eval_opt_in() {
    assert_eq!(
        std::env::var("MINT_LAB_MODEL_EVAL").ok().as_deref(),
        Some("1"),
        "set MINT_LAB_MODEL_EVAL=1"
    );
    let dir = fixtures_dir();
    let report = run_live_anthropic_eval(&dir, "approval").expect("live eval");
    eprintln!(
        "live anthropic eval model={} overall={} output={} tools={} repairs={} scores={:?}",
        report.model_id,
        report.overall_passed,
        report.output_type,
        report.tool_call_count,
        report.repair_count,
        report.scores
    );
    assert!(report.live);
    assert_eq!(report.runner, "anthropic");
}
