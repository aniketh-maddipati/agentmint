//! Session log that signs a chained AERF receipt for every gate decision.
//! Used by: the Tier 1 gate suite.

use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha512};

use crate::aerf::canonical::canonicalize;
use crate::aerf::gate::{evaluate, identical_call_count, HistoryEntry, Spec, ToolCall, Verdict};
use crate::aerf::receipt::Receipt;
use crate::aerf::sign::{key_id, sign_stripped};
use crate::aerf::AerfResult;

pub struct StepOutcome {
    pub verdict: Verdict,
    pub receipt: Receipt,
    pub receipt_json: Value,
}

pub struct Session {
    signing_key: SigningKey,
    spec: Spec,
    plan_id: String,
    agent: String,
    session_id: String,
    history: Vec<HistoryEntry>,
    receipts: Vec<Receipt>,
    previous_hash: Option<String>,
    seq: u64,
}

impl Session {
    pub fn new(signing_key: SigningKey, spec: Spec, plan_id: &str) -> Self {
        Self {
            signing_key,
            spec,
            plan_id: plan_id.to_owned(),
            agent: "tier1-agent".to_owned(),
            session_id: format!("session-{plan_id}"),
            history: Vec::new(),
            receipts: Vec::new(),
            previous_hash: None,
            seq: 0,
        }
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }

    pub fn receipts(&self) -> &[Receipt] {
        &self.receipts
    }

    pub fn evaluate(&self, call: &ToolCall) -> Verdict {
        evaluate(&self.spec, &self.history, call)
    }

    pub fn commit(
        &mut self,
        call: ToolCall,
        verdict: Verdict,
        result: Option<Value>,
        scope: ScopeTag,
    ) -> AerfResult<StepOutcome> {
        let repetition = identical_call_count(&self.history, &call) + 1;
        let passed = verdict.is_pass();
        let evidence = build_evidence(&call, &verdict, result, repetition, &scope);
        let receipt_json = self.build_receipt(&call, &verdict, evidence, &scope)?;
        let receipt = Receipt::from_value(receipt_json.clone())?;
        self.previous_hash = Some(receipt.chain_hash()?);
        self.history.push(HistoryEntry { call, passed });
        self.receipts.push(receipt.clone());
        Ok(StepOutcome {
            verdict,
            receipt,
            receipt_json,
        })
    }

    fn build_receipt(
        &mut self,
        call: &ToolCall,
        verdict: &Verdict,
        evidence: Value,
        scope: &ScopeTag,
    ) -> AerfResult<Value> {
        self.seq += 1;
        let evidence_hash = sha512_hex(canonicalize(&evidence)?.as_bytes());
        let mut object = Map::new();
        object.insert("id".into(), json!(format!("{}-{}", self.session_id, self.seq)));
        object.insert("type".into(), json!("notarised_evidence"));
        object.insert("plan_id".into(), json!(self.plan_id));
        object.insert("agent".into(), json!(self.agent));
        object.insert("action".into(), json!(call.tool));
        object.insert("in_policy".into(), json!(verdict.is_pass()));
        object.insert("policy_reason".into(), json!(policy_reason(verdict)));
        object.insert("evidence_hash_sha512".into(), json!(evidence_hash));
        object.insert("evidence".into(), evidence);
        object.insert("observed_at".into(), json!(Utc::now().to_rfc3339()));
        object.insert("key_id".into(), json!(key_id(&self.verifying_key())));
        object.insert("session_id".into(), json!(self.session_id));
        object.insert("seq".into(), json!(self.seq));
        if let Some(previous) = &self.previous_hash {
            object.insert("previous_receipt_hash".into(), json!(previous));
        }
        if scope.in_task_scope == Some(false) {
            object.insert("compliance_tags".into(), json!(["scope:out_of_task"]));
        }

        let mut receipt = Value::Object(object);
        let signature = sign_stripped(&receipt, &self.signing_key)?;
        receipt["signature"] = json!(signature);
        Ok(receipt)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScopeTag {
    pub in_task_scope: Option<bool>,
    pub scope_note: Option<String>,
}

impl ScopeTag {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn out_of_scope(note: &str) -> Self {
        Self {
            in_task_scope: Some(false),
            scope_note: Some(note.to_owned()),
        }
    }

    pub fn in_scope() -> Self {
        Self {
            in_task_scope: Some(true),
            scope_note: None,
        }
    }
}

fn build_evidence(
    call: &ToolCall,
    verdict: &Verdict,
    result: Option<Value>,
    repetition: usize,
    scope: &ScopeTag,
) -> Value {
    let mut evidence = Map::new();
    evidence.insert("tool".into(), json!(call.tool));
    evidence.insert("args".into(), call.args.clone());
    evidence.insert(
        "decision".into(),
        json!(if verdict.is_pass() { "pass" } else { "block" }),
    );
    evidence.insert("result".into(), result.unwrap_or(Value::Null));
    evidence.insert("identical_call_count".into(), json!(repetition));
    if let Verdict::Block { rule, .. } = verdict {
        evidence.insert("blocked_by".into(), json!(rule.as_str()));
    }
    if let Some(in_scope) = scope.in_task_scope {
        evidence.insert("in_task_scope".into(), json!(in_scope));
    }
    if let Some(note) = &scope.scope_note {
        evidence.insert("scope_note".into(), json!(note));
    }
    Value::Object(evidence)
}

fn policy_reason(verdict: &Verdict) -> String {
    match verdict {
        Verdict::Pass => "within policy".to_owned(),
        Verdict::Block { reason, .. } => reason.clone(),
    }
}

fn sha512_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use serde_json::json;
    use std::collections::HashMap;

    use super::*;
    use crate::aerf::gate::{BlockRule, ToolRules};
    use crate::aerf::receipt::{verify_aerf_receipt, VerifyOptions};

    fn session() -> Session {
        let mut command = ToolRules::default();
        command
            .blocked_patterns
            .insert("command".into(), vec!["rm -rf".into()]);
        let mut tools = HashMap::new();
        tools.insert("run_command".into(), command);
        let spec = Spec {
            tools,
            max_identical_calls: 3,
        };
        Session::new(SigningKey::from_bytes(&[9; 32]), spec, "unit")
    }

    #[test]
    fn every_decision_emits_a_verifiable_receipt() {
        let mut session = session();
        let public_key = session.verifying_key();

        let blocked = ToolCall::new("run_command", json!({ "command": "rm -rf /" }));
        let outcome = session
            .commit(blocked, Verdict::Block {
                rule: BlockRule::BlockedPattern,
                reason: "match".into(),
            }, None, ScopeTag::none())
            .expect("commit");

        assert!(!outcome.receipt.signature.is_empty());
        assert!(verify_aerf_receipt(&outcome.receipt, &public_key, VerifyOptions::default()).ok);
        assert_eq!(outcome.receipt.in_policy, false);
    }

    #[test]
    fn chained_receipts_link_and_increment_seq() {
        let mut session = session();
        let first = ToolCall::new("run_command", json!({ "command": "ls" }));
        let second = ToolCall::new("run_command", json!({ "command": "pwd" }));

        session
            .commit(first, Verdict::Pass, Some(json!({ "exit_code": 0 })), ScopeTag::none())
            .expect("first");
        session
            .commit(second, Verdict::Pass, Some(json!({ "exit_code": 0 })), ScopeTag::none())
            .expect("second");

        let receipts = session.receipts();
        assert_eq!(receipts[0].seq, Some(1));
        assert_eq!(receipts[1].seq, Some(2));
        assert!(receipts[0].previous_receipt_hash.is_none());
        assert_eq!(
            receipts[1].previous_receipt_hash.as_deref(),
            Some(receipts[0].chain_hash().expect("hash").as_str())
        );
    }
}
