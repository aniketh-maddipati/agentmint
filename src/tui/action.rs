//! Actions, deferred effects, and the pure reducer that advances TUI state.
//! Used by: the input translator and future terminal-driver and executor phases.

use crate::aerf::intervention::{CorrectionFork, CorrectionPreview};
use crate::tui::model::{AppState, OperationId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    PreviousEpisode,
    NextEpisode,
    PreviousEvidence,
    NextEvidence,
    NextRegion,
    PreviousRegion,
    EnterScrub,
    ExitScrub,
    BeginContextEdit,
    CommitContextEdit,
    CancelContextEdit,
    ClearContext,
    InsertChar(char),
    EditorCursorLeft,
    EditorCursorRight,
    EditorCursorHome,
    EditorCursorEnd,
    EditorBackspace,
    EditorDelete,
    RequestPreview,
    PreviewResolved {
        operation_id: OperationId,
        preview: Box<CorrectionPreview>,
    },
    PreviewUnavailable {
        operation_id: OperationId,
    },
    PreviewFailed {
        operation_id: OperationId,
        message: String,
    },
    RequestFork,
    ForkResolved {
        operation_id: OperationId,
        fork: Box<CorrectionFork>,
    },
    ForkFailed {
        operation_id: OperationId,
        message: String,
    },
    RequestHandoffCopy,
    Resize {
        width: u16,
        height: u16,
    },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    PreviewCorrection {
        operation_id: OperationId,
        episode_id: String,
        staged_context: String,
    },
    ConfirmFork {
        operation_id: OperationId,
        episode_id: String,
        staged_context: String,
    },
    CopyHandoff {
        text: String,
    },
}

#[derive(Debug)]
pub struct Transition {
    pub state: AppState,
    pub effects: Vec<Effect>,
}

pub fn reduce(mut state: AppState, action: Action) -> Transition {
    let mut effects = Vec::new();
    match action {
        Action::PreviousEpisode => state.select_previous_episode(),
        Action::NextEpisode => state.select_next_episode(),
        Action::PreviousEvidence => state.select_previous_evidence(),
        Action::NextEvidence => state.select_next_evidence(),
        Action::NextRegion => state.focus_next(),
        Action::PreviousRegion => state.focus_previous(),
        Action::EnterScrub => state.enter_scrub(),
        Action::ExitScrub => state.exit_scrub(),
        Action::BeginContextEdit => state.begin_context_edit(),
        Action::CommitContextEdit => state.commit_context_edit(),
        Action::CancelContextEdit => state.cancel_context_edit(),
        Action::ClearContext => state.clear_context(),
        Action::InsertChar(ch) => state.insert_char(ch),
        Action::EditorCursorLeft => state.editor_cursor_left(),
        Action::EditorCursorRight => state.editor_cursor_right(),
        Action::EditorCursorHome => state.editor_cursor_home(),
        Action::EditorCursorEnd => state.editor_cursor_end(),
        Action::EditorBackspace => state.editor_backspace(),
        Action::EditorDelete => state.editor_delete(),
        Action::RequestPreview => {
            if let Some((operation_id, episode_id)) = state.begin_preview() {
                effects.push(Effect::PreviewCorrection {
                    operation_id,
                    episode_id,
                    staged_context: state.staged_context().to_owned(),
                });
            }
        }
        Action::PreviewResolved {
            operation_id,
            preview,
        } => state.resolve_preview(operation_id, preview),
        Action::PreviewUnavailable { operation_id } => state.mark_preview_unavailable(operation_id),
        Action::PreviewFailed {
            operation_id,
            message,
        } => state.mark_preview_failed(operation_id, message),
        Action::RequestFork => {
            if let Some((operation_id, episode_id)) = state.begin_fork() {
                effects.push(Effect::ConfirmFork {
                    operation_id,
                    episode_id,
                    staged_context: state.staged_context().to_owned(),
                });
            }
        }
        Action::ForkResolved { operation_id, fork } => state.resolve_fork(operation_id, fork),
        Action::ForkFailed {
            operation_id,
            message,
        } => state.mark_fork_failed(operation_id, message),
        Action::RequestHandoffCopy => {
            if let Some(text) = state.handoff_copy_text() {
                effects.push(Effect::CopyHandoff { text });
            }
        }
        Action::Resize { width, height } => state.set_viewport(width, height),
        Action::Quit => state.request_quit(),
    }
    Transition { state, effects }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::aerf::adapters::codex_app_server::{CodexInitializeHandshake, CodexTranscript};
    use crate::aerf::intervention::{
        apply_recorded_demo_correction, generate_recorded_demo_correction_preview,
        CorrectionExecution, CorrectionExecutionMode, CorrectionFork, CorrectionPreview, Run,
        TerminalHandoff, TokenUsage, RECORDED_DEMO_SCHEMA_EPISODE_ID,
    };
    use crate::tui::model::{
        AppState, DisplayMode, FocusRegion, ForkState, OperationId, PreviewState,
    };

    const FIXTURE: &[u8] =
        include_bytes!("../../tests/fixtures/interventions/retry-after-v1-v2.json");

    fn fixture_run() -> Run {
        Run::from_slice(FIXTURE).expect("fixture run")
    }

    fn state() -> AppState {
        AppState::new(Arc::new(fixture_run()))
    }

    fn empty_state() -> AppState {
        let mut run = fixture_run();
        run.episodes.clear();
        AppState::new(Arc::new(run))
    }

    fn apply(state: AppState, action: Action) -> AppState {
        reduce(state, action).state
    }

    fn schema_preview() -> Box<CorrectionPreview> {
        Box::new(
            generate_recorded_demo_correction_preview(
                &fixture_run(),
                RECORDED_DEMO_SCHEMA_EPISODE_ID,
            )
            .expect("schema preview"),
        )
    }

    fn offline_fork() -> Box<CorrectionFork> {
        Box::new(
            apply_recorded_demo_correction(&fixture_run(), RECORDED_DEMO_SCHEMA_EPISODE_ID)
                .expect("offline fork"),
        )
    }

    fn live_fork_with_handoff(copy_text: &str) -> Box<CorrectionFork> {
        let mut fork =
            apply_recorded_demo_correction(&fixture_run(), RECORDED_DEMO_SCHEMA_EPISODE_ID)
                .expect("offline fork");
        fork.execution = Some(CorrectionExecution {
            mode: CorrectionExecutionMode::LiveCodex,
            checkpoint_id: "checkpoint-abc".to_owned(),
            worktree_path: "/tmp/agentmint-worktree".to_owned(),
            branch_name: "agentmint/attempt".to_owned(),
            detached_head: false,
            codex_version: "0.130.0".to_owned(),
            thread_id: Some("thread-1".to_owned()),
            attempt_id: "attempt-codex-corrected-from-schema".to_owned(),
            verification_command: vec!["cargo".to_owned(), "test".to_owned()],
            verification_exit_code: 0,
            actual_patch: "diff".to_owned(),
            actual_changed_paths: vec!["src/lib.rs".to_owned()],
            captured_token_usage: TokenUsage {
                input_tokens: 1,
                cached_input_tokens: 0,
                output_tokens: 1,
                total_tokens: 2,
            },
            handoff: TerminalHandoff {
                directory: "/tmp/agentmint-worktree".to_owned(),
                branch_state: "branch agentmint/attempt".to_owned(),
                checkpoint_id: "checkpoint-abc".to_owned(),
                attempt_id: "attempt-codex-corrected-from-schema".to_owned(),
                verification_command: vec!["cargo".to_owned(), "test".to_owned()],
                copy_text: copy_text.to_owned(),
            },
            demo_limitations: Vec::new(),
            codex_transcript: CodexTranscript {
                codex_version: "0.130.0".to_owned(),
                handshake: CodexInitializeHandshake {
                    codex_version: "0.130.0".to_owned(),
                    initialize_request: json!({"method": "initialize"}),
                    initialize_response: json!({"result": {}}),
                    initialized_notification: json!({"method": "initialized"}),
                },
                events: Vec::new(),
            },
        });
        Box::new(fork)
    }

    fn at_schema_episode() -> AppState {
        apply(state(), Action::NextEpisode)
    }

    fn preview_operation_id(effects: &[Effect]) -> OperationId {
        match effects {
            [Effect::PreviewCorrection { operation_id, .. }] => *operation_id,
            other => panic!("expected one preview effect, got {other:?}"),
        }
    }

    fn fork_operation_id(effects: &[Effect]) -> OperationId {
        match effects {
            [Effect::ConfirmFork { operation_id, .. }] => *operation_id,
            other => panic!("expected one fork effect, got {other:?}"),
        }
    }

    fn preview_ready_at_schema() -> AppState {
        let transition = reduce(at_schema_episode(), Action::RequestPreview);
        let operation_id = preview_operation_id(&transition.effects);
        apply(
            transition.state,
            Action::PreviewResolved {
                operation_id,
                preview: schema_preview(),
            },
        )
    }

    #[test]
    fn episode_navigation_clamps_at_both_edges() {
        let state = apply(state(), Action::PreviousEpisode);
        assert_eq!(state.episode_index(), 0);

        let mut state = state;
        for _ in 0..10 {
            state = apply(state, Action::NextEpisode);
        }
        assert_eq!(state.episode_index(), 5);

        let state = apply(state, Action::NextEpisode);
        assert_eq!(state.episode_index(), 5);
    }

    #[test]
    fn evidence_navigation_clamps_at_both_edges() {
        let state = apply(state(), Action::PreviousEvidence);
        assert_eq!(state.evidence_index(), 0);

        let mut state = state;
        for _ in 0..50 {
            state = apply(state, Action::NextEvidence);
        }
        let evidence_len = state.selected_episode().expect("episode").evidence.len();
        assert_eq!(state.evidence_index(), evidence_len - 1);
    }

    #[test]
    fn changing_episode_resets_evidence_selection() {
        let mut run = fixture_run();
        let extra = run.episodes[0].evidence[0].clone();
        run.episodes[0].evidence.push(extra);
        let state = AppState::new(Arc::new(run));

        let state = apply(state, Action::NextEvidence);
        assert_eq!(state.evidence_index(), 1);
        let state = apply(state, Action::NextEpisode);
        assert_eq!(state.evidence_index(), 0);
    }

    #[test]
    fn focus_cycles_forward_and_backward() {
        let state = apply(state(), Action::NextRegion);
        assert_eq!(state.focus(), FocusRegion::Evidence);
        let state = apply(state, Action::NextRegion);
        assert_eq!(state.focus(), FocusRegion::Context);
        let state = apply(state, Action::PreviousRegion);
        assert_eq!(state.focus(), FocusRegion::Evidence);
        let state = apply(state, Action::PreviousRegion);
        let state = apply(state, Action::PreviousRegion);
        assert_eq!(state.focus(), FocusRegion::Detail);
    }

    #[test]
    fn inline_and_scrub_transitions() {
        let state = apply(state(), Action::EnterScrub);
        assert_eq!(state.display_mode(), DisplayMode::Scrub);
        let state = apply(state, Action::ExitScrub);
        assert_eq!(state.display_mode(), DisplayMode::Inline);
    }

    #[test]
    fn context_commit_persists_and_cancel_restores() {
        let mut state = apply(state(), Action::BeginContextEdit);
        for ch in "note".chars() {
            state = apply(state, Action::InsertChar(ch));
        }
        let state = apply(state, Action::CommitContextEdit);
        assert_eq!(state.staged_context(), "note");
        assert!(!state.is_editing());

        let mut state = apply(state, Action::BeginContextEdit);
        for ch in "-edit".chars() {
            state = apply(state, Action::InsertChar(ch));
        }
        let state = apply(state, Action::CancelContextEdit);
        assert_eq!(state.staged_context(), "note");
        assert!(!state.is_editing());
    }

    #[test]
    fn clear_context_empties_staged_value() {
        let mut state = apply(state(), Action::BeginContextEdit);
        for ch in "keep".chars() {
            state = apply(state, Action::InsertChar(ch));
        }
        let state = apply(state, Action::CommitContextEdit);
        assert_eq!(state.staged_context(), "keep");
        let state = apply(state, Action::ClearContext);
        assert_eq!(state.staged_context(), "");
    }

    #[test]
    fn unicode_insertion_deletion_and_cursor_movement() {
        let mut state = apply(state(), Action::BeginContextEdit);
        for ch in "café→é".chars() {
            state = apply(state, Action::InsertChar(ch));
        }
        assert_eq!(state.editor_text().as_deref(), Some("café→é"));
        assert_eq!(state.editor_cursor(), Some(6));

        let state = apply(state, Action::EditorBackspace);
        assert_eq!(state.editor_text().as_deref(), Some("café→"));
        assert_eq!(state.editor_cursor(), Some(5));

        let state = apply(state, Action::EditorCursorHome);
        assert_eq!(state.editor_cursor(), Some(0));
        let state = apply(state, Action::EditorCursorRight);
        assert_eq!(state.editor_cursor(), Some(1));
        let state = apply(state, Action::EditorDelete);
        assert_eq!(state.editor_text().as_deref(), Some("cfé→"));
        let state = apply(state, Action::EditorCursorEnd);
        assert_eq!(state.editor_cursor(), Some(4));
        let state = apply(state, Action::EditorCursorLeft);
        assert_eq!(state.editor_cursor(), Some(3));
    }

    #[test]
    fn staged_context_is_snapshotted_into_preview_effect() {
        let mut state = apply(state(), Action::BeginContextEdit);
        for ch in "ctx".chars() {
            state = apply(state, Action::InsertChar(ch));
        }
        let state = apply(state, Action::CommitContextEdit);
        let transition = reduce(state, Action::RequestPreview);
        assert_eq!(
            transition.effects,
            vec![Effect::PreviewCorrection {
                operation_id: OperationId(0),
                episode_id: "episode-inspect-api".to_owned(),
                staged_context: "ctx".to_owned(),
            }]
        );
    }

    #[test]
    fn preview_request_emits_exactly_one_effect() {
        let transition = reduce(state(), Action::RequestPreview);
        assert_eq!(
            transition.effects,
            vec![Effect::PreviewCorrection {
                operation_id: OperationId(0),
                episode_id: "episode-inspect-api".to_owned(),
                staged_context: String::new(),
            }]
        );
        assert!(matches!(
            transition.state.preview(),
            PreviewState::Pending { .. }
        ));
    }

    #[test]
    fn duplicate_pending_preview_request_emits_no_effect() {
        let transition = reduce(state(), Action::RequestPreview);
        let transition = reduce(transition.state, Action::RequestPreview);
        assert!(transition.effects.is_empty());
    }

    #[test]
    fn preview_unavailable_and_failed_states_are_recorded() {
        let transition = reduce(state(), Action::RequestPreview);
        let operation_id = preview_operation_id(&transition.effects);
        let state = apply(
            transition.state,
            Action::PreviewUnavailable { operation_id },
        );
        assert!(matches!(state.preview(), PreviewState::Unavailable { .. }));

        let transition = reduce(state, Action::RequestPreview);
        let operation_id = preview_operation_id(&transition.effects);
        let state = apply(
            transition.state,
            Action::PreviewFailed {
                operation_id,
                message: "boom".to_owned(),
            },
        );
        assert!(
            matches!(state.preview(), PreviewState::Failed { message, .. } if message == "boom")
        );
    }

    #[test]
    fn stale_preview_completion_is_ignored() {
        let transition = reduce(state(), Action::RequestPreview);
        let state = apply(
            transition.state,
            Action::PreviewResolved {
                operation_id: OperationId(999),
                preview: schema_preview(),
            },
        );
        assert!(matches!(state.preview(), PreviewState::Pending { .. }));
    }

    #[test]
    fn changing_episode_invalidates_the_old_preview() {
        let state = preview_ready_at_schema();
        assert!(matches!(state.preview(), PreviewState::Ready { .. }));
        let state = apply(state, Action::NextEpisode);
        assert_eq!(state.preview(), &PreviewState::Idle);
    }

    #[test]
    fn fork_cannot_start_without_ready_preview() {
        let transition = reduce(at_schema_episode(), Action::RequestFork);
        assert!(transition.effects.is_empty());
        assert_eq!(transition.state.fork(), &ForkState::Idle);
    }

    #[test]
    fn fork_request_emits_exactly_one_effect() {
        let transition = reduce(preview_ready_at_schema(), Action::RequestFork);
        assert_eq!(
            transition.effects,
            vec![Effect::ConfirmFork {
                operation_id: OperationId(1),
                episode_id: RECORDED_DEMO_SCHEMA_EPISODE_ID.to_owned(),
                staged_context: String::new(),
            }]
        );
        assert!(matches!(transition.state.fork(), ForkState::Pending { .. }));
    }

    #[test]
    fn duplicate_pending_fork_request_emits_no_effect() {
        let transition = reduce(preview_ready_at_schema(), Action::RequestFork);
        let transition = reduce(transition.state, Action::RequestFork);
        assert!(transition.effects.is_empty());
    }

    #[test]
    fn stale_fork_completion_is_ignored() {
        let transition = reduce(preview_ready_at_schema(), Action::RequestFork);
        let state = apply(
            transition.state,
            Action::ForkResolved {
                operation_id: OperationId(999),
                fork: offline_fork(),
            },
        );
        assert!(matches!(state.fork(), ForkState::Pending { .. }));
    }

    #[test]
    fn fork_resolves_for_matching_operation() {
        let transition = reduce(preview_ready_at_schema(), Action::RequestFork);
        let operation_id = fork_operation_id(&transition.effects);
        let state = apply(
            transition.state,
            Action::ForkResolved {
                operation_id,
                fork: offline_fork(),
            },
        );
        assert!(matches!(state.fork(), ForkState::Ready { .. }));
    }

    #[test]
    fn handoff_copy_only_emits_when_available() {
        let transition = reduce(preview_ready_at_schema(), Action::RequestFork);
        let operation_id = fork_operation_id(&transition.effects);
        let state = apply(
            transition.state,
            Action::ForkResolved {
                operation_id,
                fork: offline_fork(),
            },
        );
        let transition = reduce(state, Action::RequestHandoffCopy);
        assert!(transition.effects.is_empty());

        let transition = reduce(preview_ready_at_schema(), Action::RequestFork);
        let operation_id = fork_operation_id(&transition.effects);
        let state = apply(
            transition.state,
            Action::ForkResolved {
                operation_id,
                fork: live_fork_with_handoff("cd /tmp/agentmint-worktree"),
            },
        );
        let transition = reduce(state, Action::RequestHandoffCopy);
        assert_eq!(
            transition.effects,
            vec![Effect::CopyHandoff {
                text: "cd /tmp/agentmint-worktree".to_owned(),
            }]
        );
    }

    #[test]
    fn empty_run_is_safe_under_every_action_category() {
        let actions = [
            Action::PreviousEpisode,
            Action::NextEpisode,
            Action::PreviousEvidence,
            Action::NextEvidence,
            Action::NextRegion,
            Action::PreviousRegion,
            Action::EnterScrub,
            Action::ExitScrub,
            Action::BeginContextEdit,
            Action::InsertChar('x'),
            Action::CommitContextEdit,
            Action::RequestPreview,
            Action::RequestFork,
            Action::RequestHandoffCopy,
            Action::Resize {
                width: 10,
                height: 4,
            },
        ];
        let mut state = empty_state();
        let mut total_effects = 0;
        for action in actions {
            let transition = reduce(state, action);
            total_effects += transition.effects.len();
            state = transition.state;
        }
        assert_eq!(total_effects, 0);
        assert!(state.selected_episode().is_none());
    }

    #[test]
    fn quit_and_resize_transitions() {
        let state = apply(
            state(),
            Action::Resize {
                width: 120,
                height: 40,
            },
        );
        assert_eq!(state.viewport().width, 120);
        assert_eq!(state.viewport().height, 40);
        assert!(!state.quit_requested());
        let state = apply(state, Action::Quit);
        assert!(state.quit_requested());
    }
}
