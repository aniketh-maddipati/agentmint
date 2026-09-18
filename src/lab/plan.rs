//! Structured BV reasoning artifacts (`bv-plan-v1`) and bounded replan metadata.
//! Default: embed in `tool_calls_json`. Opt-in columns: `MINT_LAB_REASONING_COLUMNS=1`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lab::domain::AgentRunRecord;
use crate::lab::tools::ToolTrace;

pub const PLAN_VERSION: &str = "bv-plan-v1";
pub const MAX_REPAIR_LOOPS: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BvPlanStep {
    pub id: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BvPlan {
    pub plan_version: String,
    pub goal: String,
    pub steps: Vec<BvPlanStep>,
    pub stop_conditions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct RepairMeta {
    pub count: u32,
    pub max: u32,
    pub triggers: Vec<String>,
}

impl RepairMeta {
    pub fn none() -> Self {
        Self {
            count: 0,
            max: MAX_REPAIR_LOOPS,
            triggers: Vec::new(),
        }
    }

    pub fn record(&mut self, trigger: &str) -> bool {
        if self.count >= self.max {
            return false;
        }
        self.count += 1;
        self.triggers.push(trigger.to_owned());
        true
    }
}

pub fn default_bv_plan() -> BvPlan {
    BvPlan {
        plan_version: PLAN_VERSION.into(),
        goal: "determine_pa_requirement".into(),
        steps: vec![
            BvPlanStep {
                id: "s1".into(),
                action: "read_assigned_context".into(),
            },
            BvPlanStep {
                id: "s2".into(),
                action: "ask_payer_or_read_payer_msgs".into(),
            },
            BvPlanStep {
                id: "s3".into(),
                action: "extract_observations".into(),
            },
            BvPlanStep {
                id: "s4".into(),
                action: "verify_evidence_and_uncertainty".into(),
            },
        ],
        stop_conditions: vec![
            "pending_payer".into(),
            "clarification".into(),
            "observations_accepted".into(),
        ],
    }
}

pub fn reasoning_columns_enabled() -> bool {
    std::env::var("MINT_LAB_REASONING_COLUMNS").ok().as_deref() == Some("1")
}

pub fn persist_reasoning_columns(record: &mut AgentRunRecord, trace: &ToolTrace) {
    if !reasoning_columns_enabled() {
        record.plan_json = None;
        record.reasoning_json = None;
        return;
    }
    record.plan_json = trace
        .plan
        .as_ref()
        .and_then(|plan| serde_json::to_string(plan).ok());
    record.reasoning_json = trace
        .repair
        .as_ref()
        .and_then(|repair| serde_json::to_string(repair).ok());
}

pub fn plan_from_record(record: &AgentRunRecord) -> Option<BvPlan> {
    if let Some(raw) = &record.plan_json {
        if let Ok(plan) = serde_json::from_str(raw) {
            return Some(plan);
        }
    }
    let trace = crate::lab::tools::parse_tool_trace(&record.tool_calls_json).ok()?;
    trace.plan
}

pub fn repair_from_record(record: &AgentRunRecord) -> Option<RepairMeta> {
    if let Some(raw) = &record.reasoning_json {
        if let Ok(repair) = serde_json::from_str(raw) {
            return Some(repair);
        }
    }
    crate::lab::tools::parse_tool_trace(&record.tool_calls_json)
        .ok()
        .and_then(|trace| trace.repair)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::tools::{parse_tool_trace, ToolTrace};
    use chrono::Utc;
    use uuid::Uuid;

    #[test]
    fn default_plan_is_bv_plan_v1() {
        let plan = default_bv_plan();
        assert_eq!(plan.plan_version, PLAN_VERSION);
        assert_eq!(plan.steps.len(), 4);
        assert!(plan.steps.iter().all(|s| !s.action.contains("think")));
    }

    #[test]
    fn repair_is_capped_at_two() {
        let mut repair = RepairMeta::none();
        assert!(repair.record("schema"));
        assert!(repair.record("unknown_evidence"));
        assert!(!repair.record("think_harder"));
        assert_eq!(repair.count, 2);
        assert_eq!(repair.max, MAX_REPAIR_LOOPS);
    }

    #[test]
    fn embed_and_column_paths_round_trip_same_plan() {
        let mut trace = ToolTrace::new();
        let plan = default_bv_plan();
        let repair = RepairMeta::none();
        trace.plan = Some(plan.clone());
        trace.repair = Some(repair.clone());
        let parsed = parse_tool_trace(&trace.to_json_string()).expect("trace");
        assert_eq!(parsed.plan.as_ref(), Some(&plan));
        assert_eq!(parsed.repair.as_ref(), Some(&repair));

        let previous = std::env::var("MINT_LAB_REASONING_COLUMNS").ok();
        std::env::set_var("MINT_LAB_REASONING_COLUMNS", "1");
        let mut record = AgentRunRecord {
            id: Uuid::new_v4(),
            case_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            prompt_version: "bv-scripted-v1".into(),
            model_id: "scripted".into(),
            context_version: 1,
            tool_calls_json: trace.to_json_string(),
            structured_output_json: "{}".into(),
            evidence_refs: vec![],
            created_at: Utc::now(),
            plan_json: None,
            reasoning_json: None,
        };
        persist_reasoning_columns(&mut record, &trace);
        assert!(record.plan_json.is_some());
        assert_eq!(plan_from_record(&record).as_ref(), Some(&plan));
        assert_eq!(repair_from_record(&record).as_ref(), Some(&repair));
        match previous {
            Some(v) => std::env::set_var("MINT_LAB_REASONING_COLUMNS", v),
            None => std::env::remove_var("MINT_LAB_REASONING_COLUMNS"),
        }
    }
}
