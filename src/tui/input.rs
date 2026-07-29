//! Pure crossterm-event to action translation, mode-aware for navigation and context editing.
//! Used by: the future terminal driver that feeds the TUI reducer.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::tui::action::Action;
use crate::tui::model::{AppState, DisplayMode};

pub fn translate(event: Event, state: &AppState) -> Option<Action> {
    match event {
        Event::Resize(width, height) => Some(Action::Resize { width, height }),
        Event::Key(key) => translate_key(key, state),
        _ => None,
    }
}

fn translate_key(key: KeyEvent, state: &AppState) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if is_ctrl_c(&key) {
        return Some(Action::Quit);
    }
    if state.is_editing() {
        return translate_editing(key);
    }
    translate_navigation(key, state)
}

fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
}

fn translate_editing(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Esc => Some(Action::CancelContextEdit),
        KeyCode::Enter => Some(Action::CommitContextEdit),
        KeyCode::Backspace => Some(Action::EditorBackspace),
        KeyCode::Delete => Some(Action::EditorDelete),
        KeyCode::Left => Some(Action::EditorCursorLeft),
        KeyCode::Right => Some(Action::EditorCursorRight),
        KeyCode::Home => Some(Action::EditorCursorHome),
        KeyCode::End => Some(Action::EditorCursorEnd),
        KeyCode::Char(ch) if is_insertable(&key) => Some(Action::InsertChar(ch)),
        _ => None,
    }
}

fn is_insertable(key: &KeyEvent) -> bool {
    !key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT)
}

fn translate_navigation(key: KeyEvent, state: &AppState) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Esc => Some(Action::ExitScrub),
        KeyCode::Enter => enter_scrub(state),
        KeyCode::Tab => Some(Action::NextRegion),
        KeyCode::BackTab => Some(Action::PreviousRegion),
        KeyCode::Left | KeyCode::Char('h') => Some(Action::PreviousEpisode),
        KeyCode::Right | KeyCode::Char('l') => Some(Action::NextEpisode),
        KeyCode::Up | KeyCode::Char('k') => Some(Action::PreviousEvidence),
        KeyCode::Down | KeyCode::Char('j') => Some(Action::NextEvidence),
        KeyCode::Char('i') => Some(Action::BeginContextEdit),
        KeyCode::Char('p') => Some(Action::RequestPreview),
        KeyCode::Char('f') => Some(Action::RequestFork),
        KeyCode::Char('y') => Some(Action::RequestHandoffCopy),
        KeyCode::Char('q') => Some(Action::Quit),
        _ => None,
    }
}

fn enter_scrub(state: &AppState) -> Option<Action> {
    match state.display_mode() {
        DisplayMode::Inline => Some(Action::EnterScrub),
        DisplayMode::Scrub => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyEventState, MouseButton, MouseEvent, MouseEventKind};

    use super::*;
    use crate::aerf::intervention::Run;
    use crate::tui::action::reduce;

    const FIXTURE: &[u8] =
        include_bytes!("../../tests/fixtures/interventions/retry-after-v1-v2.json");

    fn state() -> AppState {
        AppState::new(Arc::new(Run::from_slice(FIXTURE).expect("fixture run")))
    }

    fn editing_state() -> AppState {
        reduce(state(), Action::BeginContextEdit).state
    }

    fn press(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn press_mod(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn navigation_bindings_map_to_actions() {
        let state = state();
        let cases = [
            (press(KeyCode::Char('q')), Action::Quit),
            (press(KeyCode::Enter), Action::EnterScrub),
            (press(KeyCode::Esc), Action::ExitScrub),
            (press(KeyCode::Tab), Action::NextRegion),
            (press(KeyCode::BackTab), Action::PreviousRegion),
            (press(KeyCode::Left), Action::PreviousEpisode),
            (press(KeyCode::Char('h')), Action::PreviousEpisode),
            (press(KeyCode::Right), Action::NextEpisode),
            (press(KeyCode::Char('l')), Action::NextEpisode),
            (press(KeyCode::Up), Action::PreviousEvidence),
            (press(KeyCode::Char('k')), Action::PreviousEvidence),
            (press(KeyCode::Down), Action::NextEvidence),
            (press(KeyCode::Char('j')), Action::NextEvidence),
            (press(KeyCode::Char('i')), Action::BeginContextEdit),
            (press(KeyCode::Char('p')), Action::RequestPreview),
            (press(KeyCode::Char('f')), Action::RequestFork),
            (press(KeyCode::Char('y')), Action::RequestHandoffCopy),
        ];
        for (event, expected) in cases {
            assert_eq!(translate(event, &state), Some(expected));
        }
    }

    #[test]
    fn ctrl_c_quits_in_navigation_and_editing() {
        let event = press_mod(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(translate(event.clone(), &state()), Some(Action::Quit));
        assert_eq!(translate(event, &editing_state()), Some(Action::Quit));
    }

    #[test]
    fn enter_only_enters_scrub_from_inline() {
        let scrub = reduce(state(), Action::EnterScrub).state;
        assert_eq!(translate(press(KeyCode::Enter), &scrub), None);
    }

    #[test]
    fn resize_translates_to_resize_action() {
        assert_eq!(
            translate(Event::Resize(100, 30), &state()),
            Some(Action::Resize {
                width: 100,
                height: 30,
            })
        );
    }

    #[test]
    fn key_release_events_are_suppressed() {
        let mut key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert_eq!(translate(Event::Key(key), &state()), None);
    }

    #[test]
    fn key_repeat_events_translate_normally() {
        let mut key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Repeat;
        assert_eq!(
            translate(Event::Key(key), &state()),
            Some(Action::NextEvidence)
        );
    }

    #[test]
    fn mouse_events_are_suppressed() {
        let event = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 3,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(translate(event, &state()), None);
    }

    #[test]
    fn editing_inserts_printable_command_letters_as_text() {
        let state = editing_state();
        for ch in ['q', 'p', 'f', 'h', 'j', 'k', 'l', 'i', 'y'] {
            assert_eq!(
                translate(press(KeyCode::Char(ch)), &state),
                Some(Action::InsertChar(ch))
            );
        }
    }

    #[test]
    fn editing_inserts_unicode_characters() {
        let state = editing_state();
        assert_eq!(
            translate(press(KeyCode::Char('é')), &state),
            Some(Action::InsertChar('é'))
        );
        assert_eq!(
            translate(press(KeyCode::Char('→')), &state),
            Some(Action::InsertChar('→'))
        );
    }

    #[test]
    fn editing_key_precedence_over_navigation() {
        let state = editing_state();
        let cases = [
            (press(KeyCode::Esc), Action::CancelContextEdit),
            (press(KeyCode::Enter), Action::CommitContextEdit),
            (press(KeyCode::Backspace), Action::EditorBackspace),
            (press(KeyCode::Delete), Action::EditorDelete),
            (press(KeyCode::Left), Action::EditorCursorLeft),
            (press(KeyCode::Right), Action::EditorCursorRight),
            (press(KeyCode::Home), Action::EditorCursorHome),
            (press(KeyCode::End), Action::EditorCursorEnd),
        ];
        for (event, expected) in cases {
            assert_eq!(translate(event, &state), Some(expected));
        }
    }

    #[test]
    fn tab_does_not_change_panels_while_editing() {
        assert_eq!(translate(press(KeyCode::Tab), &editing_state()), None);
    }

    #[test]
    fn control_modified_navigation_keys_are_ignored() {
        let event = Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert_eq!(translate(event, &state()), None);
    }

    #[test]
    fn unhandled_key_returns_none() {
        let event = Event::Key(KeyEvent {
            code: KeyCode::F(5),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        assert_eq!(translate(event, &state()), None);
    }
}
