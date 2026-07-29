//! Pure application state, selection invariants, and staged-context editing for the TUI core.
//! Used by: the TUI reducer, input translator, and future Ratatui rendering phases.

use std::sync::Arc;

use crate::aerf::intervention::{
    CorrectionFork, CorrectionPreview, Episode, Evidence, Run, TerminalHandoff,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Inline,
    Scrub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusRegion {
    Episodes,
    Evidence,
    Context,
    Detail,
}

impl FocusRegion {
    fn next(self) -> Self {
        match self {
            Self::Episodes => Self::Evidence,
            Self::Evidence => Self::Context,
            Self::Context => Self::Detail,
            Self::Detail => Self::Episodes,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Episodes => Self::Detail,
            Self::Evidence => Self::Episodes,
            Self::Context => Self::Evidence,
            Self::Detail => Self::Context,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLifecycle {
    Running,
    QuitRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewState {
    Idle,
    Pending {
        operation_id: OperationId,
        episode_id: String,
    },
    Ready {
        episode_id: String,
        preview: Box<CorrectionPreview>,
    },
    Unavailable {
        episode_id: String,
    },
    Failed {
        episode_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkState {
    Idle,
    Pending {
        operation_id: OperationId,
        episode_id: String,
    },
    Ready {
        episode_id: String,
        fork: Box<CorrectionFork>,
    },
    Failed {
        episode_id: String,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContextEditor {
    buffer: Vec<char>,
    cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextState {
    staged: String,
    editor: Option<ContextEditor>,
}

impl ContextState {
    fn new() -> Self {
        Self {
            staged: String::new(),
            editor: None,
        }
    }

    fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    fn begin(&mut self) {
        if self.editor.is_some() {
            return;
        }
        let buffer: Vec<char> = self.staged.chars().collect();
        let cursor = buffer.len();
        self.editor = Some(ContextEditor { buffer, cursor });
    }

    fn insert(&mut self, ch: char) {
        if let Some(editor) = self.editor.as_mut() {
            editor.buffer.insert(editor.cursor, ch);
            editor.cursor += 1;
        }
    }

    fn backspace(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.cursor > 0 {
                editor.cursor -= 1;
                editor.buffer.remove(editor.cursor);
            }
        }
    }

    fn delete(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.cursor < editor.buffer.len() {
                editor.buffer.remove(editor.cursor);
            }
        }
    }

    fn cursor_left(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.cursor = editor.cursor.saturating_sub(1);
        }
    }

    fn cursor_right(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.cursor = (editor.cursor + 1).min(editor.buffer.len());
        }
    }

    fn cursor_home(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.cursor = 0;
        }
    }

    fn cursor_end(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.cursor = editor.buffer.len();
        }
    }

    fn commit(&mut self) {
        if let Some(editor) = self.editor.take() {
            self.staged = editor.buffer.into_iter().collect();
        }
    }

    fn cancel(&mut self) {
        self.editor = None;
    }

    fn clear(&mut self) {
        self.staged = String::new();
        self.editor = None;
    }

    fn staged(&self) -> &str {
        &self.staged
    }

    fn editor_cursor(&self) -> Option<usize> {
        self.editor.as_ref().map(|editor| editor.cursor)
    }

    fn editor_text(&self) -> Option<String> {
        self.editor
            .as_ref()
            .map(|editor| editor.buffer.iter().collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    run: Arc<Run>,
    mode: DisplayMode,
    focus: FocusRegion,
    episode_index: usize,
    evidence_index: usize,
    viewport: Viewport,
    context: ContextState,
    preview: PreviewState,
    fork: ForkState,
    next_operation: u64,
    lifecycle: RunLifecycle,
}

impl AppState {
    pub fn new(run: Arc<Run>) -> Self {
        Self {
            run,
            mode: DisplayMode::Inline,
            focus: FocusRegion::Episodes,
            episode_index: 0,
            evidence_index: 0,
            viewport: Viewport {
                width: 0,
                height: 0,
            },
            context: ContextState::new(),
            preview: PreviewState::Idle,
            fork: ForkState::Idle,
            next_operation: 0,
            lifecycle: RunLifecycle::Running,
        }
    }

    pub fn run(&self) -> &Run {
        &self.run
    }

    pub fn display_mode(&self) -> DisplayMode {
        self.mode
    }

    pub fn focus(&self) -> FocusRegion {
        self.focus
    }

    pub fn episode_index(&self) -> usize {
        self.episode_index
    }

    pub fn evidence_index(&self) -> usize {
        self.evidence_index
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn preview(&self) -> &PreviewState {
        &self.preview
    }

    pub fn fork(&self) -> &ForkState {
        &self.fork
    }

    pub fn quit_requested(&self) -> bool {
        matches!(self.lifecycle, RunLifecycle::QuitRequested)
    }

    pub fn is_editing(&self) -> bool {
        self.context.is_editing()
    }

    pub fn staged_context(&self) -> &str {
        self.context.staged()
    }

    pub fn editor_cursor(&self) -> Option<usize> {
        self.context.editor_cursor()
    }

    pub fn editor_text(&self) -> Option<String> {
        self.context.editor_text()
    }

    pub fn selected_episode(&self) -> Option<&Episode> {
        self.run.episodes.get(self.episode_index)
    }

    pub fn selected_episode_id(&self) -> Option<&str> {
        self.selected_episode()
            .map(|episode| episode.episode_id.as_str())
    }

    pub fn selected_evidence(&self) -> Option<&Evidence> {
        self.selected_episode()?.evidence.get(self.evidence_index)
    }

    pub fn current_handoff(&self) -> Option<&TerminalHandoff> {
        match &self.fork {
            ForkState::Ready { fork, .. } => {
                fork.execution.as_ref().map(|execution| &execution.handoff)
            }
            _ => None,
        }
    }

    pub(crate) fn select_next_episode(&mut self) {
        let len = self.run.episodes.len();
        if len == 0 {
            return;
        }
        let previous = self.episode_index;
        if self.episode_index + 1 < len {
            self.episode_index += 1;
        }
        if self.episode_index != previous {
            self.on_episode_changed();
        }
    }

    pub(crate) fn select_previous_episode(&mut self) {
        if self.run.episodes.is_empty() {
            return;
        }
        let previous = self.episode_index;
        self.episode_index = self.episode_index.saturating_sub(1);
        if self.episode_index != previous {
            self.on_episode_changed();
        }
    }

    pub(crate) fn select_next_evidence(&mut self) {
        let Some(len) = self
            .selected_episode()
            .map(|episode| episode.evidence.len())
        else {
            return;
        };
        if self.evidence_index + 1 < len {
            self.evidence_index += 1;
        }
    }

    pub(crate) fn select_previous_evidence(&mut self) {
        if self.selected_episode().is_none() {
            return;
        }
        self.evidence_index = self.evidence_index.saturating_sub(1);
    }

    fn on_episode_changed(&mut self) {
        self.evidence_index = 0;
        self.preview = PreviewState::Idle;
    }

    pub(crate) fn focus_next(&mut self) {
        self.focus = self.focus.next();
    }

    pub(crate) fn focus_previous(&mut self) {
        self.focus = self.focus.previous();
    }

    pub(crate) fn enter_scrub(&mut self) {
        self.mode = DisplayMode::Scrub;
    }

    pub(crate) fn exit_scrub(&mut self) {
        self.mode = DisplayMode::Inline;
    }

    pub(crate) fn set_viewport(&mut self, width: u16, height: u16) {
        self.viewport = Viewport { width, height };
    }

    pub(crate) fn request_quit(&mut self) {
        self.lifecycle = RunLifecycle::QuitRequested;
    }

    pub(crate) fn begin_context_edit(&mut self) {
        self.context.begin();
    }

    pub(crate) fn insert_char(&mut self, ch: char) {
        self.context.insert(ch);
    }

    pub(crate) fn editor_backspace(&mut self) {
        self.context.backspace();
    }

    pub(crate) fn editor_delete(&mut self) {
        self.context.delete();
    }

    pub(crate) fn editor_cursor_left(&mut self) {
        self.context.cursor_left();
    }

    pub(crate) fn editor_cursor_right(&mut self) {
        self.context.cursor_right();
    }

    pub(crate) fn editor_cursor_home(&mut self) {
        self.context.cursor_home();
    }

    pub(crate) fn editor_cursor_end(&mut self) {
        self.context.cursor_end();
    }

    pub(crate) fn commit_context_edit(&mut self) {
        self.context.commit();
    }

    pub(crate) fn cancel_context_edit(&mut self) {
        self.context.cancel();
    }

    pub(crate) fn clear_context(&mut self) {
        self.context.clear();
    }

    pub(crate) fn begin_preview(&mut self) -> Option<(OperationId, String)> {
        let episode_id = self.selected_episode_id()?.to_owned();
        if matches!(self.preview, PreviewState::Pending { .. }) {
            return None;
        }
        let operation_id = self.allocate_operation();
        self.preview = PreviewState::Pending {
            operation_id,
            episode_id: episode_id.clone(),
        };
        Some((operation_id, episode_id))
    }

    pub(crate) fn resolve_preview(
        &mut self,
        operation_id: OperationId,
        preview: Box<CorrectionPreview>,
    ) {
        let episode_id = match &self.preview {
            PreviewState::Pending {
                operation_id: pending,
                episode_id,
            } if *pending == operation_id => episode_id.clone(),
            _ => return,
        };
        self.preview = PreviewState::Ready {
            episode_id,
            preview,
        };
    }

    pub(crate) fn mark_preview_unavailable(&mut self, operation_id: OperationId) {
        let episode_id = match &self.preview {
            PreviewState::Pending {
                operation_id: pending,
                episode_id,
            } if *pending == operation_id => episode_id.clone(),
            _ => return,
        };
        self.preview = PreviewState::Unavailable { episode_id };
    }

    pub(crate) fn mark_preview_failed(&mut self, operation_id: OperationId, message: String) {
        let episode_id = match &self.preview {
            PreviewState::Pending {
                operation_id: pending,
                episode_id,
            } if *pending == operation_id => episode_id.clone(),
            _ => return,
        };
        self.preview = PreviewState::Failed {
            episode_id,
            message,
        };
    }

    pub(crate) fn begin_fork(&mut self) -> Option<(OperationId, String)> {
        let selected = self.selected_episode_id()?.to_owned();
        let ready = matches!(
            &self.preview,
            PreviewState::Ready { episode_id, .. } if *episode_id == selected
        );
        if !ready {
            return None;
        }
        if matches!(self.fork, ForkState::Pending { .. }) {
            return None;
        }
        let operation_id = self.allocate_operation();
        self.fork = ForkState::Pending {
            operation_id,
            episode_id: selected.clone(),
        };
        Some((operation_id, selected))
    }

    pub(crate) fn resolve_fork(&mut self, operation_id: OperationId, fork: Box<CorrectionFork>) {
        let episode_id = match &self.fork {
            ForkState::Pending {
                operation_id: pending,
                episode_id,
            } if *pending == operation_id => episode_id.clone(),
            _ => return,
        };
        self.fork = ForkState::Ready { episode_id, fork };
    }

    pub(crate) fn mark_fork_failed(&mut self, operation_id: OperationId, message: String) {
        let episode_id = match &self.fork {
            ForkState::Pending {
                operation_id: pending,
                episode_id,
            } if *pending == operation_id => episode_id.clone(),
            _ => return,
        };
        self.fork = ForkState::Failed {
            episode_id,
            message,
        };
    }

    pub(crate) fn handoff_copy_text(&self) -> Option<String> {
        self.current_handoff()
            .map(|handoff| handoff.copy_text.clone())
    }

    fn allocate_operation(&mut self) -> OperationId {
        let id = OperationId(self.next_operation);
        self.next_operation += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] =
        include_bytes!("../../tests/fixtures/interventions/retry-after-v1-v2.json");

    fn fixture_run() -> Arc<Run> {
        Arc::new(Run::from_slice(FIXTURE).expect("fixture run"))
    }

    fn empty_run() -> Arc<Run> {
        let mut run = Run::from_slice(FIXTURE).expect("fixture run");
        run.episodes.clear();
        Arc::new(run)
    }

    #[test]
    fn initial_selection_targets_first_episode_and_evidence() {
        let state = AppState::new(fixture_run());
        assert_eq!(state.episode_index(), 0);
        assert_eq!(state.evidence_index(), 0);
        assert_eq!(state.display_mode(), DisplayMode::Inline);
        assert_eq!(state.focus(), FocusRegion::Episodes);
        assert!(!state.quit_requested());
        assert!(!state.is_editing());
    }

    #[test]
    fn selected_accessors_resolve_against_the_fixture() {
        let state = AppState::new(fixture_run());
        assert_eq!(state.run().episodes.len(), 6);
        assert_eq!(state.selected_episode_id(), Some("episode-inspect-api"));
        assert!(state.selected_episode().is_some());
        assert!(state.selected_evidence().is_some());
    }

    #[test]
    fn empty_run_accessors_return_none_without_panic() {
        let state = AppState::new(empty_run());
        assert!(state.selected_episode().is_none());
        assert_eq!(state.selected_episode_id(), None);
        assert!(state.selected_evidence().is_none());
        assert!(state.current_handoff().is_none());
    }

    #[test]
    fn episode_without_evidence_is_valid() {
        let mut run = Run::from_slice(FIXTURE).expect("fixture run");
        run.episodes[0].evidence.clear();
        let state = AppState::new(Arc::new(run));
        assert!(state.selected_episode().is_some());
        assert!(state.selected_evidence().is_none());
    }

    #[test]
    fn handoff_available_only_after_live_fork_with_execution() {
        let state = AppState::new(fixture_run());
        assert!(state.current_handoff().is_none());
        assert!(state.handoff_copy_text().is_none());
    }
}
