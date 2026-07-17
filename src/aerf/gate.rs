//! Minimum-spec gate engine: pattern, value, requires, cross_ref and loop rules.
//! Used by: session receipt emission and the Tier 1 gate suite.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct ToolRules {
    pub blocked_patterns: HashMap<String, Vec<String>>,
    pub blocked_values: HashMap<String, Vec<String>>,
    pub requires: Vec<String>,
    pub cross_ref: Vec<CrossRef>,
}

#[derive(Debug, Clone)]
pub struct CrossRef {
    pub field: String,
    pub other_tool: String,
    pub other_field: String,
}

#[derive(Debug, Clone)]
pub struct Spec {
    pub tools: HashMap<String, ToolRules>,
    pub max_identical_calls: usize,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub tool: String,
    pub args: Value,
}

impl ToolCall {
    pub fn new(tool: &str, args: Value) -> Self {
        Self {
            tool: tool.to_owned(),
            args,
        }
    }

    pub fn field(&self, name: &str) -> Option<&Value> {
        self.args.get(name)
    }

    pub fn is_identical(&self, other: &ToolCall) -> bool {
        self.tool == other.tool && self.args == other.args
    }
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub call: ToolCall,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockRule {
    BlockedPattern,
    BlockedValue,
    MissingRequirement,
    CrossRefMismatch,
    LoopBreaker,
}

impl BlockRule {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockRule::BlockedPattern => "blocked_patterns",
            BlockRule::BlockedValue => "blocked_values",
            BlockRule::MissingRequirement => "requires",
            BlockRule::CrossRefMismatch => "cross_ref",
            BlockRule::LoopBreaker => "loop_breaker",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Block { rule: BlockRule, reason: String },
}

impl Verdict {
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass)
    }

    pub fn is_block(&self) -> bool {
        !self.is_pass()
    }

    pub fn rule(&self) -> Option<BlockRule> {
        match self {
            Verdict::Pass => None,
            Verdict::Block { rule, .. } => Some(*rule),
        }
    }
}

pub fn evaluate(spec: &Spec, history: &[HistoryEntry], call: &ToolCall) -> Verdict {
    if let Some(rules) = spec.tools.get(&call.tool) {
        if let Some(verdict) = check_blocked_patterns(rules, call) {
            return verdict;
        }
        if let Some(verdict) = check_blocked_values(rules, call) {
            return verdict;
        }
        if let Some(verdict) = check_requires(rules, history) {
            return verdict;
        }
        if let Some(verdict) = check_cross_ref(rules, history, call) {
            return verdict;
        }
    }
    if let Some(verdict) = check_loop_breaker(spec, history, call) {
        return verdict;
    }
    Verdict::Pass
}

pub fn identical_call_count(history: &[HistoryEntry], call: &ToolCall) -> usize {
    history
        .iter()
        .filter(|entry| entry.call.is_identical(call))
        .count()
}

fn check_blocked_patterns(rules: &ToolRules, call: &ToolCall) -> Option<Verdict> {
    for (field, patterns) in &rules.blocked_patterns {
        let Some(value) = call.field(field).and_then(Value::as_str) else {
            continue;
        };
        for pattern in patterns {
            if pattern_matches(pattern, value) {
                return Some(block(
                    BlockRule::BlockedPattern,
                    format!("field '{field}' matched blocked pattern '{pattern}'"),
                ));
            }
        }
    }
    None
}

fn check_blocked_values(rules: &ToolRules, call: &ToolCall) -> Option<Verdict> {
    for (field, values) in &rules.blocked_values {
        let Some(value) = call.field(field).and_then(Value::as_str) else {
            continue;
        };
        if values.iter().any(|blocked| blocked == value) {
            return Some(block(
                BlockRule::BlockedValue,
                format!("field '{field}' equals blocked value '{value}'"),
            ));
        }
    }
    None
}

fn check_requires(rules: &ToolRules, history: &[HistoryEntry]) -> Option<Verdict> {
    for required in &rules.requires {
        let satisfied = history
            .iter()
            .any(|entry| entry.passed && &entry.call.tool == required);
        if !satisfied {
            return Some(block(
                BlockRule::MissingRequirement,
                format!("requires a prior successful '{required}' in this session"),
            ));
        }
    }
    None
}

fn check_cross_ref(rules: &ToolRules, history: &[HistoryEntry], call: &ToolCall) -> Option<Verdict> {
    for cross in &rules.cross_ref {
        let Some(this_value) = call.field(&cross.field) else {
            return Some(block(
                BlockRule::CrossRefMismatch,
                format!("field '{}' absent, cannot satisfy cross_ref", cross.field),
            ));
        };
        let matched = history.iter().any(|entry| {
            entry.passed
                && entry.call.tool == cross.other_tool
                && entry.call.field(&cross.other_field) == Some(this_value)
        });
        if !matched {
            return Some(block(
                BlockRule::CrossRefMismatch,
                format!(
                    "field '{}'={} matches no prior {}.{}",
                    cross.field,
                    render(this_value),
                    cross.other_tool,
                    cross.other_field
                ),
            ));
        }
    }
    None
}

fn check_loop_breaker(spec: &Spec, history: &[HistoryEntry], call: &ToolCall) -> Option<Verdict> {
    let prior = identical_call_count(history, call);
    if prior >= spec.max_identical_calls {
        return Some(block(
            BlockRule::LoopBreaker,
            format!(
                "identical {{tool, args}} repeated {} times (max {})",
                prior + 1,
                spec.max_identical_calls
            ),
        ));
    }
    None
}

fn block(rule: BlockRule, reason: String) -> Verdict {
    Verdict::Block { rule, reason }
}

fn render(value: &Value) -> String {
    match value {
        Value::String(string) => string.clone(),
        other => other.to_string(),
    }
}

fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern.contains('*') || pattern.contains('?') {
        return glob_matches(pattern, value);
    }
    value.contains(pattern)
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let value: Vec<char> = value.chars().collect();
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star_index: Option<usize> = None;
    let mut resume_index = 0;

    while value_index < value.len() {
        let literal = pattern_index < pattern.len()
            && (pattern[pattern_index] == value[value_index] || pattern[pattern_index] == '?');
        if literal {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_index = Some(pattern_index);
            resume_index = value_index;
            pattern_index += 1;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            resume_index += 1;
            value_index = resume_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn spec() -> Spec {
        let mut tools = HashMap::new();

        let mut read = ToolRules::default();
        read.blocked_patterns
            .insert("path".into(), vec!["*.env".into()]);
        tools.insert("read_file".into(), read);

        let mut write = ToolRules::default();
        write.requires = vec!["read_file".into()];
        write.cross_ref = vec![CrossRef {
            field: "path".into(),
            other_tool: "read_file".into(),
            other_field: "path".into(),
        }];
        tools.insert("write_file".into(), write);

        let mut command = ToolRules::default();
        command
            .blocked_patterns
            .insert("command".into(), vec!["rm -rf".into()]);
        tools.insert("run_command".into(), command);

        Spec {
            tools,
            max_identical_calls: 3,
        }
    }

    fn passed(tool: &str, args: Value) -> HistoryEntry {
        HistoryEntry {
            call: ToolCall::new(tool, args),
            passed: true,
        }
    }

    #[test]
    fn glob_blocks_env_and_allows_source() {
        assert!(glob_matches("*.env", ".env"));
        assert!(glob_matches("*.env", "config/prod.env"));
        assert!(!glob_matches("*.env", "src/app.ts"));
    }

    #[test]
    fn substring_matches_full_untruncated_string() {
        let padded = format!("{}rm -rf /", "true && ".repeat(50));
        let call = ToolCall::new("run_command", json!({ "command": padded }));

        let verdict = evaluate(&spec(), &[], &call);

        assert_eq!(verdict.rule(), Some(BlockRule::BlockedPattern));
    }

    #[test]
    fn write_without_read_is_blocked() {
        let call = ToolCall::new("write_file", json!({ "path": "src/app.ts" }));

        let verdict = evaluate(&spec(), &[], &call);

        assert_eq!(verdict.rule(), Some(BlockRule::MissingRequirement));
    }

    #[test]
    fn cross_ref_blocks_mismatched_file() {
        let history = vec![passed("read_file", json!({ "path": "src/app.ts" }))];
        let call = ToolCall::new("write_file", json!({ "path": "src/other.ts" }));

        let verdict = evaluate(&spec(), &history, &call);

        assert_eq!(verdict.rule(), Some(BlockRule::CrossRefMismatch));
    }

    #[test]
    fn cross_ref_allows_matched_file() {
        let history = vec![passed("read_file", json!({ "path": "src/app.ts" }))];
        let call = ToolCall::new("write_file", json!({ "path": "src/app.ts" }));

        assert!(evaluate(&spec(), &history, &call).is_pass());
    }

    #[test]
    fn loop_breaker_blocks_fourth_identical_call() {
        let call = ToolCall::new("run_tests", json!({}));
        let history: Vec<HistoryEntry> = (0..3)
            .map(|_| HistoryEntry {
                call: call.clone(),
                passed: true,
            })
            .collect();

        assert!(evaluate(&spec(), &history[..2], &call).is_pass());
        assert_eq!(
            evaluate(&spec(), &history, &call).rule(),
            Some(BlockRule::LoopBreaker)
        );
    }
}
