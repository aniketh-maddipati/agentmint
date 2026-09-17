//! Scripted and optional unverified model agent runners.
//! Used by: workflow for BV tasks. Agents never mutate case status.

use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::lab::domain::{AgentRunRecord, CaseSnapshot, ObservationKind, Role, Task, Uncertainty};
use crate::lab::error::{LabError, LabResult};

pub const PROMPT_VERSION: &str = "bv-scripted-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub question: String,
    pub evidence_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentOutput {
    PendingQuestion(PendingQuestion),
    Observations {
        observations: Vec<DraftObservation>,
        needs_human_review: bool,
    },
    Clarification {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftObservation {
    pub kind: ObservationKind,
    pub statement: String,
    pub uncertainty: Uncertainty,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub record: AgentRunRecord,
    pub output: AgentOutput,
}

pub trait AgentRunner: Send + Sync {
    fn run_bv(&self, task: &Task, snapshot: &CaseSnapshot) -> LabResult<AgentRunResult>;
}

#[derive(Debug, Default)]
pub struct ScriptedAgentRunner;

impl AgentRunner for ScriptedAgentRunner {
    fn run_bv(&self, task: &Task, snapshot: &CaseSnapshot) -> LabResult<AgentRunResult> {
        let source_blobs: Vec<String> = snapshot
            .conversation
            .iter()
            .map(|m| m.text.clone())
            .chain(std::iter::once(task.context_json.clone()))
            .collect();

        if let Some(injection) = detect_injection(&source_blobs) {
            let obs = DraftObservation {
                kind: ObservationKind::InjectionAttempt,
                statement: format!(
                    "Possible prompt-injection content detected in source material: {injection}"
                ),
                uncertainty: Uncertainty::Known,
                evidence_refs: vec!["conversation".into()],
            };
            return Ok(build_result(
                task,
                snapshot,
                AgentOutput::Observations {
                    observations: vec![obs],
                    needs_human_review: true,
                },
                json!([{"tool":"detect_injection","hit":true}]),
            ));
        }

        let payer_msgs: Vec<_> = snapshot
            .conversation
            .iter()
            .filter(|m| m.role == Role::Payer)
            .collect();

        if payer_msgs.is_empty() {
            return Ok(build_result(
                task,
                snapshot,
                AgentOutput::PendingQuestion(PendingQuestion {
                    question: format!(
                        "Is prior authorization required for CPT {} on plan {} for DOS {}?",
                        snapshot.case.service.cpt,
                        snapshot.case.coverage.plan_id,
                        snapshot.case.coverage.dos
                    ),
                    evidence_hint: "payer_bv_response".into(),
                }),
                json!([{"tool":"ask_payer","status":"pending"}]),
            ));
        }

        let joined = payer_msgs
            .iter()
            .map(|m| m.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        if joined.trim().is_empty() || joined.to_lowercase().contains("garbled") {
            return Ok(build_result(
                task,
                snapshot,
                AgentOutput::Clarification {
                    message: "Payer response malformed or unsupported; need clarification.".into(),
                },
                json!([{"tool":"parse_payer","status":"malformed"}]),
            ));
        }

        let evidence_refs: Vec<String> =
            payer_msgs.iter().map(|m| format!("msg:{}", m.id)).collect();

        let observations = interpret_payer_text(&joined, &evidence_refs);
        let needs_human_review = observations.iter().any(|o| {
            matches!(o.kind, ObservationKind::InjectionAttempt)
                || (o.uncertainty == Uncertainty::Unknown
                    && joined.to_lowercase().contains("may require"))
        });

        Ok(build_result(
            task,
            snapshot,
            AgentOutput::Observations {
                observations,
                needs_human_review,
            },
            json!([{"tool":"parse_payer","status":"ok"}]),
        ))
    }
}

fn detect_injection(blobs: &[String]) -> Option<String> {
    const MARKERS: &[&str] = &[
        "ignore previous instructions",
        "disregard all prior",
        "system: you are now",
        "[[inject]]",
        "override safety",
    ];
    for blob in blobs {
        let lower = blob.to_lowercase();
        for marker in MARKERS {
            if lower.contains(marker) {
                return Some((*marker).to_string());
            }
        }
    }
    None
}

fn interpret_payer_text(text: &str, evidence_refs: &[String]) -> Vec<DraftObservation> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();

    if lower.contains("inactive") || lower.contains("not eligible") || lower.contains("terminated")
    {
        out.push(DraftObservation {
            kind: ObservationKind::Eligibility,
            statement: "Member appears inactive or ineligible for DOS.".into(),
            uncertainty: Uncertainty::Known,
            evidence_refs: evidence_refs.to_vec(),
        });
        return out;
    }

    if lower.contains("not covered") || lower.contains("exclusion") {
        out.push(DraftObservation {
            kind: ObservationKind::Coverage,
            statement: "Service appears not covered under the plan.".into(),
            uncertainty: Uncertainty::Known,
            evidence_refs: evidence_refs.to_vec(),
        });
        return out;
    }

    if lower.contains("conflict") || (lower.contains("required") && lower.contains("not required"))
    {
        out.push(DraftObservation {
            kind: ObservationKind::PaRequirement,
            statement: "Conflicting PA requirement signals in payer response.".into(),
            uncertainty: Uncertainty::Unknown,
            evidence_refs: evidence_refs.to_vec(),
        });
        return out;
    }

    if lower.contains("may require")
        || lower.contains("unclear")
        || lower.contains("unable to determine")
    {
        out.push(DraftObservation {
            kind: ObservationKind::PaRequirement,
            statement: "PA requirement remains unclear from payer language.".into(),
            uncertainty: Uncertainty::Unknown,
            evidence_refs: evidence_refs.to_vec(),
        });
        return out;
    }

    if lower.contains("not required") || lower.contains("no prior authorization") {
        out.push(DraftObservation {
            kind: ObservationKind::Eligibility,
            statement: "Member appears active for DOS.".into(),
            uncertainty: Uncertainty::Known,
            evidence_refs: evidence_refs.to_vec(),
        });
        out.push(DraftObservation {
            kind: ObservationKind::PaRequirement,
            statement: "Prior authorization is not required for the requested service.".into(),
            uncertainty: Uncertainty::Known,
            evidence_refs: evidence_refs.to_vec(),
        });
        return out;
    }

    if lower.contains("prior authorization is required")
        || lower.contains("pa is required")
        || lower.contains("authorization required")
    {
        out.push(DraftObservation {
            kind: ObservationKind::Eligibility,
            statement: "Member appears active for DOS.".into(),
            uncertainty: Uncertainty::Known,
            evidence_refs: evidence_refs.to_vec(),
        });
        out.push(DraftObservation {
            kind: ObservationKind::PaRequirement,
            statement: "Prior authorization is required for CPT 72148.".into(),
            uncertainty: Uncertainty::Known,
            evidence_refs: evidence_refs.to_vec(),
        });
        if lower.contains("clinical notes") || lower.contains("documentation") {
            out.push(DraftObservation {
                kind: ObservationKind::DocumentationNeed,
                statement: "Clinical documentation is required for PA submission.".into(),
                uncertainty: Uncertainty::Known,
                evidence_refs: evidence_refs.to_vec(),
            });
        }
        return out;
    }

    out.push(DraftObservation {
        kind: ObservationKind::Clarification,
        statement: "Payer response did not yield a supported BV conclusion.".into(),
        uncertainty: Uncertainty::Unknown,
        evidence_refs: evidence_refs.to_vec(),
    });
    out
}

fn build_result(
    task: &Task,
    snapshot: &CaseSnapshot,
    output: AgentOutput,
    tool_calls: serde_json::Value,
) -> AgentRunResult {
    let evidence_refs = match &output {
        AgentOutput::Observations { observations, .. } => observations
            .iter()
            .flat_map(|o| o.evidence_refs.clone())
            .collect(),
        AgentOutput::PendingQuestion(q) => vec![q.evidence_hint.clone()],
        AgentOutput::Clarification { .. } => Vec::new(),
    };
    let structured_output_json =
        serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_string());
    AgentRunResult {
        record: AgentRunRecord {
            id: Uuid::new_v4(),
            case_id: snapshot.case.id,
            task_id: task.id,
            prompt_version: PROMPT_VERSION.into(),
            model_id: "scripted".into(),
            context_version: snapshot.case.coverage_version + snapshot.case.service_version,
            tool_calls_json: tool_calls.to_string(),
            structured_output_json,
            evidence_refs,
            created_at: snapshot.case.updated_at,
        },
        output,
    }
}

#[derive(Debug, Default)]
pub struct OpenAiAgentRunner;

impl AgentRunner for OpenAiAgentRunner {
    fn run_bv(&self, _task: &Task, _snapshot: &CaseSnapshot) -> LabResult<AgentRunResult> {
        match std::env::var("OPENAI_API_KEY") {
            Ok(key) if !key.is_empty() => Err(LabError::Unverified(
                "OpenAiAgentRunner is a stub; live model calls are not implemented".into(),
            )),
            _ => Err(LabError::Unverified(
                "OPENAI_API_KEY not set; OpenAiAgentRunner will not fabricate success".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::domain::*;
    use chrono::Utc;

    fn empty_snapshot() -> CaseSnapshot {
        let now = Utc::now();
        CaseSnapshot {
            case: Case {
                id: Uuid::new_v4(),
                run_id: Uuid::new_v4(),
                scenario_id: "t".into(),
                workflow_version: WORKFLOW_VERSION.into(),
                stage: CaseStage::Bv,
                service: ServiceContext {
                    cpt: "72148".into(),
                    diagnosis: "M54.5".into(),
                    site: "outpatient".into(),
                },
                coverage: CoverageContext {
                    payer_name: "p".into(),
                    member_id: "m".into(),
                    plan_id: "pl".into(),
                    dos: "2026-10-01".into(),
                },
                service_version: 1,
                coverage_version: 1,
                disposition: None,
                paused_from: None,
                created_at: now,
                updated_at: now,
            },
            tasks: vec![],
            attempts: vec![],
            conversation: vec![],
            observations: vec![],
            determinations: vec![],
            documents: vec![],
            packets: vec![],
            reviews: vec![],
            submissions: vec![],
            decisions: vec![],
            events: vec![],
            pending: vec![],
            agent_runs: vec![],
        }
    }

    #[test]
    fn asks_question_when_no_payer_answer() {
        let agent = ScriptedAgentRunner;
        let snap = empty_snapshot();
        let task = Task {
            id: Uuid::new_v4(),
            case_id: snap.case.id,
            purpose: TaskPurpose::BenefitsVerification,
            status: TaskStatus::Open,
            owner: Role::Operator,
            context_json: "{}".into(),
            created_at: Utc::now(),
            completed_at: None,
        };
        let result = agent.run_bv(&task, &snap).unwrap();
        assert!(matches!(result.output, AgentOutput::PendingQuestion(_)));
    }
}
