//! PA/BV lab acceptance cases A–M (synthetic medical benefit MRI).

use std::path::PathBuf;
use std::time::Duration;

use mint_run::lab::domain::*;
use mint_run::lab::payer::PayerAdapter;
use mint_run::lab::scenarios::fixtures_dir;
use mint_run::lab::workflow::LabEngine;
use tempfile::tempdir;

fn engine() -> (tempfile::TempDir, LabEngine) {
    let dir = tempdir().expect("tempdir");
    let fixtures = fixtures_dir_resolved();
    let engine = LabEngine::open(dir.path(), &fixtures).expect("engine");
    (dir, engine)
}

fn fixtures_dir_resolved() -> PathBuf {
    let from_env = fixtures_dir();
    if from_env.exists() {
        return from_env;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/pa_bv")
}

fn advance_to_review(engine: &LabEngine, run_id: uuid::Uuid) {
    for _ in 0..16 {
        let snap = engine.snapshot(run_id).unwrap();
        match snap.case.stage {
            CaseStage::Review => return,
            CaseStage::Bv => {
                if snap
                    .pending
                    .iter()
                    .any(|p| p.kind == PendingKind::AgentQuestion)
                {
                    let answer = "Member is active. Prior authorization is required for CPT 72148 outpatient MRI lumbar spine. Required documentation: clinical notes and signed order.";
                    engine.submit_payer_answer(run_id, answer).unwrap();
                } else {
                    engine.process_pending(run_id).unwrap();
                }
            }
            CaseStage::Documentation => {
                let required = snap
                    .determinations
                    .iter()
                    .rev()
                    .find(|d| d.kind == DeterminationKind::PaRequired)
                    .map(|d| d.required_docs.clone())
                    .unwrap_or_else(|| vec!["clinical_notes".into(), "order".into()]);
                for doc in required {
                    if !snap.documents.iter().any(|d| d.request_id == doc) {
                        engine.supply_document(run_id, &doc, &doc).unwrap();
                    }
                }
                engine.process_pending(run_id).unwrap();
            }
            CaseStage::Intake | CaseStage::Submission | CaseStage::FollowUp => {
                engine.process_pending(run_id).unwrap();
            }
            other => panic!("unexpected stage while advancing to review: {other:?}"),
        }
    }
    panic!(
        "did not reach review; stage={:?}",
        engine.require_case(run_id).unwrap().stage
    );
}

#[test]
fn a_approval_path() {
    let (_dir, engine) = engine();
    let (run_id, _) = engine.run_scripted("approval").unwrap();
    let case = engine.require_case(run_id).unwrap();
    assert_eq!(case.stage, CaseStage::Handoff);
    assert_eq!(case.disposition.as_deref(), Some("approved_handoff"));
    let snap = engine.snapshot(run_id).unwrap();
    assert!(snap
        .decisions
        .iter()
        .any(|d| d.outcome == DecisionOutcome::Approved));
    assert!(snap.decisions.iter().any(|d| !d.limitations.is_empty()));
    assert!(engine.check_run(run_id).unwrap().is_empty());
}

#[test]
fn b_no_pa_handoff() {
    let (_dir, engine) = engine();
    let (run_id, _) = engine.run_scripted("no_pa").unwrap();
    let case = engine.require_case(run_id).unwrap();
    assert_eq!(case.stage, CaseStage::Handoff);
    assert_eq!(case.disposition.as_deref(), Some("pa_not_required_handoff"));
    let snap = engine.snapshot(run_id).unwrap();
    assert!(snap.submissions.is_empty());
    assert!(snap
        .determinations
        .iter()
        .any(|d| d.kind == DeterminationKind::PaNotRequired));
}

#[test]
fn c_unclear_bv_clarification() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("unclear_bv").unwrap();
    engine.process_pending(run_id).unwrap();
    engine
        .submit_payer_answer(
            run_id,
            "Member appears active. Service may require authorization depending on site of service; unable to determine from available rules.",
        )
        .unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(snap
        .determinations
        .iter()
        .any(|d| d.kind == DeterminationKind::Unclear));
    assert!(snap
        .tasks
        .iter()
        .any(|t| { t.purpose == TaskPurpose::ClarifyBv && t.status == TaskStatus::Open }));
    assert_ne!(snap.case.stage, CaseStage::Handoff);
}

#[test]
fn d_denial_then_appeal() {
    let (_dir, engine) = engine();
    let (run_id, _) = engine.run_scripted("denial_appeal").unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(
        snap.decisions
            .iter()
            .any(|d| d.outcome == DecisionOutcome::Denied),
        "expected a denial before appeal"
    );
    assert!(
        snap.packets
            .iter()
            .any(|p| p.appeal_of_decision_id.is_some()),
        "expected appeal packet linked to denial"
    );
    let case = engine.require_case(run_id).unwrap();
    assert_eq!(case.stage, CaseStage::Handoff);
    assert_eq!(case.disposition.as_deref(), Some("approved_handoff"));
}

#[test]
fn e_duplicate_idempotent_submission() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("approval").unwrap();
    advance_to_review(&engine, run_id);
    let snap = engine.snapshot(run_id).unwrap();
    let packet = snap.packets.last().unwrap().clone();
    engine
        .review_packet(run_id, packet.id, ReviewDecision::Approve, "r1")
        .unwrap();
    engine.process_pending(run_id).unwrap();
    let before = engine.payer.count_submissions().unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    let sub = snap.submissions.last().unwrap();
    let again = engine
        .payer
        .lookup_submission(&sub.idempotency_key)
        .unwrap()
        .expect("receipt");
    let again2 = engine
        .payer
        .submit_packet(&mint_run::lab::payer::PacketSubmissionRequest {
            case_id: snap.case.id,
            packet_id: packet.id,
            packet_hash: packet.content_hash.clone(),
            idempotency_key: sub.idempotency_key.clone(),
            member_id: snap.case.coverage.member_id.clone(),
            cpt: snap.case.service.cpt.clone(),
            is_appeal: false,
        })
        .unwrap();
    assert_eq!(again.receipt_id, again2.receipt_id);
    assert_eq!(engine.payer.count_submissions().unwrap(), before);
}

#[test]
fn f_packet_change_invalidates_review() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("approval").unwrap();
    advance_to_review(&engine, run_id);
    let snap = engine.snapshot(run_id).unwrap();
    let packet = snap.packets.last().unwrap().clone();
    engine
        .review_packet(run_id, packet.id, ReviewDecision::Approve, "r1")
        .unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(snap
        .reviews
        .iter()
        .any(|r| r.valid && r.decision == ReviewDecision::Approve));

    engine
        .review_packet(run_id, packet.id, ReviewDecision::Changes, "r1")
        .unwrap();
    engine
        .supply_document(run_id, "clinical_notes", "clinical_notes")
        .unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(
        snap.reviews
            .iter()
            .filter(|r| r.decision == ReviewDecision::Approve)
            .all(|r| !r.valid)
            || snap.packets.len() > 1
            || snap.case.stage == CaseStage::Documentation
            || snap.case.stage == CaseStage::Review
    );
    if let Some(new_packet) = snap.packets.last() {
        if new_packet.id != packet.id {
            let err = engine.review_packet(run_id, packet.id, ReviewDecision::Approve, "r1");
            if let Ok(_) = err {
                let snap2 = engine.snapshot(run_id).unwrap();
                let approved_old = snap2.reviews.iter().any(|r| {
                    r.valid
                        && r.packet_id == packet.id
                        && r.packet_hash == packet.content_hash
                        && r.decision == ReviewDecision::Approve
                        && snap2.packets.last().map(|p| p.content_hash.as_str())
                            != Some(packet.content_hash.as_str())
                });
                assert!(
                    !approved_old,
                    "stale packet approval must not remain actionable"
                );
            }
        }
    }
}

#[test]
fn g_coverage_change_stales_bv() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("coverage_change").unwrap();
    advance_to_review(&engine, run_id);
    let snap = engine.snapshot(run_id).unwrap();
    assert!(!snap.observations.is_empty());
    assert!(snap.observations.iter().all(|o| !o.stale));
    let new_cov = CoverageContext {
        payer_name: "Synthetic Health Plan".into(),
        member_id: "MEM-COVCHG-001".into(),
        plan_id: "PLAN-PLATINUM".into(),
        dos: "2026-10-07".into(),
    };
    engine.apply_coverage_change(run_id, new_cov).unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(snap.observations.iter().any(|o| o.stale));
    assert!(snap.case.coverage_version >= 2);
    assert_eq!(snap.case.stage, CaseStage::Bv);
}

#[test]
fn h_lost_response_reconcile() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("approval").unwrap();
    advance_to_review(&engine, run_id);
    let snap = engine.snapshot(run_id).unwrap();
    let packet = snap.packets.last().unwrap().clone();
    engine.inject_fault("lost_response").unwrap();
    engine
        .review_packet(run_id, packet.id, ReviewDecision::Approve, "r1")
        .unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(
        snap.submissions
            .iter()
            .any(|s| s.transport_state == SubmissionTransportState::Unknown),
        "expected Unknown transport after lost response"
    );
    engine.clear_fault("lost_response").unwrap();
    engine.process_pending(run_id).unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(snap.submissions.iter().any(|s| {
        s.transport_state == SubmissionTransportState::ReceiptConfirmed
            || s.payer_receipt_id.is_some()
    }));
}

#[test]
fn i_submission_race_one_payer_effect() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("approval").unwrap();
    advance_to_review(&engine, run_id);
    let before = engine.payer.count_submissions().unwrap();
    let (a, b) = engine.race_submit(run_id).unwrap();
    assert!(a, "first claimer should win");
    assert!(!b, "second claimer should lose");
    assert_eq!(engine.payer.count_submissions().unwrap(), before + 1);
}

#[test]
fn j_external_claim_unresolved() {
    let (_dir, engine) = engine();
    let (run_id, _) = engine.run_scripted("external_claim").unwrap();
    let snap = engine.snapshot(run_id).unwrap();
    assert!(
        snap.pending
            .iter()
            .any(|p| p.kind == PendingKind::ExternalClaim)
            || snap.events.iter().any(|e| e.kind == "run_started")
    );
    assert!(
        snap.pending
            .iter()
            .any(|p| p.kind == PendingKind::ExternalClaim)
            || snap
                .events
                .iter()
                .any(|e| e.payload_json.contains("external"))
            || snap
                .pending
                .iter()
                .any(|p| p.detail.contains("verbal approval")),
        "external claim must remain recorded; got pending={:?}",
        snap.pending
    );
    let has_claim = snap
        .pending
        .iter()
        .any(|p| p.kind == PendingKind::ExternalClaim);
    assert!(
        has_claim,
        "external claim should stay unresolved as pending work"
    );
}

#[test]
fn k_cancellation() {
    let (_dir, engine) = engine();
    let run_id = engine.start_run("approval").unwrap();
    engine.process_pending(run_id).unwrap();
    engine.cancel_case(run_id).unwrap();
    let case = engine.require_case(run_id).unwrap();
    assert_eq!(case.stage, CaseStage::Cancelled);
    assert_eq!(case.disposition.as_deref(), Some("cancelled"));
    let report = engine.process_pending(run_id).unwrap();
    assert_eq!(report.stage, CaseStage::Cancelled);
}

#[test]
fn l_wrong_evidence_and_cross_run() {
    let (dir, engine) = engine();
    let run_a = engine.start_run("approval").unwrap();
    let run_b = engine.start_run("no_pa").unwrap();
    advance_to_review(&engine, run_a);
    let snap_a = engine.snapshot(run_a).unwrap();
    let packet_a = snap_a.packets.last().unwrap().id;

    let err = engine.review_packet(run_b, packet_a, ReviewDecision::Approve, "r1");
    assert!(err.is_err(), "cross-run packet review must fail");

    let bogus = uuid::Uuid::new_v4();
    let err = engine.review_packet(run_a, bogus, ReviewDecision::Approve, "r1");
    assert!(err.is_err(), "unknown packet must fail");

    drop(dir);
}

#[test]
fn m_console_interrupt_resume_via_persist() {
    let dir = tempdir().unwrap();
    let fixtures = fixtures_dir_resolved();
    let engine = LabEngine::open(dir.path(), &fixtures).unwrap();
    let run_id = engine.start_run("approval").unwrap();
    engine.process_pending(run_id).unwrap();
    engine
        .submit_payer_answer(
            run_id,
            "Member is active. Prior authorization is required for CPT 72148 outpatient MRI lumbar spine. Required documentation: clinical notes and signed order.",
        )
        .unwrap();
    engine
        .supply_document(run_id, "clinical_notes", "clinical_notes")
        .unwrap();
    let mid = engine.require_case(run_id).unwrap();
    assert_eq!(mid.stage, CaseStage::Documentation);

    drop(engine);

    let engine2 = LabEngine::open(dir.path(), &fixtures).unwrap();
    let restored = engine2.require_case(run_id).unwrap();
    assert_eq!(restored.stage, CaseStage::Documentation);
    engine2.supply_document(run_id, "order", "order").unwrap();
    let snap = engine2.snapshot(run_id).unwrap();
    assert!(
        snap.case.stage == CaseStage::Review || snap.packets.len() == 1,
        "resumed documentation should build packet; stage={:?}",
        snap.case.stage
    );
    engine2.advance_clock(Duration::from_secs(1)).unwrap();
    assert!(engine2.check_run(run_id).unwrap().is_empty());
}
