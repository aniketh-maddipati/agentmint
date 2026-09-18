//! Structured inspect view distinguishing fact / claim / inference / unknown.
//! Used by: `mint lab inspect` and console /evidence.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::lab::domain::*;
use crate::lab::error::LabResult;
use crate::lab::workflow::LabEngine;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicClass {
    RecordedFact,
    AgentClaim,
    Inference,
    Unknown,
    Hypothetical,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectItem {
    pub class: EpistemicClass,
    pub kind: String,
    pub id: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InspectReport {
    pub run_id: Uuid,
    pub stage: CaseStage,
    pub disposition: Option<String>,
    pub outstanding_work: Vec<String>,
    pub items: Vec<InspectItem>,
    pub packet_versions: Vec<String>,
    pub reviews: Vec<String>,
    pub submissions: Vec<String>,
    pub decisions: Vec<String>,
}

pub fn inspect_run(engine: &LabEngine, run_id: Uuid) -> LabResult<InspectReport> {
    let snap = engine.snapshot(run_id)?;
    let mut items = Vec::new();

    items.push(InspectItem {
        class: EpistemicClass::RecordedFact,
        kind: "case".into(),
        id: snap.case.id.to_string(),
        summary: format!(
            "stage={:?} service={} coverage_v={} service_v={}",
            snap.case.stage,
            snap.case.service.cpt,
            snap.case.coverage_version,
            snap.case.service_version
        ),
        evidence_refs: vec![],
    });

    for msg in &snap.conversation {
        items.push(InspectItem {
            class: EpistemicClass::RecordedFact,
            kind: "conversation".into(),
            id: msg.id.to_string(),
            summary: format!("{:?}: {}", msg.role, truncate(&msg.text, 120)),
            evidence_refs: vec![format!("msg:{}", msg.id)],
        });
    }

    for obs in &snap.observations {
        let class = if obs.source.contains("agent") {
            EpistemicClass::AgentClaim
        } else {
            EpistemicClass::RecordedFact
        };
        let class = if obs.uncertainty == Uncertainty::Unknown {
            EpistemicClass::Unknown
        } else {
            class
        };
        items.push(InspectItem {
            class,
            kind: format!("observation:{:?}", obs.kind),
            id: obs.id.to_string(),
            summary: format!(
                "{}{}",
                if obs.stale { "[stale] " } else { "" },
                obs.statement
            ),
            evidence_refs: obs.evidence_refs.clone(),
        });
    }

    for det in &snap.determinations {
        items.push(InspectItem {
            class: EpistemicClass::Inference,
            kind: format!("determination:{:?}", det.kind),
            id: det.id.to_string(),
            summary: det.rationale.clone(),
            evidence_refs: det
                .observation_ids
                .iter()
                .map(|id| id.to_string())
                .collect(),
        });
    }

    for doc in &snap.documents {
        items.push(InspectItem {
            class: EpistemicClass::RecordedFact,
            kind: "document".into(),
            id: doc.id.to_string(),
            summary: format!(
                "request={} fixture={} hash={}",
                doc.request_id, doc.fixture_name, doc.content_hash
            ),
            evidence_refs: vec![doc.content_hash.clone()],
        });
    }

    for packet in &snap.packets {
        items.push(InspectItem {
            class: EpistemicClass::RecordedFact,
            kind: "packet".into(),
            id: packet.id.to_string(),
            summary: format!("v{} hash={}", packet.version, packet.content_hash),
            evidence_refs: packet
                .document_ids
                .iter()
                .map(|id| id.to_string())
                .collect(),
        });
    }

    for review in &snap.reviews {
        items.push(InspectItem {
            class: EpistemicClass::RecordedFact,
            kind: "review".into(),
            id: review.id.to_string(),
            summary: format!(
                "{:?} valid={} hash={}",
                review.decision, review.valid, review.packet_hash
            ),
            evidence_refs: vec![review.packet_id.to_string()],
        });
    }

    for sub in &snap.submissions {
        items.push(InspectItem {
            class: match sub.transport_state {
                SubmissionTransportState::Unknown => EpistemicClass::Unknown,
                _ => EpistemicClass::RecordedFact,
            },
            kind: "submission".into(),
            id: sub.id.to_string(),
            summary: format!(
                "{:?} receipt={:?}",
                sub.transport_state, sub.payer_receipt_id
            ),
            evidence_refs: vec![sub.idempotency_key.clone()],
        });
    }

    for decision in &snap.decisions {
        items.push(InspectItem {
            class: EpistemicClass::RecordedFact,
            kind: "decision".into(),
            id: decision.id.to_string(),
            summary: format!(
                "{:?} limitations={:?}",
                decision.outcome, decision.limitations
            ),
            evidence_refs: vec![decision.submission_id.to_string()],
        });
    }

    for run in &snap.agent_runs {
        let plan = crate::lab::plan::plan_from_record(run);
        let repair = crate::lab::plan::repair_from_record(run);
        let plan_goal = plan.as_ref().map(|p| p.goal.as_str()).unwrap_or("none");
        let repairs = repair.as_ref().map(|r| r.count).unwrap_or(0);
        items.push(InspectItem {
            class: EpistemicClass::AgentClaim,
            kind: "agent_run".into(),
            id: run.id.to_string(),
            summary: format!(
                "model={} prompt={} context_v={} plan={} repairs={}",
                run.model_id, run.prompt_version, run.context_version, plan_goal, repairs
            ),
            evidence_refs: run.evidence_refs.clone(),
        });
        if let Some(plan) = plan {
            items.push(InspectItem {
                class: EpistemicClass::AgentClaim,
                kind: "bv_plan".into(),
                id: format!("{}:plan", run.id),
                summary: format!(
                    "{} steps={} stop={:?}",
                    plan.plan_version,
                    plan.steps.len(),
                    plan.stop_conditions
                ),
                evidence_refs: plan.steps.iter().map(|s| s.action.clone()).collect(),
            });
        }
    }

    items.push(InspectItem {
        class: EpistemicClass::Hypothetical,
        kind: "note".into(),
        id: "lab".into(),
        summary: "Synthetic lab only; not clinical guidance or payment guarantee.".into(),
        evidence_refs: vec![],
    });

    let outstanding_work = snap
        .pending
        .iter()
        .map(|p| format!("{:?} {} — {}", p.kind, p.ref_id, p.detail))
        .chain(
            snap.tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Open)
                .map(|t| format!("task {:?}/{:?}", t.purpose, t.owner)),
        )
        .collect();

    Ok(InspectReport {
        run_id,
        stage: snap.case.stage,
        disposition: snap.case.disposition.clone(),
        outstanding_work,
        packet_versions: snap
            .packets
            .iter()
            .map(|p| format!("v{} {} {}", p.version, p.id, p.content_hash))
            .collect(),
        reviews: snap
            .reviews
            .iter()
            .map(|r| format!("{:?} valid={} {}", r.decision, r.valid, r.id))
            .collect(),
        submissions: snap
            .submissions
            .iter()
            .map(|s| {
                format!(
                    "{:?} receipt={:?} {}",
                    s.transport_state, s.payer_receipt_id, s.id
                )
            })
            .collect(),
        decisions: snap
            .decisions
            .iter()
            .map(|d| format!("{:?} {}", d.outcome, d.id))
            .collect(),
        items,
    })
}

pub fn inspect_text(report: &InspectReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "run={} stage={:?} disposition={:?}\n",
        report.run_id, report.stage, report.disposition
    ));
    out.push_str("outstanding:\n");
    for w in &report.outstanding_work {
        out.push_str(&format!("  - {w}\n"));
    }
    out.push_str("items:\n");
    for item in &report.items {
        out.push_str(&format!(
            "  [{:?}] {} {} — {}\n",
            item.class, item.kind, item.id, item.summary
        ));
    }
    out
}

pub fn events_json(engine: &LabEngine, run_id: Uuid) -> LabResult<serde_json::Value> {
    let snap = engine.snapshot(run_id)?;
    Ok(json!(snap.events))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let trimmed: String = s.chars().take(max).collect();
        format!("{trimmed}…")
    }
}
