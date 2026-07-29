//! Focused parsing coverage for normalized intervention fixtures.
//! Used by: cargo test acceptance for intervention schema stability.

use std::error::Error;
use std::fs;
use std::path::Path;

use agentmint::aerf::intervention::{
    build_semantic_episodes, AttemptOutcome, Evidence, InputEnvelope, InputItem, RawEvent,
    RawEventOutcome, RawEventParseState, RawEventSource, Run,
};
use serde_json::json;

#[test]
fn parses_retry_after_v1_v2_fixture() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/interventions/retry-after-v1-v2.json");
    let run = Run::from_slice(&fs::read(path)?)?;

    assert_eq!(run.raw_events.len(), 8);
    assert_eq!(run.episodes.len(), 6);
    assert_eq!(run.raw_events[0].event_id, "inspect-api-1");
    assert_eq!(run.raw_events[7].event_id, "passing-test-1");
    assert_eq!(run.attempts.len(), 2);
    assert_eq!(run.attempts[0].endpoint, "/v1/responses");
    assert_eq!(run.attempts[0].retry_after_ms, Some(2000));
    assert_eq!(run.attempts[1].endpoint, "/v2/responses");
    assert_eq!(run.attempts[1].outcome, AttemptOutcome::Succeeded);
    assert_eq!(run.raw_events[5].status_code, Some(500));
    assert_eq!(run.raw_events[7].status_code, Some(200));
    assert_eq!(run.token_usage.cached_input_tokens, 48);

    Ok(())
}

#[test]
fn golden_episode_labels_and_order_match_fixture() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/interventions/retry-after-v1-v2.json");
    let run = Run::from_slice(&fs::read(path)?)?;
    let built = run.build_episodes();

    assert_eq!(built, run.episodes);
    assert_eq!(
        built
            .iter()
            .map(|episode| episode.title.as_str())
            .collect::<Vec<_>>(),
        vec![
            "inspect API",
            "edit schema",
            "update client",
            "failed test",
            "revise fixture",
            "passing test"
        ]
    );

    Ok(())
}

#[test]
fn golden_episode_evidence_stays_with_raw_event_group() -> Result<(), Box<dyn Error>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/interventions/retry-after-v1-v2.json");
    let run = Run::from_slice(&fs::read(path)?)?;
    let built = build_semantic_episodes(&run.raw_events);

    for episode in &built {
        for evidence in &episode.evidence {
            match evidence {
                Evidence::ObservedFact {
                    source_event_id, ..
                }
                | Evidence::AgentStatement {
                    source_event_id, ..
                } => assert!(episode.raw_event_ids.contains(source_event_id)),
                Evidence::DerivedInference { derived_from, .. } => {
                    assert!(derived_from
                        .iter()
                        .all(|event_id| episode.raw_event_ids.contains(event_id)));
                }
                Evidence::HumanContext { .. } => {}
            }
        }
    }

    assert_eq!(
        built[0].raw_event_ids,
        vec!["inspect-api-1", "inspect-api-2", "inspect-api-3"]
    );

    Ok(())
}

#[test]
fn malformed_and_truncated_events_recover_deterministically() {
    let events = vec![
        raw_event(
            "inspect-a",
            None,
            Some("docs/api/v1/responses.md"),
            None,
            RawEventParseState::Malformed,
        ),
        raw_event(
            "inspect-b",
            Some("read_file"),
            Some("docs/api/v2/responses.md"),
            None,
            RawEventParseState::Truncated,
        ),
        raw_event(
            "pass-1",
            Some("run_tests"),
            None,
            Some(RawEventOutcome::Succeeded),
            RawEventParseState::Complete,
        ),
    ];

    let built = build_semantic_episodes(&events);

    assert_eq!(
        built
            .iter()
            .map(|episode| episode.title.as_str())
            .collect::<Vec<_>>(),
        vec!["inspect API", "passing test"]
    );
    assert_eq!(built[0].raw_event_ids, vec!["inspect-a", "inspect-b"]);
    assert!(matches!(
        built[0].evidence[1],
        Evidence::DerivedInference { .. }
    ));
}

#[test]
fn rejects_cached_input_that_is_not_in_input() {
    let invalid = json!({
        "run_id": "run-invalid",
        "session_id": "session-invalid",
        "provider": "openai",
        "model": "gpt-test",
        "request": {
            "input": [
                { "type": "message", "role": "user", "content": "only one input item" }
            ],
            "cached_input": [
                { "type": "message", "role": "system", "content": "not present in input" }
            ]
        },
        "raw_events": [],
        "episodes": [],
        "interventions": [],
        "correction_packets": [],
        "attempts": [],
        "verification": {
            "verified_at": "2026-07-28T12:10:00Z",
            "verdict": "needs_human_review"
        },
        "token_usage": {
            "input_tokens": 1,
            "cached_input_tokens": 1,
            "output_tokens": 0,
            "total_tokens": 2
        }
    });

    let err = serde_json::from_value::<Run>(invalid).expect_err("subset error");

    assert!(err
        .to_string()
        .contains("cached_input must be a subset of input"));
}

fn raw_event(
    event_id: &str,
    tool_name: Option<&str>,
    file_path: Option<&str>,
    outcome: Option<RawEventOutcome>,
    parse_state: RawEventParseState,
) -> RawEvent {
    RawEvent {
        event_id: event_id.to_owned(),
        recorded_at: "2026-07-28T12:00:00Z".to_owned(),
        source: RawEventSource::Recorder,
        endpoint: "/local/workspace".to_owned(),
        request: InputEnvelope {
            input: vec![InputItem::Message {
                role: "user".to_owned(),
                content: "repair fixture".to_owned(),
            }],
            cached_input: Vec::new(),
        },
        tool_name: tool_name.map(str::to_owned),
        file_path: file_path.map(str::to_owned),
        summary: file_path.map(str::to_owned),
        outcome,
        parse_state,
        status_code: None,
        retry_after_ms: None,
        body: None,
    }
}
