//! Scenario fixtures for medical-mri-pa-v1 lab runs.
//! Used by: workflow start_run and guided noninteractive run.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::lab::domain::{CoverageContext, ServiceContext, WORKFLOW_VERSION};
use crate::lab::error::{LabError, LabResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioFixture {
    pub id: String,
    #[serde(default = "default_workflow")]
    pub workflow_version: String,
    pub service: ServiceContext,
    pub coverage: CoverageContext,
    #[serde(default = "default_true")]
    pub intake_complete: bool,
    #[serde(default)]
    pub missing_intake_fields: Vec<String>,
    #[serde(default)]
    pub required_docs: Vec<String>,
    #[serde(default)]
    pub scripted_payer_answers: Vec<String>,
    #[serde(default)]
    pub hidden_facts: HiddenFacts,
    pub expected_disposition: String,
    #[serde(default)]
    pub document_fixtures: HashMap<String, String>,
    #[serde(default)]
    pub auto_supply_docs: bool,
    #[serde(default)]
    pub auto_review_approve: bool,
    #[serde(default)]
    pub appeal_after_denial: bool,
    #[serde(default)]
    pub coverage_change: Option<CoverageChange>,
    #[serde(default)]
    pub external_claim: Option<String>,
    #[serde(default)]
    pub injection_in_source: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HiddenFacts {
    pub bv_outcome: Option<String>,
    pub decision_outcome: Option<String>,
    pub limitations: Vec<String>,
    pub denial_reason: Option<String>,
    pub appeal_decision_outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageChange {
    pub after_stage: String,
    pub new_coverage: CoverageContext,
}

fn default_workflow() -> String {
    WORKFLOW_VERSION.to_string()
}

fn default_true() -> bool {
    true
}

pub fn fixtures_dir() -> PathBuf {
    if let Ok(path) = std::env::var("MINT_LAB_FIXTURES") {
        return PathBuf::from(path);
    }
    let candidates = [
        PathBuf::from("fixtures/pa_bv"),
        PathBuf::from("/workspace/.wt-pa-bv/fixtures/pa_bv"),
    ];
    for c in candidates {
        if c.exists() {
            return c;
        }
    }
    PathBuf::from("fixtures/pa_bv")
}

pub fn load_scenario(scenario_id: &str) -> LabResult<ScenarioFixture> {
    load_scenario_from(&fixtures_dir(), scenario_id)
}

pub fn load_scenario_from(dir: &Path, scenario_id: &str) -> LabResult<ScenarioFixture> {
    let path = dir.join(format!("{scenario_id}.json"));
    if !path.exists() {
        return Err(LabError::NotFound(format!(
            "scenario fixture {}",
            path.display()
        )));
    }
    let raw = fs::read_to_string(&path)?;
    let fixture: ScenarioFixture = serde_json::from_str(&raw)?;
    if fixture.id != scenario_id {
        return Err(LabError::Invalid(format!(
            "fixture id {} does not match requested {}",
            fixture.id, scenario_id
        )));
    }
    Ok(fixture)
}

pub fn list_scenarios() -> LabResult<Vec<String>> {
    list_scenarios_from(&fixtures_dir())
}

pub fn list_scenarios_from(dir: &Path) -> LabResult<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                ids.push(stem.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workflow_constant() {
        assert_eq!(default_workflow(), WORKFLOW_VERSION);
    }
}
