//! Structural turn policies over beliefs and tool results.
//! Used by: the turn loop.

use std::collections::{HashMap, VecDeque};

use crate::aerf::gate::GateDecision;
use crate::turnrt::belief::BeliefRecord;

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResultRecord {
    pub gear: usize,
    pub command: String,
    pub exit_code: Option<i32>,
    pub deleted_lines: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyHit {
    pub policy: &'static str,
    pub reason: String,
    pub decision: GateDecision,
}

pub trait TurnPolicy {
    fn name(&self) -> &'static str;
    fn on_belief(&mut self, belief: &BeliefRecord) -> Option<PolicyHit>;
    fn on_tool_result(&mut self, result: &ToolResultRecord) -> Option<PolicyHit>;
}

pub struct NonZeroExit {
    consecutive_failures: usize,
}

impl Default for NonZeroExit {
    fn default() -> Self {
        Self::new()
    }
}

impl NonZeroExit {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
        }
    }
}

impl TurnPolicy for NonZeroExit {
    fn name(&self) -> &'static str {
        "non_zero_exit"
    }

    fn on_belief(&mut self, _: &BeliefRecord) -> Option<PolicyHit> {
        None
    }

    fn on_tool_result(&mut self, result: &ToolResultRecord) -> Option<PolicyHit> {
        if matches!(result.exit_code, Some(0)) {
            self.consecutive_failures = 0;
            return None;
        }

        if result.exit_code.is_some() {
            self.consecutive_failures += 1;
        }

        if self.consecutive_failures >= 2 {
            return Some(PolicyHit {
                policy: self.name(),
                reason: "two consecutive non-zero exits".to_owned(),
                decision: GateDecision::Block,
            });
        }

        None
    }
}

pub struct BigDeletion {
    pub max_deleted_lines: usize,
}

impl Default for BigDeletion {
    fn default() -> Self {
        Self::new()
    }
}

impl BigDeletion {
    pub fn new() -> Self {
        Self {
            max_deleted_lines: 30,
        }
    }

    pub fn inspect_command(&self, command: &str) -> Option<PolicyHit> {
        let deleted_lines = count_deleted_lines(command);
        if deleted_lines > self.max_deleted_lines {
            return Some(PolicyHit {
                policy: "big_deletion",
                reason: format!("proposed deletion of {} lines", deleted_lines),
                decision: GateDecision::Block,
            });
        }
        None
    }
}

impl TurnPolicy for BigDeletion {
    fn name(&self) -> &'static str {
        "big_deletion"
    }

    fn on_belief(&mut self, _: &BeliefRecord) -> Option<PolicyHit> {
        None
    }

    fn on_tool_result(&mut self, result: &ToolResultRecord) -> Option<PolicyHit> {
        if result.deleted_lines > self.max_deleted_lines {
            return Some(PolicyHit {
                policy: self.name(),
                reason: format!("proposed deletion of {} lines", result.deleted_lines),
                decision: GateDecision::Block,
            });
        }
        None
    }
}

pub struct ThrashDetector {
    pub window: usize,
    pub similarity_threshold: f32,
    pub min_failures: usize,
    beliefs: VecDeque<BeliefRecord>,
    exits: HashMap<usize, Option<i32>>,
}

impl Default for ThrashDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl ThrashDetector {
    pub fn new() -> Self {
        Self {
            window: 3,
            similarity_threshold: 0.5,
            min_failures: 2,
            beliefs: VecDeque::new(),
            exits: HashMap::new(),
        }
    }
}

impl TurnPolicy for ThrashDetector {
    fn name(&self) -> &'static str {
        "thrash_detector"
    }

    fn on_belief(&mut self, belief: &BeliefRecord) -> Option<PolicyHit> {
        self.beliefs.push_back(belief.clone());
        while self.beliefs.len() > self.window {
            self.beliefs.pop_front();
        }

        if self.beliefs.len() < self.window {
            return None;
        }

        let claims = self
            .beliefs
            .iter()
            .map(|belief| belief.claim.as_str())
            .collect::<Vec<_>>();
        if !all_similar(&claims, self.similarity_threshold) {
            return None;
        }

        let failures = self
            .exits
            .values()
            .filter(|exit_code| matches!(exit_code, Some(code) if *code != 0))
            .count();
        if failures < self.min_failures {
            return None;
        }

        Some(PolicyHit {
            policy: self.name(),
            reason: format!(
                "same claim {} gears running, {} failures",
                self.window, failures
            ),
            decision: GateDecision::Block,
        })
    }

    fn on_tool_result(&mut self, result: &ToolResultRecord) -> Option<PolicyHit> {
        self.exits.insert(result.gear, result.exit_code);
        None
    }
}

pub struct ResolutionMismatch {
    pub said_threshold: f32,
    pub logit_threshold: f32,
}

impl Default for ResolutionMismatch {
    fn default() -> Self {
        Self::new()
    }
}

impl ResolutionMismatch {
    pub fn new() -> Self {
        Self {
            said_threshold: 0.7,
            logit_threshold: 0.5,
        }
    }
}

impl TurnPolicy for ResolutionMismatch {
    fn name(&self) -> &'static str {
        "resolution_mismatch"
    }

    fn on_belief(&mut self, belief: &BeliefRecord) -> Option<PolicyHit> {
        let logit = belief.logit?;
        if !contains_resolution_language(&belief.claim) {
            return None;
        }

        if belief.said >= self.said_threshold && logit <= self.logit_threshold {
            return Some(PolicyHit {
                policy: self.name(),
                reason: format!(
                    "claims resolved: {:.2} said / {:.2} logit",
                    belief.said, logit
                ),
                decision: GateDecision::Block,
            });
        }

        None
    }

    fn on_tool_result(&mut self, _: &ToolResultRecord) -> Option<PolicyHit> {
        None
    }
}

pub fn count_deleted_lines(command: &str) -> usize {
    command
        .lines()
        .filter(|line| line.starts_with('-') && !line.starts_with("---"))
        .count()
}

fn all_similar(claims: &[&str], threshold: f32) -> bool {
    for left in 0..claims.len() {
        for right in left + 1..claims.len() {
            if jaccard_similarity(claims[left], claims[right]) < threshold {
                return false;
            }
        }
    }
    true
}

fn jaccard_similarity(left: &str, right: &str) -> f32 {
    let left = normalize_words(left);
    let right = normalize_words(right);
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }

    let intersection = left.iter().filter(|word| right.contains(*word)).count() as f32;
    let union = left.len() + right.len() - intersection as usize;
    intersection / union as f32
}

fn normalize_words(input: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &["the", "a", "an", "is", "in", "at", "to", "of", "and", "now"];
    let mut words = input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(canonical_word)
        .filter(|word| !STOPWORDS.contains(&word.as_str()))
        .collect::<Vec<_>>();
    words.sort();
    words.dedup();
    words
}

fn canonical_word(word: &str) -> String {
    let mut word = word.to_ascii_lowercase();
    if word.ends_with("ing") && word.len() > 5 {
        word.truncate(word.len() - 3);
    } else if word.ends_with("ed") && word.len() > 4 {
        word.truncate(word.len() - 2);
    }
    if word.ends_with('e') && word.len() > 4 {
        word.truncate(word.len() - 1);
    }
    if word.ends_with('s') && word.len() > 3 {
        word.truncate(word.len() - 1);
    }
    word
}

fn contains_resolution_language(claim: &str) -> bool {
    const RESOLUTION_WORDS: &[&str] = &[
        "fixed",
        "resolved",
        "done",
        "works now",
        "should work",
        "passes",
        "passing",
        "complete",
        "solved",
    ];

    let lower = claim.to_ascii_lowercase();
    RESOLUTION_WORDS.iter().any(|word| lower.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn belief(claim: &str, said: f32, logit: Option<f32>) -> BeliefRecord {
        BeliefRecord {
            claim: claim.to_owned(),
            confidence: said,
            source: "tests".to_owned(),
            said,
            logit,
            raw: claim.to_owned(),
            parse_error: None,
        }
    }

    #[test]
    fn policy_nonzero_exit() {
        let mut policy = NonZeroExit::new();
        assert!(policy
            .on_tool_result(&ToolResultRecord {
                gear: 1,
                command: "cargo test".to_owned(),
                exit_code: Some(1),
                deleted_lines: 0
            })
            .is_none());
        assert!(policy
            .on_tool_result(&ToolResultRecord {
                gear: 2,
                command: "cargo test".to_owned(),
                exit_code: Some(2),
                deleted_lines: 0
            })
            .is_some());
        assert!(policy
            .on_tool_result(&ToolResultRecord {
                gear: 3,
                command: "cargo test".to_owned(),
                exit_code: Some(0),
                deleted_lines: 0
            })
            .is_none());
        assert!(policy
            .on_tool_result(&ToolResultRecord {
                gear: 4,
                command: "cargo test".to_owned(),
                exit_code: Some(1),
                deleted_lines: 0
            })
            .is_none());
    }

    #[test]
    fn policy_thrash() {
        let mut policy = ThrashDetector::new();
        let _ = policy.on_tool_result(&ToolResultRecord {
            gear: 1,
            command: "cargo test".to_owned(),
            exit_code: Some(1),
            deleted_lines: 0,
        });
        let _ = policy.on_belief(&belief(
            "duplicate key handling in parser is broken",
            0.3,
            Some(0.3),
        ));
        let _ = policy.on_tool_result(&ToolResultRecord {
            gear: 2,
            command: "cargo test".to_owned(),
            exit_code: Some(1),
            deleted_lines: 0,
        });
        let _ = policy.on_belief(&belief(
            "the parser mishandles duplicate keys",
            0.3,
            Some(0.3),
        ));
        let _ = policy.on_tool_result(&ToolResultRecord {
            gear: 3,
            command: "cargo test".to_owned(),
            exit_code: Some(0),
            deleted_lines: 0,
        });
        let hit = policy.on_belief(&belief("parser drops duplicated keys", 0.3, Some(0.3)));
        assert!(hit.is_some());

        let mut policy = ThrashDetector::new();
        let _ = policy.on_tool_result(&ToolResultRecord {
            gear: 1,
            command: "a".to_owned(),
            exit_code: Some(1),
            deleted_lines: 0,
        });
        let _ = policy.on_belief(&belief("fix parser", 0.3, Some(0.3)));
        let _ = policy.on_tool_result(&ToolResultRecord {
            gear: 2,
            command: "b".to_owned(),
            exit_code: Some(1),
            deleted_lines: 0,
        });
        let _ = policy.on_belief(&belief("update docs", 0.3, Some(0.3)));
        let _ = policy.on_tool_result(&ToolResultRecord {
            gear: 3,
            command: "c".to_owned(),
            exit_code: Some(1),
            deleted_lines: 0,
        });
        assert!(policy
            .on_belief(&belief("refactor tests", 0.3, Some(0.3)))
            .is_none());

        let mut policy = ThrashDetector::new();
        let _ = policy.on_belief(&belief(
            "duplicate key handling in parser is broken",
            0.3,
            Some(0.3),
        ));
        let _ = policy.on_belief(&belief(
            "the parser mishandles duplicate keys",
            0.3,
            Some(0.3),
        ));
        assert!(policy
            .on_belief(&belief("parser drops duplicated keys", 0.3, Some(0.3)))
            .is_none());
    }

    #[test]
    fn policy_resolution_mismatch() {
        let mut policy = ResolutionMismatch::new();
        let hit = policy.on_belief(&belief("fixed the duplicate key bug", 0.85, Some(0.4)));
        assert!(hit
            .as_ref()
            .is_some_and(|entry| entry.reason.contains("0.85 said / 0.40 logit")));
        assert!(policy
            .on_belief(&belief("fixed the duplicate key bug", 0.85, Some(0.8)))
            .is_none());
        assert!(policy
            .on_belief(&belief("fixed the duplicate key bug", 0.85, None))
            .is_none());
        assert!(policy
            .on_belief(&belief("investigating duplicate keys", 0.85, Some(0.4)))
            .is_none());
    }
}
