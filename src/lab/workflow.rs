//! PA/BV lab workflow engine for medical-mri-pa-v1.
//! Used by: CLI, console, and acceptance tests.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::lab::agent::{AgentOutput, AgentRunner, ScriptedAgentRunner};
use crate::lab::clock::{Clock, MutableClock};
use crate::lab::domain::*;
use crate::lab::error::{LabError, LabResult};
use crate::lab::payer::{FakePayer, PacketSubmissionRequest, PayerAdapter};
use crate::lab::scenarios::{load_scenario_from, ScenarioFixture};
use crate::lab::store::CaseStore;

const MAX_FOLLOW_UP_CHECKS: u32 = 3;
const FOLLOW_UP_SECS: i64 = 3600;

pub struct LabEngine {
    pub store: CaseStore,
    pub payer: Arc<FakePayer>,
    clock: MutableClock,
    agent: Arc<dyn AgentRunner>,
    fixtures_dir: std::path::PathBuf,
    scenario_cache: std::sync::Mutex<std::collections::HashMap<Uuid, ScenarioFixture>>,
    faults: std::sync::Mutex<HashSet<String>>,
    follow_up_counts: std::sync::Mutex<std::collections::HashMap<Uuid, u32>>,
}

#[derive(Debug, Clone)]
pub struct TickReport {
    pub stage: CaseStage,
    pub happened: Vec<String>,
    pub next_owner: Option<Role>,
    pub next_action: Option<String>,
    pub evidence: Vec<String>,
    pub blockers: Vec<String>,
}

impl LabEngine {
    pub fn open(data_dir: &Path, fixtures_dir: &Path) -> LabResult<Self> {
        std::fs::create_dir_all(data_dir)?;
        let store = CaseStore::open(&data_dir.join("lab.db"))?;
        let payer = Arc::new(FakePayer::open(&data_dir.join("payer.db"))?);
        let clock = MutableClock::new(Utc::now());
        Ok(Self {
            store,
            payer,
            clock,
            agent: Arc::new(ScriptedAgentRunner),
            fixtures_dir: fixtures_dir.to_path_buf(),
            scenario_cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            faults: std::sync::Mutex::new(HashSet::new()),
            follow_up_counts: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn with_agent(mut self, agent: Arc<dyn AgentRunner>) -> Self {
        self.agent = agent;
        self
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    pub fn advance_clock(&self, duration: Duration) -> LabResult<()> {
        self.clock.advance(duration)
    }

    pub fn inject_fault(&self, name: &str) -> LabResult<()> {
        let mut guard = self
            .faults
            .lock()
            .map_err(|_| LabError::Storage("faults lock poisoned".into()))?;
        guard.insert(name.to_string());
        Ok(())
    }

    pub fn clear_fault(&self, name: &str) -> LabResult<()> {
        let mut guard = self
            .faults
            .lock()
            .map_err(|_| LabError::Storage("faults lock poisoned".into()))?;
        guard.remove(name);
        Ok(())
    }

    fn has_fault(&self, name: &str) -> bool {
        self.faults
            .lock()
            .map(|g| g.contains(name))
            .unwrap_or(false)
    }

    fn cache_scenario(&self, run_id: Uuid, fixture: ScenarioFixture) -> LabResult<()> {
        let mut guard = self
            .scenario_cache
            .lock()
            .map_err(|_| LabError::Storage("scenario cache poisoned".into()))?;
        guard.insert(run_id, fixture);
        Ok(())
    }

    fn scenario_for(&self, run_id: Uuid, scenario_id: &str) -> LabResult<ScenarioFixture> {
        {
            let guard = self
                .scenario_cache
                .lock()
                .map_err(|_| LabError::Storage("scenario cache poisoned".into()))?;
            if let Some(f) = guard.get(&run_id) {
                return Ok(f.clone());
            }
        }
        let fixture = load_scenario_from(&self.fixtures_dir, scenario_id)?;
        self.cache_scenario(run_id, fixture.clone())?;
        Ok(fixture)
    }

    pub fn start_run(&self, scenario_id: &str) -> LabResult<Uuid> {
        let fixture = load_scenario_from(&self.fixtures_dir, scenario_id)?;
        self.configure_payer_from_fixture(&fixture)?;
        let now = self.now();
        let run_id = Uuid::new_v4();
        let case_id = Uuid::new_v4();
        let case = Case {
            id: case_id,
            run_id,
            scenario_id: fixture.id.clone(),
            workflow_version: fixture.workflow_version.clone(),
            stage: CaseStage::Intake,
            service: fixture.service.clone(),
            coverage: fixture.coverage.clone(),
            service_version: 1,
            coverage_version: 1,
            disposition: None,
            paused_from: None,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_case(&case)?;
        self.store.append_event(
            case_id,
            "run_started",
            &json!({"scenario": scenario_id, "run_id": run_id}),
            now,
        )?;
        self.cache_scenario(run_id, fixture)?;
        Ok(run_id)
    }

    fn configure_payer_from_fixture(&self, fixture: &ScenarioFixture) -> LabResult<()> {
        let bv_text = fixture
            .scripted_payer_answers
            .first()
            .cloned()
            .unwrap_or_else(|| {
                "Member is active. Prior authorization is required for CPT 72148.".into()
            });
        let kind = fixture.hidden_facts.bv_outcome.clone();
        self.payer.configure_bv(&bv_text, kind.as_deref())?;
        let decision = fixture
            .hidden_facts
            .decision_outcome
            .clone()
            .unwrap_or_else(|| "approved".into());
        let limitations = if fixture.hidden_facts.limitations.is_empty() {
            vec![
                "outpatient site of service only".into(),
                "authorization valid 30 days".into(),
            ]
        } else {
            fixture.hidden_facts.limitations.clone()
        };
        self.payer.configure_outcome(
            &decision,
            limitations,
            fixture.hidden_facts.denial_reason.clone(),
        )?;
        Ok(())
    }

    pub fn require_case(&self, run_id: Uuid) -> LabResult<Case> {
        self.store
            .get_case_by_run(run_id)?
            .ok_or_else(|| LabError::NotFound(format!("run {run_id}")))
    }

    pub fn snapshot(&self, run_id: Uuid) -> LabResult<CaseSnapshot> {
        let case = self.require_case(run_id)?;
        self.store.load_snapshot(case.id)
    }

    pub fn tick(&self, run_id: Uuid) -> LabResult<TickReport> {
        self.process_pending(run_id)
    }

    pub fn process_pending(&self, run_id: Uuid) -> LabResult<TickReport> {
        let mut case = self.require_case(run_id)?;
        if case.stage == CaseStage::Cancelled {
            return Ok(TickReport {
                stage: case.stage,
                happened: vec!["case cancelled".into()],
                next_owner: None,
                next_action: None,
                evidence: vec![],
                blockers: vec!["cancelled".into()],
            });
        }
        if case.stage == CaseStage::Paused {
            return Ok(TickReport {
                stage: case.stage,
                happened: vec!["case paused".into()],
                next_owner: Some(Role::Operator),
                next_action: Some("resume".into()),
                evidence: vec![],
                blockers: vec!["paused".into()],
            });
        }

        let mut happened = Vec::new();
        let fixture = self.scenario_for(run_id, &case.scenario_id)?;

        match case.stage {
            CaseStage::Intake => {
                self.drive_intake(&mut case, &fixture, &mut happened)?;
            }
            CaseStage::Bv => {
                self.drive_bv(&mut case, &fixture, &mut happened)?;
            }
            CaseStage::Documentation => {
                self.drive_documentation(&mut case, &fixture, &mut happened)?;
            }
            CaseStage::Review => {
                happened.push("awaiting packet review".into());
            }
            CaseStage::Submission => {
                self.drive_submission(&mut case, &fixture, &mut happened)?;
            }
            CaseStage::FollowUp => {
                self.drive_follow_up(&mut case, &mut happened)?;
            }
            CaseStage::Decision => {
                self.drive_decision(&mut case, &fixture, &mut happened)?;
            }
            CaseStage::Appeal => {
                happened.push("appeal in progress".into());
            }
            CaseStage::Handoff | CaseStage::ManualDisposition => {
                happened.push(format!("terminal stage {:?}", case.stage));
            }
            CaseStage::Cancelled | CaseStage::Paused => {}
        }

        let snap = self.store.load_snapshot(case.id)?;
        let (next_owner, next_action, blockers) = infer_next(&snap);
        let evidence = snap
            .observations
            .iter()
            .filter(|o| !o.stale)
            .map(|o| format!("{}:{}", o.id, o.kind_label()))
            .collect();

        Ok(TickReport {
            stage: snap.case.stage,
            happened,
            next_owner,
            next_action,
            evidence,
            blockers,
        })
    }

    fn set_stage(&self, case: &mut Case, stage: CaseStage, note: &str) -> LabResult<()> {
        case.stage = stage;
        case.updated_at = self.now();
        self.store.update_case(case)?;
        self.store.append_event(
            case.id,
            "stage_changed",
            &json!({"stage": stage, "note": note}),
            case.updated_at,
        )?;
        Ok(())
    }

    fn drive_intake(
        &self,
        case: &mut Case,
        fixture: &ScenarioFixture,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        if !fixture.intake_complete && !fixture.missing_intake_fields.is_empty() {
            let open_missing = self.store.load_snapshot(case.id)?.tasks.iter().any(|t| {
                t.purpose == TaskPurpose::CollectMissingInfo && t.status == TaskStatus::Open
            });
            if !open_missing {
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::CollectMissingInfo,
                    status: TaskStatus::Open,
                    owner: Role::Customer,
                    context_json: json!({"fields": fixture.missing_intake_fields}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                happened.push("created missing-info task".into());
                return Ok(());
            }
            happened.push("waiting on missing intake info".into());
            return Ok(());
        }

        if let Some(inj) = &fixture.injection_in_source {
            let msg = ConversationMessage {
                id: Uuid::new_v4(),
                case_id: case.id,
                role: Role::Customer,
                text: inj.clone(),
                created_at: self.now(),
            };
            self.store.insert_conversation(&msg)?;
            happened.push("recorded injection-bearing source content".into());
        }

        if let Some(claim) = &fixture.external_claim {
            let pending = PendingWork {
                id: Uuid::new_v4(),
                case_id: case.id,
                kind: PendingKind::ExternalClaim,
                ref_id: "customer_claim".into(),
                detail: claim.clone(),
                due_at: None,
                created_at: self.now(),
            };
            self.store.insert_pending(&pending)?;
            happened.push("recorded unresolved external customer claim".into());
        }

        self.set_stage(case, CaseStage::Bv, "intake complete")?;
        happened.push("intake complete; entering BV".into());
        let task = Task {
            id: Uuid::new_v4(),
            case_id: case.id,
            purpose: TaskPurpose::BenefitsVerification,
            status: TaskStatus::Open,
            owner: Role::Operator,
            context_json: json!({
                "service": case.service,
                "coverage": case.coverage,
                "coverage_version": case.coverage_version,
                "service_version": case.service_version
            })
            .to_string(),
            created_at: self.now(),
            completed_at: None,
        };
        self.store.insert_task(&task)?;
        Ok(())
    }

    fn drive_bv(
        &self,
        case: &mut Case,
        fixture: &ScenarioFixture,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        let snap = self.store.load_snapshot(case.id)?;
        let task = snap
            .tasks
            .iter()
            .rev()
            .find(|t| {
                t.purpose == TaskPurpose::BenefitsVerification && t.status == TaskStatus::Open
            })
            .cloned()
            .ok_or_else(|| LabError::Invalid("no open BV task".into()))?;

        if snap
            .pending
            .iter()
            .any(|p| p.kind == PendingKind::AgentQuestion)
        {
            happened.push("awaiting payer answer to agent question".into());
            return Ok(());
        }

        let result = self.agent.run_bv(&task, &snap)?;
        let mut record = result.record;
        record.created_at = self.now();
        self.store.insert_agent_run(&record)?;

        match result.output {
            AgentOutput::PendingQuestion(q) => {
                let pending = PendingWork {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    kind: PendingKind::AgentQuestion,
                    ref_id: task.id.to_string(),
                    detail: q.question.clone(),
                    due_at: None,
                    created_at: self.now(),
                };
                self.store.insert_pending(&pending)?;
                self.store.append_event(
                    case.id,
                    "agent_question",
                    &json!({"question": q.question}),
                    self.now(),
                )?;
                happened.push("agent asked payer question".into());
            }
            AgentOutput::Clarification { message } => {
                let t = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::ClarifyBv,
                    status: TaskStatus::Open,
                    owner: Role::Operator,
                    context_json: json!({"message": message}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&t)?;
                happened.push("agent requested clarification".into());
            }
            AgentOutput::Observations {
                observations,
                needs_human_review,
            } => {
                let mut obs_ids = Vec::new();
                for draft in observations {
                    let obs = Observation {
                        id: Uuid::new_v4(),
                        case_id: case.id,
                        kind: draft.kind,
                        statement: draft.statement,
                        uncertainty: draft.uncertainty,
                        evidence_refs: draft.evidence_refs,
                        stale: false,
                        source: "scripted_agent".into(),
                        created_at: self.now(),
                    };
                    obs_ids.push(obs.id);
                    self.store.insert_observation(&obs)?;
                }
                happened.push(format!("recorded {} observations", obs_ids.len()));

                let injection_seen = self
                    .store
                    .load_snapshot(case.id)?
                    .observations
                    .iter()
                    .any(|o| o.kind == ObservationKind::InjectionAttempt && !o.stale);
                let injection_in_chat = snap
                    .conversation
                    .iter()
                    .any(|m| m.text.to_lowercase().contains("ignore previous"));
                if (needs_human_review || injection_in_chat) && injection_seen {
                    let t = Task {
                        id: Uuid::new_v4(),
                        case_id: case.id,
                        purpose: TaskPurpose::HumanReview,
                        status: TaskStatus::Open,
                        owner: Role::Reviewer,
                        context_json: json!({"reason":"injection_attempt"}).to_string(),
                        created_at: self.now(),
                        completed_at: None,
                    };
                    self.store.insert_task(&t)?;
                    self.set_stage(case, CaseStage::ManualDisposition, "injection review")?;
                    case.disposition = Some("injection_review".into());
                    case.updated_at = self.now();
                    self.store.update_case(case)?;
                    happened.push("injection attempt escalated to human review".into());
                    self.complete_task(&task)?;
                    return Ok(());
                }

                let fresh = self.store.load_snapshot(case.id)?;
                let determination = synthesize_determination(case, &fresh, fixture)?;
                self.store.insert_determination(&determination)?;
                happened.push(format!("determination {:?}", determination.kind));
                self.complete_task(&task)?;
                self.route_after_determination(case, &determination, fixture, happened)?;
            }
        }
        Ok(())
    }

    fn complete_task(&self, task: &Task) -> LabResult<()> {
        let mut t = task.clone();
        t.status = TaskStatus::Done;
        t.completed_at = Some(self.now());
        self.store.update_task(&t)
    }

    fn route_after_determination(
        &self,
        case: &mut Case,
        det: &Determination,
        fixture: &ScenarioFixture,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        match det.kind {
            DeterminationKind::PaRequired => {
                for doc in &det.required_docs {
                    let pending = PendingWork {
                        id: Uuid::new_v4(),
                        case_id: case.id,
                        kind: PendingKind::DocumentRequest,
                        ref_id: doc.clone(),
                        detail: format!("required document {doc}"),
                        due_at: None,
                        created_at: self.now(),
                    };
                    self.store.insert_pending(&pending)?;
                }
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::CollectDocumentation,
                    status: TaskStatus::Open,
                    owner: Role::Customer,
                    context_json: json!({"required_docs": det.required_docs}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                self.set_stage(case, CaseStage::Documentation, "PA required")?;
                happened.push("entered documentation".into());
                if fixture.auto_supply_docs {
                    for doc in &det.required_docs.clone() {
                        let _ = self.supply_document(case.run_id, doc, doc);
                    }
                    let _ = self.process_pending(case.run_id);
                }
            }
            DeterminationKind::PaNotRequired => {
                case.disposition = Some("pa_not_required_handoff".into());
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::DeliverHandoff,
                    status: TaskStatus::Open,
                    owner: Role::Customer,
                    context_json: json!({"note":"PA not required; not a payment guarantee"})
                        .to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                self.set_stage(case, CaseStage::Handoff, "PA not required")?;
                case.updated_at = self.now();
                self.store.update_case(case)?;
                happened.push("handoff: PA not required".into());
            }
            DeterminationKind::InactiveMember | DeterminationKind::NotCovered => {
                let label = match det.kind {
                    DeterminationKind::InactiveMember => "inactive_member",
                    _ => "not_covered",
                };
                case.disposition = Some(format!("{label}_handoff"));
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::DeliverHandoff,
                    status: TaskStatus::Open,
                    owner: Role::Customer,
                    context_json: json!({"disposition": label}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                self.set_stage(case, CaseStage::Handoff, label)?;
                case.updated_at = self.now();
                self.store.update_case(case)?;
                happened.push(format!("handoff disposition {label}"));
            }
            DeterminationKind::Unclear => {
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::ClarifyBv,
                    status: TaskStatus::Open,
                    owner: Role::Operator,
                    context_json: json!({"reason":"unclear_bv"}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                happened.push("unclear BV — clarification task opened".into());
            }
            DeterminationKind::Conflict => {
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::HumanReview,
                    status: TaskStatus::Open,
                    owner: Role::Reviewer,
                    context_json: json!({"reason":"conflict_bv"}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                self.set_stage(case, CaseStage::Review, "conflict requires review")?;
                happened.push("conflict escalated to review".into());
            }
        }
        Ok(())
    }

    fn drive_documentation(
        &self,
        case: &mut Case,
        _fixture: &ScenarioFixture,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        let snap = self.store.load_snapshot(case.id)?;
        let required: HashSet<String> = snap
            .determinations
            .iter()
            .rev()
            .find(|d| d.kind == DeterminationKind::PaRequired)
            .map(|d| d.required_docs.iter().cloned().collect())
            .unwrap_or_default();
        let supplied: HashSet<String> = snap
            .documents
            .iter()
            .map(|d| d.request_id.clone())
            .collect();
        let missing: Vec<_> = required.difference(&supplied).cloned().collect();
        if !missing.is_empty() {
            happened.push(format!("waiting on docs: {}", missing.join(",")));
            return Ok(());
        }
        self.build_packet(case, None, happened)?;
        Ok(())
    }

    fn build_packet(
        &self,
        case: &mut Case,
        appeal_of: Option<Uuid>,
        happened: &mut Vec<String>,
    ) -> LabResult<Packet> {
        let snap = self.store.load_snapshot(case.id)?;
        let docs: Vec<_> = snap.documents.iter().collect();
        let mut hasher = Sha256::new();
        let mut ids = Vec::new();
        for doc in &docs {
            hasher.update(doc.content_hash.as_bytes());
            ids.push(doc.id);
        }
        let content_hash = hex::encode(hasher.finalize());
        let version = snap.packets.len() as u32 + 1;
        let packet = Packet {
            id: Uuid::new_v4(),
            case_id: case.id,
            version,
            document_ids: ids,
            content_hash: content_hash.clone(),
            appeal_of_decision_id: appeal_of,
            created_at: self.now(),
        };
        self.store.insert_packet(&packet)?;
        self.store.invalidate_all_reviews(case.id)?;
        let pending = PendingWork {
            id: Uuid::new_v4(),
            case_id: case.id,
            kind: PendingKind::Review,
            ref_id: packet.id.to_string(),
            detail: format!("review packet v{version} hash {content_hash}"),
            due_at: None,
            created_at: self.now(),
        };
        self.store.insert_pending(&pending)?;
        let task = Task {
            id: Uuid::new_v4(),
            case_id: case.id,
            purpose: TaskPurpose::ReviewPacket,
            status: TaskStatus::Open,
            owner: Role::Reviewer,
            context_json: json!({"packet_id": packet.id, "hash": content_hash}).to_string(),
            created_at: self.now(),
            completed_at: None,
        };
        self.store.insert_task(&task)?;
        self.set_stage(case, CaseStage::Review, "packet built")?;
        happened.push(format!("built packet v{version}"));
        Ok(packet)
    }

    pub fn supply_document(
        &self,
        run_id: Uuid,
        request_id: &str,
        fixture_name: &str,
    ) -> LabResult<TickReport> {
        let case = self.require_case(run_id)?;
        if case.stage == CaseStage::Cancelled {
            return Err(LabError::Cancelled);
        }
        let fixture = self.scenario_for(run_id, &case.scenario_id)?;
        let content = fixture
            .document_fixtures
            .get(fixture_name)
            .cloned()
            .or_else(|| fixture.document_fixtures.get(request_id).cloned())
            .unwrap_or_else(|| format!("synthetic document {fixture_name}"));
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let content_hash = hex::encode(hasher.finalize());
        let doc = Document {
            id: Uuid::new_v4(),
            case_id: case.id,
            request_id: request_id.to_string(),
            fixture_name: fixture_name.to_string(),
            content_hash,
            content,
            created_at: self.now(),
        };
        self.store.insert_document(&doc)?;
        self.store
            .clear_pending_kind(case.id, PendingKind::DocumentRequest)?;
        let snap = self.store.load_snapshot(case.id)?;
        for req in snap
            .determinations
            .iter()
            .rev()
            .find(|d| d.kind == DeterminationKind::PaRequired)
            .map(|d| d.required_docs.clone())
            .unwrap_or_default()
        {
            if !snap.documents.iter().any(|d| d.request_id == req) && req != request_id {
                let pending = PendingWork {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    kind: PendingKind::DocumentRequest,
                    ref_id: req.clone(),
                    detail: format!("required document {req}"),
                    due_at: None,
                    created_at: self.now(),
                };
                self.store.insert_pending(&pending)?;
            }
        }
        self.store.append_event(
            case.id,
            "document_supplied",
            &json!({"request_id": request_id, "doc_id": doc.id}),
            self.now(),
        )?;
        if case.stage == CaseStage::Documentation || case.stage == CaseStage::Appeal {
            let _ = case;
        }
        self.process_pending(run_id)
    }

    pub fn submit_payer_answer(&self, run_id: Uuid, text: &str) -> LabResult<TickReport> {
        let case = self.require_case(run_id)?;
        let msg = ConversationMessage {
            id: Uuid::new_v4(),
            case_id: case.id,
            role: Role::Payer,
            text: text.to_string(),
            created_at: self.now(),
        };
        self.store.insert_conversation(&msg)?;
        self.store
            .clear_pending_kind(case.id, PendingKind::AgentQuestion)?;
        self.store.append_event(
            case.id,
            "payer_answer",
            &json!({"msg_id": msg.id}),
            self.now(),
        )?;
        self.process_pending(run_id)
    }

    pub fn review_packet(
        &self,
        run_id: Uuid,
        packet_id: Uuid,
        decision: ReviewDecision,
        reviewer: &str,
    ) -> LabResult<TickReport> {
        let mut case = self.require_case(run_id)?;
        let snap = self.store.load_snapshot(case.id)?;
        let packet = snap
            .packets
            .iter()
            .find(|p| p.id == packet_id)
            .ok_or_else(|| LabError::NotFound(format!("packet {packet_id}")))?
            .clone();

        if decision == ReviewDecision::Approve {
            let latest = snap.packets.last().map(|p| p.content_hash.as_str());
            if latest != Some(packet.content_hash.as_str()) {
                return Err(LabError::Conflict(
                    "cannot approve non-current packet hash".into(),
                ));
            }
        }

        let review = Review {
            id: Uuid::new_v4(),
            case_id: case.id,
            packet_id: packet.id,
            packet_hash: packet.content_hash.clone(),
            decision,
            reviewer: reviewer.to_string(),
            valid: true,
            created_at: self.now(),
        };
        self.store.insert_review(&review)?;
        self.store
            .clear_pending_kind(case.id, PendingKind::Review)?;

        match decision {
            ReviewDecision::Approve => {
                self.set_stage(&mut case, CaseStage::Submission, "packet approved")?;
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::SubmitPa,
                    status: TaskStatus::Open,
                    owner: Role::Operator,
                    context_json: json!({"packet_id": packet.id}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
            }
            ReviewDecision::Decline => {
                case.disposition = Some("review_declined".into());
                self.set_stage(&mut case, CaseStage::ManualDisposition, "review declined")?;
                case.updated_at = self.now();
                self.store.update_case(&case)?;
            }
            ReviewDecision::Changes => {
                self.store.invalidate_reviews_for_packet(packet.id)?;
                self.set_stage(&mut case, CaseStage::Documentation, "changes requested")?;
            }
        }
        self.process_pending(run_id)
    }

    fn drive_submission(
        &self,
        case: &mut Case,
        fixture: &ScenarioFixture,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        let snap = self.store.load_snapshot(case.id)?;
        let packet = snap
            .packets
            .last()
            .cloned()
            .ok_or_else(|| LabError::Invalid("no packet to submit".into()))?;

        if let Some(existing) = snap
            .submissions
            .iter()
            .rev()
            .find(|s| s.packet_id == packet.id)
        {
            if existing.transport_state == SubmissionTransportState::ReceiptConfirmed {
                self.set_stage(case, CaseStage::FollowUp, "receipt confirmed")?;
                happened.push("already receipt-confirmed".into());
                return Ok(());
            }
            if existing.transport_state == SubmissionTransportState::Unknown {
                happened.push("submission unknown; use follow-up/reconcile".into());
                self.set_stage(case, CaseStage::FollowUp, "unknown transport")?;
                return Ok(());
            }
            if existing.transport_state == SubmissionTransportState::Attempted {
                happened.push("submission already attempted".into());
                return Ok(());
            }
        }

        let approved = snap.reviews.iter().any(|r| {
            r.valid
                && r.packet_id == packet.id
                && r.packet_hash == packet.content_hash
                && r.decision == ReviewDecision::Approve
        });
        if !approved {
            return Err(LabError::Invalid(
                "submission requires valid approval of exact packet hash".into(),
            ));
        }

        let is_appeal = packet.appeal_of_decision_id.is_some();
        if is_appeal {
            if let Some(outcome) = &fixture.hidden_facts.appeal_decision_outcome {
                self.payer.configure_outcome(
                    outcome,
                    fixture.hidden_facts.limitations.clone(),
                    fixture.hidden_facts.denial_reason.clone(),
                )?;
            }
        }

        let idempotency_key = format!(
            "pa:{}:pkt:{}:{}",
            case.id,
            packet.id,
            if is_appeal { "appeal" } else { "initial" }
        );

        let now = self.now();
        let mut sub = Submission {
            id: Uuid::new_v4(),
            case_id: case.id,
            packet_id: packet.id,
            idempotency_key: idempotency_key.clone(),
            transport_state: SubmissionTransportState::Attempted,
            ownership_generation: 0,
            payer_receipt_id: None,
            detail_json: json!({}).to_string(),
            created_at: now,
            updated_at: now,
        };
        self.store.insert_submission(&sub)?;

        let claimed = self.store.claim_submission(sub.id, 0)?;
        if !claimed {
            happened.push("lost submission claim race".into());
            return Ok(());
        }
        sub.ownership_generation = 1;

        if self.has_fault("lost_response") {
            self.payer.inject_lost_response(&idempotency_key)?;
        }

        let request = PacketSubmissionRequest {
            case_id: case.id,
            packet_id: packet.id,
            packet_hash: packet.content_hash.clone(),
            idempotency_key: idempotency_key.clone(),
            member_id: case.coverage.member_id.clone(),
            cpt: case.service.cpt.clone(),
            is_appeal,
        };

        match self.payer.submit_packet(&request) {
            Ok(receipt) => {
                if receipt.status == "intake_rejected" {
                    sub.transport_state = SubmissionTransportState::IntakeRejected;
                    sub.detail_json = json!({"detail": receipt.detail}).to_string();
                    sub.updated_at = self.now();
                    self.store.update_submission(&sub)?;
                    case.disposition = Some("intake_rejected".into());
                    self.set_stage(case, CaseStage::ManualDisposition, "intake rejected")?;
                    case.updated_at = self.now();
                    self.store.update_case(case)?;
                    happened.push("intake rejected".into());
                } else {
                    sub.transport_state = SubmissionTransportState::ReceiptConfirmed;
                    sub.payer_receipt_id = Some(receipt.receipt_id);
                    sub.detail_json = json!({"detail": receipt.detail}).to_string();
                    sub.updated_at = self.now();
                    self.store.update_submission(&sub)?;
                    self.set_stage(case, CaseStage::FollowUp, "receipt confirmed")?;
                    let due = self.now() + chrono::Duration::seconds(FOLLOW_UP_SECS);
                    let pending = PendingWork {
                        id: Uuid::new_v4(),
                        case_id: case.id,
                        kind: PendingKind::FollowUp,
                        ref_id: sub.id.to_string(),
                        detail: "lookup decision".into(),
                        due_at: Some(due),
                        created_at: self.now(),
                    };
                    self.store.insert_pending(&pending)?;
                    happened.push("submission receipt confirmed".into());
                }
            }
            Err(LabError::Unverified(msg)) => {
                if let Ok(Some(found)) = self.payer.lookup_submission(&idempotency_key) {
                    sub.transport_state = SubmissionTransportState::Unknown;
                    sub.payer_receipt_id = Some(found.receipt_id);
                    sub.detail_json = json!({"lost": msg, "reconcile_hint": true}).to_string();
                } else {
                    sub.transport_state = SubmissionTransportState::Unknown;
                    sub.detail_json = json!({"lost": msg}).to_string();
                }
                sub.updated_at = self.now();
                self.store.update_submission(&sub)?;
                self.set_stage(case, CaseStage::FollowUp, "lost response")?;
                happened.push("submission unknown due to lost response".into());
            }
            Err(err) => return Err(err),
        }
        Ok(())
    }

    pub fn race_submit(&self, run_id: Uuid) -> LabResult<(bool, bool)> {
        let mut case = self.require_case(run_id)?;
        let snap = self.store.load_snapshot(case.id)?;
        let packet = snap
            .packets
            .last()
            .cloned()
            .ok_or_else(|| LabError::Invalid("no packet".into()))?;
        let approved = snap.reviews.iter().any(|r| {
            r.valid
                && r.packet_id == packet.id
                && r.packet_hash == packet.content_hash
                && r.decision == ReviewDecision::Approve
        });
        if !approved {
            let review = Review {
                id: Uuid::new_v4(),
                case_id: case.id,
                packet_id: packet.id,
                packet_hash: packet.content_hash.clone(),
                decision: ReviewDecision::Approve,
                reviewer: "race-test".into(),
                valid: true,
                created_at: self.now(),
            };
            self.store.insert_review(&review)?;
        }
        let idempotency_key = format!("pa:{}:pkt:{}:race", case.id, packet.id);
        let now = self.now();
        let sub = Submission {
            id: Uuid::new_v4(),
            case_id: case.id,
            packet_id: packet.id,
            idempotency_key: idempotency_key.clone(),
            transport_state: SubmissionTransportState::Attempted,
            ownership_generation: 0,
            payer_receipt_id: None,
            detail_json: "{}".into(),
            created_at: now,
            updated_at: now,
        };
        self.store.insert_submission(&sub)?;
        let a = self.store.claim_submission(sub.id, 0)?;
        let b = self.store.claim_submission(sub.id, 0)?;
        if a {
            let request = PacketSubmissionRequest {
                case_id: case.id,
                packet_id: packet.id,
                packet_hash: packet.content_hash,
                idempotency_key,
                member_id: case.coverage.member_id.clone(),
                cpt: case.service.cpt.clone(),
                is_appeal: false,
            };
            let _ = self.payer.submit_packet(&request)?;
            case.stage = CaseStage::FollowUp;
            case.updated_at = self.now();
            self.store.update_case(&case)?;
        }
        Ok((a, b))
    }

    fn drive_follow_up(&self, case: &mut Case, happened: &mut Vec<String>) -> LabResult<()> {
        let snap = self.store.load_snapshot(case.id)?;
        let latest_packet_id = snap.packets.last().map(|p| p.id);
        let sub = snap
            .submissions
            .iter()
            .rev()
            .find(|s| latest_packet_id == Some(s.packet_id))
            .cloned()
            .or_else(|| snap.submissions.iter().next_back().cloned())
            .ok_or_else(|| LabError::Invalid("no submission for follow-up".into()))?;

        if sub.transport_state == SubmissionTransportState::Unknown {
            if let Some(receipt) = self.payer.lookup_submission(&sub.idempotency_key)? {
                let mut updated = sub.clone();
                updated.transport_state = SubmissionTransportState::ReceiptConfirmed;
                updated.payer_receipt_id = Some(receipt.receipt_id.clone());
                updated.updated_at = self.now();
                self.store.update_submission(&updated)?;
                happened.push("reconciled unknown submission to receipt-confirmed".into());
            } else {
                happened.push("unknown submission not yet visible at payer".into());
                return Ok(());
            }
        }

        let snap = self.store.load_snapshot(case.id)?;
        let sub = snap
            .submissions
            .iter()
            .next_back()
            .cloned()
            .ok_or_else(|| LabError::Invalid("no submission after reconcile".into()))?;
        let receipt_id = match &sub.payer_receipt_id {
            Some(id) => id.clone(),
            None => {
                happened.push("no receipt id yet".into());
                return Ok(());
            }
        };

        let count = {
            let mut guard = self
                .follow_up_counts
                .lock()
                .map_err(|_| LabError::Storage("follow-up lock poisoned".into()))?;
            let entry = guard.entry(case.id).or_insert(0);
            *entry += 1;
            *entry
        };

        if let Some(decision) = self.payer.retrieve_decision(&receipt_id)? {
            if decision.outcome == "pending" {
                if count >= MAX_FOLLOW_UP_CHECKS {
                    let task = Task {
                        id: Uuid::new_v4(),
                        case_id: case.id,
                        purpose: TaskPurpose::HumanReview,
                        status: TaskStatus::Open,
                        owner: Role::Operator,
                        context_json: json!({"reason":"follow_up_exhausted"}).to_string(),
                        created_at: self.now(),
                        completed_at: None,
                    };
                    self.store.insert_task(&task)?;
                    happened.push("follow-up exhausted; escalated".into());
                } else {
                    happened.push("decision still pending".into());
                }
                return Ok(());
            }
            self.record_decision_from_lookup(case, &sub, &decision, happened)?;
            return Ok(());
        }

        if count >= MAX_FOLLOW_UP_CHECKS {
            let task = Task {
                id: Uuid::new_v4(),
                case_id: case.id,
                purpose: TaskPurpose::HumanReview,
                status: TaskStatus::Open,
                owner: Role::Operator,
                context_json: json!({"reason":"no_decision"}).to_string(),
                created_at: self.now(),
                completed_at: None,
            };
            self.store.insert_task(&task)?;
            happened.push("no decision after bound checks".into());
        } else {
            happened.push("decision not available yet".into());
        }
        Ok(())
    }

    fn record_decision_from_lookup(
        &self,
        case: &mut Case,
        sub: &Submission,
        lookup: &crate::lab::payer::DecisionLookup,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        let snap = self.store.load_snapshot(case.id)?;
        if snap.decisions.iter().any(|d| d.submission_id == sub.id) {
            happened.push("decision already recorded for submission".into());
            if let Some(decision) = snap
                .decisions
                .iter()
                .rev()
                .find(|d| d.submission_id == sub.id)
            {
                if case.stage == CaseStage::FollowUp || case.stage == CaseStage::Decision {
                    self.drive_decision_outcome(case, decision, happened)?;
                }
            }
            return Ok(());
        }
        let outcome = match lookup.outcome.as_str() {
            "approved" => DecisionOutcome::Approved,
            "denied" => DecisionOutcome::Denied,
            "pending" => DecisionOutcome::Pending,
            "more_info" => DecisionOutcome::MoreInfo,
            _ => DecisionOutcome::Unclear,
        };
        let decision = Decision {
            id: Uuid::new_v4(),
            case_id: case.id,
            submission_id: sub.id,
            outcome,
            limitations: lookup.limitations.clone(),
            reason: lookup.reason.clone(),
            created_at: self.now(),
        };
        self.store.insert_decision(&decision)?;
        self.store
            .clear_pending_kind(case.id, PendingKind::FollowUp)?;
        self.set_stage(case, CaseStage::Decision, "decision recorded")?;
        happened.push(format!("decision {:?}", outcome));
        self.drive_decision_outcome(case, &decision, happened)?;
        Ok(())
    }

    fn drive_decision(
        &self,
        case: &mut Case,
        _fixture: &ScenarioFixture,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        let snap = self.store.load_snapshot(case.id)?;
        if let Some(decision) = snap.decisions.last() {
            self.drive_decision_outcome(case, decision, happened)?;
        } else {
            happened.push("no decision yet".into());
        }
        Ok(())
    }

    fn drive_decision_outcome(
        &self,
        case: &mut Case,
        decision: &Decision,
        happened: &mut Vec<String>,
    ) -> LabResult<()> {
        match decision.outcome {
            DecisionOutcome::Approved => {
                case.disposition = Some("approved_handoff".into());
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::DeliverHandoff,
                    status: TaskStatus::Open,
                    owner: Role::Customer,
                    context_json: json!({
                        "limitations": decision.limitations,
                        "decision_id": decision.id
                    })
                    .to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                self.set_stage(case, CaseStage::Handoff, "approved")?;
                case.updated_at = self.now();
                self.store.update_case(case)?;
                happened.push("approved → handoff".into());
            }
            DecisionOutcome::Denied => {
                let snap = self.store.load_snapshot(case.id)?;
                if !snap
                    .pending
                    .iter()
                    .any(|p| p.kind == PendingKind::AppealDecision)
                {
                    let pending = PendingWork {
                        id: Uuid::new_v4(),
                        case_id: case.id,
                        kind: PendingKind::AppealDecision,
                        ref_id: decision.id.to_string(),
                        detail: "awaiting human appeal decision".into(),
                        due_at: None,
                        created_at: self.now(),
                    };
                    self.store.insert_pending(&pending)?;
                }
                happened.push("denied — waiting for human appeal decision".into());
            }
            DecisionOutcome::Unclear | DecisionOutcome::MoreInfo => {
                let task = Task {
                    id: Uuid::new_v4(),
                    case_id: case.id,
                    purpose: TaskPurpose::HumanReview,
                    status: TaskStatus::Open,
                    owner: Role::Reviewer,
                    context_json: json!({"decision": decision.outcome}).to_string(),
                    created_at: self.now(),
                    completed_at: None,
                };
                self.store.insert_task(&task)?;
                self.set_stage(case, CaseStage::Review, "unclear decision")?;
                happened.push("unclear/more-info decision → review".into());
            }
            DecisionOutcome::Pending => {
                happened.push("decision still pending".into());
            }
        }
        Ok(())
    }

    pub fn initiate_appeal(&self, run_id: Uuid) -> LabResult<TickReport> {
        let mut case = self.require_case(run_id)?;
        let snap = self.store.load_snapshot(case.id)?;
        let denial = snap
            .decisions
            .iter()
            .rev()
            .find(|d| d.outcome == DecisionOutcome::Denied)
            .cloned()
            .ok_or_else(|| LabError::Invalid("no denial to appeal".into()))?;

        let prior_appeals = snap
            .packets
            .iter()
            .filter(|p| p.appeal_of_decision_id.is_some())
            .count();
        if prior_appeals >= 1 {
            case.disposition = Some("manual_after_appeal".into());
            self.set_stage(&mut case, CaseStage::ManualDisposition, "appeal exhausted")?;
            case.updated_at = self.now();
            self.store.update_case(&case)?;
            return self.process_pending(run_id);
        }

        self.store
            .clear_pending_kind(case.id, PendingKind::AppealDecision)?;
        self.set_stage(&mut case, CaseStage::Appeal, "appeal initiated")?;
        let task = Task {
            id: Uuid::new_v4(),
            case_id: case.id,
            purpose: TaskPurpose::AppealPreparation,
            status: TaskStatus::Open,
            owner: Role::Operator,
            context_json: json!({"denial_id": denial.id}).to_string(),
            created_at: self.now(),
            completed_at: None,
        };
        self.store.insert_task(&task)?;
        let mut happened = vec!["appeal initiated".into()];
        let packet = self.build_packet(&mut case, Some(denial.id), &mut happened)?;
        let _ = packet;
        self.process_pending(run_id)
    }

    pub fn cancel_case(&self, run_id: Uuid) -> LabResult<TickReport> {
        let mut case = self.require_case(run_id)?;
        self.set_stage(&mut case, CaseStage::Cancelled, "cancelled")?;
        case.disposition = Some("cancelled".into());
        case.updated_at = self.now();
        self.store.update_case(&case)?;
        Ok(TickReport {
            stage: CaseStage::Cancelled,
            happened: vec!["case cancelled".into()],
            next_owner: None,
            next_action: None,
            evidence: vec![],
            blockers: vec![],
        })
    }

    pub fn pause(&self, run_id: Uuid) -> LabResult<()> {
        let mut case = self.require_case(run_id)?;
        case.paused_from = Some(case.stage);
        self.set_stage(&mut case, CaseStage::Paused, "paused")?;
        case.updated_at = self.now();
        self.store.update_case(&case)
    }

    pub fn resume(&self, run_id: Uuid) -> LabResult<TickReport> {
        let mut case = self.require_case(run_id)?;
        let from = case.paused_from.unwrap_or(CaseStage::Intake);
        case.paused_from = None;
        self.set_stage(&mut case, from, "resumed")?;
        case.updated_at = self.now();
        self.store.update_case(&case)?;
        self.process_pending(run_id)
    }

    pub fn apply_coverage_change(
        &self,
        run_id: Uuid,
        new_coverage: CoverageContext,
    ) -> LabResult<TickReport> {
        let mut case = self.require_case(run_id)?;
        let snap = self.store.load_snapshot(case.id)?;
        let has_unknown = snap
            .submissions
            .iter()
            .any(|s| s.transport_state == SubmissionTransportState::Unknown);
        case.coverage = new_coverage;
        case.coverage_version += 1;
        case.updated_at = self.now();
        self.store.update_case(&case)?;
        self.store.mark_observations_stale(case.id)?;
        self.store.append_event(
            case.id,
            "coverage_changed",
            &json!({
                "coverage_version": case.coverage_version,
                "preserved_unknown_submissions": has_unknown
            }),
            self.now(),
        )?;

        if matches!(
            case.stage,
            CaseStage::Documentation
                | CaseStage::Review
                | CaseStage::Bv
                | CaseStage::Intake
                | CaseStage::Handoff
        ) && !has_unknown
        {
            for task in snap.tasks.iter().filter(|t| t.status == TaskStatus::Open) {
                if matches!(
                    task.purpose,
                    TaskPurpose::SubmitPa
                        | TaskPurpose::CollectDocumentation
                        | TaskPurpose::ReviewPacket
                ) {
                    let mut t = task.clone();
                    t.status = TaskStatus::Cancelled;
                    t.completed_at = Some(self.now());
                    self.store.update_task(&t)?;
                }
            }
            self.store
                .clear_pending_kind(case.id, PendingKind::Review)?;
            self.store
                .clear_pending_kind(case.id, PendingKind::DocumentRequest)?;
            self.set_stage(&mut case, CaseStage::Bv, "reassess after coverage change")?;
            case.updated_at = self.now();
            self.store.update_case(&case)?;
            let task = Task {
                id: Uuid::new_v4(),
                case_id: case.id,
                purpose: TaskPurpose::BenefitsVerification,
                status: TaskStatus::Open,
                owner: Role::Operator,
                context_json: json!({
                    "service": case.service,
                    "coverage": case.coverage,
                    "coverage_version": case.coverage_version,
                    "reassess": true
                })
                .to_string(),
                created_at: self.now(),
                completed_at: None,
            };
            self.store.insert_task(&task)?;
        }

        Ok(TickReport {
            stage: case.stage,
            happened: vec!["coverage changed; BV observations marked stale".into()],
            next_owner: Some(Role::Operator),
            next_action: Some("reassess benefits verification".into()),
            evidence: vec![],
            blockers: vec![],
        })
    }

    pub fn check_run(&self, run_id: Uuid) -> LabResult<Vec<String>> {
        let snap = self.snapshot(run_id)?;
        let mut failures = Vec::new();

        let mut last_seq = 0u64;
        for event in &snap.events {
            if event.seq <= last_seq {
                failures.push(format!("event seq not monotonic at {}", event.seq));
            }
            last_seq = event.seq;
        }

        for review in snap
            .reviews
            .iter()
            .filter(|r| r.valid && r.decision == ReviewDecision::Approve)
        {
            if let Some(packet) = snap.packets.iter().find(|p| p.id == review.packet_id) {
                if packet.content_hash != review.packet_hash {
                    failures.push(format!(
                        "valid approval {} does not match packet hash",
                        review.id
                    ));
                }
            }
        }

        for sub in &snap.submissions {
            if sub.transport_state == SubmissionTransportState::ReceiptConfirmed
                && sub.payer_receipt_id.is_none()
            {
                failures.push(format!(
                    "receipt-confirmed submission {} missing receipt id",
                    sub.id
                ));
            }
        }

        for det in &snap.determinations {
            for obs_id in &det.observation_ids {
                if !snap.observations.iter().any(|o| o.id == *obs_id) {
                    failures.push(format!(
                        "determination {} references missing observation {obs_id}",
                        det.id
                    ));
                }
            }
        }

        if snap.case.stage == CaseStage::Cancelled
            && snap.case.disposition.as_deref() != Some("cancelled")
        {
            failures.push("cancelled case missing disposition".into());
        }

        Ok(failures)
    }

    pub fn run_scripted(&self, scenario_id: &str) -> LabResult<(Uuid, TickReport)> {
        let run_id = self.start_run(scenario_id)?;
        let fixture = self.scenario_for(run_id, scenario_id)?;
        let mut report = self.process_pending(run_id)?;

        for _ in 0..32 {
            let case = self.require_case(run_id)?;
            match case.stage {
                CaseStage::Bv => {
                    let snap = self.snapshot(run_id)?;
                    if snap
                        .pending
                        .iter()
                        .any(|p| p.kind == PendingKind::AgentQuestion)
                    {
                        if let Some(answer) = fixture.scripted_payer_answers.first() {
                            report = self.submit_payer_answer(run_id, answer)?;
                            continue;
                        }
                    }
                    report = self.process_pending(run_id)?;
                }
                CaseStage::Documentation => {
                    let snap = self.snapshot(run_id)?;
                    let required = snap
                        .determinations
                        .iter()
                        .rev()
                        .find(|d| d.kind == DeterminationKind::PaRequired)
                        .map(|d| d.required_docs.clone())
                        .unwrap_or_default();
                    for doc in required {
                        if !snap.documents.iter().any(|d| d.request_id == doc) {
                            let _ = self.supply_document(run_id, &doc, &doc)?;
                        }
                    }
                    report = self.process_pending(run_id)?;
                }
                CaseStage::Review => {
                    let snap = self.snapshot(run_id)?;
                    if snap.tasks.iter().any(|t| {
                        t.purpose == TaskPurpose::HumanReview
                            && t.status == TaskStatus::Open
                            && t.context_json.contains("conflict")
                    }) {
                        break;
                    }
                    if let Some(packet) = snap.packets.last() {
                        if fixture.auto_review_approve
                            || fixture.expected_disposition.contains("approved")
                            || fixture.expected_disposition.contains("denied")
                            || fixture.appeal_after_denial
                        {
                            report = self.review_packet(
                                run_id,
                                packet.id,
                                ReviewDecision::Approve,
                                "scripted-reviewer",
                            )?;
                            continue;
                        }
                    }
                    break;
                }
                CaseStage::Submission | CaseStage::FollowUp | CaseStage::Decision => {
                    report = self.process_pending(run_id)?;
                }
                CaseStage::Appeal => {
                    let snap = self.snapshot(run_id)?;
                    if let Some(packet) = snap.packets.last() {
                        let needs_review = !snap.reviews.iter().any(|r| {
                            r.valid
                                && r.packet_id == packet.id
                                && r.decision == ReviewDecision::Approve
                        });
                        if needs_review {
                            report = self.review_packet(
                                run_id,
                                packet.id,
                                ReviewDecision::Approve,
                                "scripted-reviewer",
                            )?;
                            continue;
                        }
                    }
                    report = self.process_pending(run_id)?;
                }
                CaseStage::Handoff | CaseStage::ManualDisposition | CaseStage::Cancelled => break,
                CaseStage::Intake | CaseStage::Paused => {
                    report = self.process_pending(run_id)?;
                }
            }

            let case = self.require_case(run_id)?;
            if case.stage == CaseStage::Decision || case.stage == CaseStage::FollowUp {
                let snap = self.snapshot(run_id)?;
                if snap
                    .decisions
                    .iter()
                    .any(|d| d.outcome == DecisionOutcome::Denied)
                    && fixture.appeal_after_denial
                    && snap
                        .pending
                        .iter()
                        .any(|p| p.kind == PendingKind::AppealDecision)
                {
                    report = self.initiate_appeal(run_id)?;
                    continue;
                }
            }

            if matches!(
                case.stage,
                CaseStage::Handoff | CaseStage::ManualDisposition | CaseStage::Cancelled
            ) {
                break;
            }
        }
        Ok((run_id, report))
    }
}

trait KindLabel {
    fn kind_label(&self) -> String;
}

impl KindLabel for Observation {
    fn kind_label(&self) -> String {
        format!("{:?}", self.kind)
    }
}

fn synthesize_determination(
    case: &Case,
    snap: &CaseSnapshot,
    fixture: &ScenarioFixture,
) -> LabResult<Determination> {
    let fresh: Vec<_> = snap.observations.iter().filter(|o| !o.stale).collect();
    let obs_ids: Vec<Uuid> = fresh.iter().map(|o| o.id).collect();

    if fresh
        .iter()
        .any(|o| o.kind == ObservationKind::InjectionAttempt)
    {
        return Ok(Determination {
            id: Uuid::new_v4(),
            case_id: case.id,
            kind: DeterminationKind::Unclear,
            rationale: "injection attempt requires human review".into(),
            required_docs: vec![],
            observation_ids: obs_ids,
            coverage_version: case.coverage_version,
            service_version: case.service_version,
            created_at: case.updated_at,
        });
    }

    let kind = if let Some(hidden) = &fixture.hidden_facts.bv_outcome {
        match hidden.as_str() {
            "pa_required" => DeterminationKind::PaRequired,
            "pa_not_required" => DeterminationKind::PaNotRequired,
            "inactive_member" => DeterminationKind::InactiveMember,
            "not_covered" => DeterminationKind::NotCovered,
            "conflict" => DeterminationKind::Conflict,
            _ => DeterminationKind::Unclear,
        }
    } else if fresh
        .iter()
        .any(|o| o.statement.to_lowercase().contains("conflict"))
    {
        DeterminationKind::Conflict
    } else if fresh.iter().any(|o| {
        o.kind == ObservationKind::Eligibility && o.statement.to_lowercase().contains("inactive")
    }) {
        DeterminationKind::InactiveMember
    } else if fresh.iter().any(|o| {
        o.kind == ObservationKind::Coverage && o.statement.to_lowercase().contains("not covered")
    }) {
        DeterminationKind::NotCovered
    } else if fresh.iter().any(|o| {
        o.kind == ObservationKind::PaRequirement
            && o.statement.to_lowercase().contains("not required")
            && o.uncertainty == Uncertainty::Known
    }) {
        DeterminationKind::PaNotRequired
    } else if fresh.iter().any(|o| {
        o.kind == ObservationKind::PaRequirement
            && o.statement.to_lowercase().contains("is required")
            && o.uncertainty == Uncertainty::Known
    }) {
        DeterminationKind::PaRequired
    } else {
        DeterminationKind::Unclear
    };

    let required_docs = if kind == DeterminationKind::PaRequired {
        if fixture.required_docs.is_empty() {
            vec!["clinical_notes".into(), "order".into()]
        } else {
            fixture.required_docs.clone()
        }
    } else {
        vec![]
    };

    Ok(Determination {
        id: Uuid::new_v4(),
        case_id: case.id,
        kind,
        rationale: format!("synthesized from {} fresh observations", fresh.len()),
        required_docs,
        observation_ids: obs_ids,
        coverage_version: case.coverage_version,
        service_version: case.service_version,
        created_at: case.updated_at,
    })
}

fn infer_next(snap: &CaseSnapshot) -> (Option<Role>, Option<String>, Vec<String>) {
    let mut blockers = Vec::new();
    for p in &snap.pending {
        blockers.push(format!("{:?}:{}", p.kind, p.detail));
    }
    match snap.case.stage {
        CaseStage::Bv
            if snap
                .pending
                .iter()
                .any(|p| p.kind == PendingKind::AgentQuestion) =>
        {
            (
                Some(Role::Payer),
                Some("answer BV question".into()),
                blockers,
            )
        }
        CaseStage::Documentation => (
            Some(Role::Customer),
            Some("supply required documents".into()),
            blockers,
        ),
        CaseStage::Review => (Some(Role::Reviewer), Some("review packet".into()), blockers),
        CaseStage::Decision
            if snap
                .pending
                .iter()
                .any(|p| p.kind == PendingKind::AppealDecision) =>
        {
            (
                Some(Role::Operator),
                Some("initiate appeal or accept denial".into()),
                blockers,
            )
        }
        CaseStage::Handoff => (
            Some(Role::Customer),
            Some("acknowledge handoff".into()),
            blockers,
        ),
        CaseStage::ManualDisposition => (
            Some(Role::Operator),
            Some("manual disposition".into()),
            blockers,
        ),
        _ => (
            Some(Role::Operator),
            Some("tick / process_pending".into()),
            blockers,
        ),
    }
}
