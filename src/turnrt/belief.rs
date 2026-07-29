//! Belief and tool fence parsing for streamed turn output.
//! Used by: engine consumers, loop policies, and tape export.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const BELIEF_PROMPT_CONVENTION: &str = r#"Before every tool call, emit exactly one fenced block:
```belief
{"claim":"<one sentence>","confidence":0.0,"source":"<what evidence>"}
```
then a second fenced block:
```tool
{"name":"shell","args":{"cmd":"..."}}
```"#;

const BELIEF_OPEN: &str = "```belief";
const TOOL_OPEN: &str = "```tool";
const FENCE_CLOSE: &str = "\n```";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeliefRecord {
    pub claim: String,
    pub confidence: f32,
    pub source: String,
    pub said: f32,
    pub logit: Option<f32>,
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRecord {
    pub name: String,
    pub args: Value,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParserEvent {
    Belief(BeliefRecord),
    Tool(ToolCallRecord),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedBelief {
    pub claim: String,
    pub confidence: f32,
    pub source: String,
}

#[derive(Debug, Clone)]
struct TokenTrace {
    end_char: usize,
    logprob: Option<f32>,
}

#[derive(Debug, Default)]
pub struct StreamBeliefParser {
    buffer: String,
    tokens: Vec<TokenTrace>,
    search_from: usize,
    belief_done: bool,
    tool_done: bool,
    events: Vec<ParserEvent>,
}

impl StreamBeliefParser {
    pub fn feed(&mut self, text_delta: &str, token_logprob: Option<f32>) {
        if text_delta.is_empty() {
            return;
        }

        self.buffer.push_str(text_delta);
        self.tokens.push(TokenTrace {
            end_char: self.buffer.len(),
            logprob: token_logprob,
        });
        self.advance();
    }

    pub fn take_events(&mut self) -> Vec<ParserEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn belief(&self) -> Option<BeliefRecord> {
        self.events.iter().find_map(|event| match event {
            ParserEvent::Belief(record) => Some(record.clone()),
            ParserEvent::Tool(_) => None,
        })
    }

    pub fn tool(&self) -> Option<ToolCallRecord> {
        self.events.iter().find_map(|event| match event {
            ParserEvent::Tool(record) => Some(record.clone()),
            ParserEvent::Belief(_) => None,
        })
    }

    pub fn parse_belief_block(input: &str, logit: Option<f32>) -> BeliefRecord {
        match serde_json::from_str::<ParsedBelief>(input) {
            Ok(parsed) => BeliefRecord {
                claim: parsed.claim,
                confidence: parsed.confidence,
                source: parsed.source,
                said: parsed.confidence,
                logit,
                raw: input.to_owned(),
                parse_error: None,
            },
            Err(error) => BeliefRecord {
                claim: String::new(),
                confidence: 0.0,
                source: String::new(),
                said: 0.0,
                logit,
                raw: input.to_owned(),
                parse_error: Some(error.to_string()),
            },
        }
    }

    pub fn parse_missing_belief(raw: &str) -> BeliefRecord {
        BeliefRecord {
            claim: String::new(),
            confidence: 0.0,
            source: String::new(),
            said: 0.0,
            logit: None,
            raw: raw.to_owned(),
            parse_error: Some("missing belief block".to_owned()),
        }
    }

    fn advance(&mut self) {
        if !self.belief_done {
            if let Some((open, close)) =
                find_fenced_block(&self.buffer, BELIEF_OPEN, self.search_from)
            {
                let body_start = open + BELIEF_OPEN.len();
                let body = self.buffer[body_start..close]
                    .trim_start_matches('\n')
                    .to_owned();
                let logit = mean_logprob_to_signal(self.logprobs_for_span(body_start, close));
                let belief = Self::parse_belief_block(&body, logit);
                self.events.push(ParserEvent::Belief(belief));
                self.belief_done = true;
                self.search_from = close + FENCE_CLOSE.len();
            }
        }

        if self.belief_done && !self.tool_done {
            if let Some((open, close)) =
                find_fenced_block(&self.buffer, TOOL_OPEN, self.search_from)
            {
                let body_start = open + TOOL_OPEN.len();
                let body = self.buffer[body_start..close]
                    .trim_start_matches('\n')
                    .to_owned();
                let record = match serde_json::from_str::<ToolEnvelope>(&body) {
                    Ok(tool) => ToolCallRecord {
                        name: tool.name,
                        args: tool.args,
                        raw: body,
                    },
                    Err(_) => ToolCallRecord {
                        name: String::new(),
                        args: Value::Null,
                        raw: body,
                    },
                };
                self.events.push(ParserEvent::Tool(record));
                self.tool_done = true;
                self.search_from = close + FENCE_CLOSE.len();
            }
        }
    }

    fn logprobs_for_span(&self, start: usize, end: usize) -> Vec<f32> {
        let mut previous_end = 0usize;
        let mut result = Vec::new();

        for token in &self.tokens {
            let token_start = previous_end;
            let token_end = token.end_char;
            previous_end = token_end;

            if token_end <= start || token_start >= end {
                continue;
            }

            if let Some(logprob) = token.logprob {
                result.push(logprob);
            }
        }

        result
    }
}

#[derive(Debug, Deserialize)]
struct ToolEnvelope {
    name: String,
    args: Value,
}

impl<'de> Deserialize<'de> for ParsedBelief {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawBelief {
            claim: String,
            confidence: f32,
            source: String,
        }

        let raw = RawBelief::deserialize(deserializer)?;
        Ok(Self {
            claim: raw.claim,
            confidence: raw.confidence,
            source: raw.source,
        })
    }
}

pub fn mean_logprob_to_signal(logprobs: Vec<f32>) -> Option<f32> {
    if logprobs.is_empty() {
        return None;
    }

    let mean = logprobs.iter().sum::<f32>() / logprobs.len() as f32;
    Some(mean.exp().clamp(0.0, 1.0))
}

fn find_fenced_block(buffer: &str, open: &str, from: usize) -> Option<(usize, usize)> {
    let open_index = buffer[from..].find(open)? + from;
    let close_start =
        buffer[open_index + open.len()..].find(FENCE_CLOSE)? + open_index + open.len();
    Some((open_index, close_start))
}

#[cfg(test)]
mod tests {
    use std::f32::consts::LN_2;

    use super::*;

    #[test]
    fn belief_parse() {
        let mut parser = StreamBeliefParser::default();
        parser.feed(
            "```belief\n{\"claim\":\"fixed bug\",\"confidence\":0.8,\"source\":\"tests\"}\n```",
            Some(-0.1),
        );
        let events = parser.take_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParserEvent::Belief(record) => {
                assert_eq!(record.claim, "fixed bug");
                assert_eq!(record.said, 0.8);
                assert!(record.logit.expect("logit") > 0.0);
            }
            ParserEvent::Tool(_) => panic!("expected belief"),
        }
    }

    #[test]
    fn belief_split_across_chunks() {
        let mut parser = StreamBeliefParser::default();
        parser.feed("```belief\n{\"claim\":\"parser ", Some(-0.2));
        parser.feed(
            "ok\",\"confidence\":0.6,\"source\":\"logs\"}\n```",
            Some(-0.2),
        );
        let events = parser.take_events();
        assert_eq!(events.len(), 1);
        match &events[0] {
            ParserEvent::Belief(record) => assert_eq!(record.claim, "parser ok"),
            ParserEvent::Tool(_) => panic!("expected belief"),
        }
    }

    #[test]
    fn belief_malformed_json() {
        let record = StreamBeliefParser::parse_belief_block("{\"claim\":", Some(0.5));
        assert!(record.parse_error.is_some());
        assert_eq!(record.raw, "{\"claim\":");
    }

    #[test]
    fn belief_missing_block() {
        let record = StreamBeliefParser::parse_missing_belief("no belief here");
        assert_eq!(record.parse_error.as_deref(), Some("missing belief block"));
    }

    #[test]
    fn logit_mapping() {
        let signal = mean_logprob_to_signal(vec![-LN_2, -LN_2]).expect("signal");
        assert!((signal - 0.5).abs() < 0.01);
        assert_eq!(mean_logprob_to_signal(Vec::new()), None);
    }
}
