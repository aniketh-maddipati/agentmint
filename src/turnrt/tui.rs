//! Minimal scrollback-friendly terminal interaction for turn.
//! Used by: the turn loop.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use colored::Colorize;
use crossterm::cursor::MoveToColumn;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, MouseEvent, MouseEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{execute, queue};

use crate::turnrt::belief::BeliefRecord;

const SCRUB_SCROLL_TICKS_PER_STEP: i16 = 3;
const SCRUB_DRAG_COLUMNS_PER_STEP: i16 = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum PauseTrigger {
    Key,
    Mouse,
    Policy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GearView {
    pub gear: usize,
    pub frontier: usize,
    pub claim: String,
    pub said: f32,
    pub logit: Option<f32>,
    pub command: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InputState {
    pub pause_requested: Option<PauseTrigger>,
    pub approved: bool,
    pub quit: bool,
    pub staged_correction: Option<String>,
    pub live_input: String,
    pub anchor: usize,
    frontier: usize,
    scroll_accumulator: i16,
    drag_accumulator: i16,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    pub fn new() -> Self {
        Self {
            pause_requested: None,
            approved: false,
            quit: false,
            staged_correction: None,
            live_input: String::new(),
            anchor: 0,
            frontier: 0,
            scroll_accumulator: 0,
            drag_accumulator: 0,
        }
    }

    pub fn set_frontier(&mut self, frontier: usize) {
        self.frontier = frontier;
        self.anchor = self.anchor.min(frontier);
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(' ') => self.pause_requested = Some(PauseTrigger::Key),
            KeyCode::Char('y') => self.approved = true,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Left => self.move_anchor(-1),
            KeyCode::Right => self.move_anchor(1),
            KeyCode::Enter => {
                if !self.live_input.is_empty() {
                    self.staged_correction = Some(std::mem::take(&mut self.live_input));
                }
            }
            KeyCode::Backspace => {
                self.live_input.pop();
            }
            KeyCode::Char(ch) => {
                self.live_input.push(ch);
            }
            _ => {}
        }
    }

    pub fn on_mouse(&mut self, event: MouseEvent, paused: bool) {
        if !paused {
            self.pause_requested = Some(PauseTrigger::Mouse);
            return;
        }

        match event.kind {
            MouseEventKind::ScrollLeft => self.apply_scroll(-1),
            MouseEventKind::ScrollRight => self.apply_scroll(1),
            MouseEventKind::Drag(_) => self.apply_drag(event.column as i16),
            MouseEventKind::Down(_) => self.pause_requested = Some(PauseTrigger::Mouse),
            _ => {}
        }
    }

    pub fn frontier(&self) -> usize {
        self.frontier
    }

    fn apply_scroll(&mut self, direction: i16) {
        self.scroll_accumulator += direction;
        if self.scroll_accumulator.abs() >= SCRUB_SCROLL_TICKS_PER_STEP {
            let steps = self.scroll_accumulator / SCRUB_SCROLL_TICKS_PER_STEP;
            self.move_anchor(steps.into());
            self.scroll_accumulator %= SCRUB_SCROLL_TICKS_PER_STEP;
        }
    }

    fn apply_drag(&mut self, delta: i16) {
        self.drag_accumulator += delta;
        if self.drag_accumulator.abs() >= SCRUB_DRAG_COLUMNS_PER_STEP {
            let steps = self.drag_accumulator / SCRUB_DRAG_COLUMNS_PER_STEP;
            self.move_anchor(steps.into());
            self.drag_accumulator %= SCRUB_DRAG_COLUMNS_PER_STEP;
        }
    }

    fn move_anchor(&mut self, delta: isize) {
        let next = self.anchor as isize + delta;
        self.anchor = next.clamp(0, self.frontier as isize) as usize;
    }
}

pub trait TerminalControl: Send + Sync {
    fn enable_raw_mode(&self) -> std::io::Result<()>;
    fn disable_raw_mode(&self) -> std::io::Result<()>;
    fn enable_mouse_capture(&self) -> std::io::Result<()>;
    fn disable_mouse_capture(&self) -> std::io::Result<()>;
}

pub struct CrosstermControl;

impl TerminalControl for CrosstermControl {
    fn enable_raw_mode(&self) -> std::io::Result<()> {
        enable_raw_mode()
    }

    fn disable_raw_mode(&self) -> std::io::Result<()> {
        disable_raw_mode()
    }

    fn enable_mouse_capture(&self) -> std::io::Result<()> {
        execute!(std::io::stdout(), EnableMouseCapture)
    }

    fn disable_mouse_capture(&self) -> std::io::Result<()> {
        execute!(std::io::stdout(), DisableMouseCapture)
    }
}

pub struct CleanupHandle {
    control: Arc<dyn TerminalControl>,
    cleaned: AtomicBool,
}

impl CleanupHandle {
    pub fn new(control: Arc<dyn TerminalControl>) -> Self {
        Self {
            control,
            cleaned: AtomicBool::new(false),
        }
    }

    pub fn cleanup(&self) {
        if self.cleaned.swap(true, Ordering::SeqCst) {
            return;
        }

        let _ = self.control.disable_mouse_capture();
        let _ = self.control.disable_raw_mode();
    }
}

pub fn install_panic_cleanup(cleanup: Arc<CleanupHandle>) {
    std::panic::set_hook(Box::new(move |_| {
        cleanup.cleanup();
    }));
}

pub fn format_gear_line(view: &GearView) -> String {
    let delta = view
        .logit
        .map(|logit| (view.said - logit).abs())
        .unwrap_or_default();
    let marker = if delta > 0.25 {
        "⚠".yellow().to_string()
    } else {
        String::new()
    };
    let logit = view
        .logit
        .map(|value| format!("{value:.2} logit"))
        .unwrap_or_else(|| "n/a logit".to_owned());
    format!(
        "gear {} ▸ \"{}\" {:.2} said · {} {} ▸ exit {}",
        view.gear,
        view.claim,
        view.said,
        logit,
        marker,
        view.exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_owned())
    )
}

pub fn render_stream<W: Write>(writer: &mut W, staged: &str) -> std::io::Result<()> {
    queue!(writer, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    write!(writer, "{}", staged)?;
    writer.flush()
}

pub fn render_pause_card<W: Write>(
    writer: &mut W,
    reason: &str,
    belief: Option<&BeliefRecord>,
    command: &str,
) -> std::io::Result<()> {
    writeln!(writer, "{}", reason)?;
    if let Some(belief) = belief {
        writeln!(
            writer,
            "belief: \"{}\" {:.2} said · {}",
            belief.claim,
            belief.said,
            belief
                .logit
                .map(|value| format!("{value:.2} logit"))
                .unwrap_or_else(|| "n/a logit".to_owned())
        )?;
    } else {
        writeln!(writer, "belief: unavailable")?;
    }
    writeln!(writer, "tool: {}", command)?;
    writeln!(
        writer,
        "y approve · type to correct · drag/←→ scrub · q quit"
    )?;
    writer.flush()
}

pub fn shared_input_state() -> Arc<Mutex<InputState>> {
    Arc::new(Mutex::new(InputState::new()))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyModifiers, MouseButton};

    use super::*;

    #[test]
    fn scrub_input() {
        let mut input = InputState::new();
        input.set_frontier(7);
        input.on_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollRight,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            true,
        );
        input.on_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollRight,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            true,
        );
        input.on_mouse(
            MouseEvent {
                kind: MouseEventKind::ScrollRight,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            true,
        );
        assert_eq!(input.anchor, 1);

        input.on_mouse(
            MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 16,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            true,
        );
        assert_eq!(input.anchor, 3);

        input.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(input.anchor, 2);

        for _ in 0..20 {
            input.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        }
        assert_eq!(input.anchor, 7);
    }

    #[test]
    fn terminal_restore() {
        #[derive(Default)]
        struct FakeControl {
            output: Arc<Mutex<Vec<&'static str>>>,
        }

        impl TerminalControl for FakeControl {
            fn enable_raw_mode(&self) -> std::io::Result<()> {
                Ok(())
            }

            fn disable_raw_mode(&self) -> std::io::Result<()> {
                self.output.lock().expect("lock").push("raw_off");
                Ok(())
            }

            fn enable_mouse_capture(&self) -> std::io::Result<()> {
                Ok(())
            }

            fn disable_mouse_capture(&self) -> std::io::Result<()> {
                self.output.lock().expect("lock").push("\u{1b}[?1006l");
                Ok(())
            }
        }

        let output = Arc::new(Mutex::new(Vec::new()));
        let cleanup = Arc::new(CleanupHandle::new(Arc::new(FakeControl {
            output: output.clone(),
        })));
        cleanup.cleanup();
        cleanup.cleanup();
        let values = output.lock().expect("lock").clone();
        assert_eq!(values, vec!["\u{1b}[?1006l", "raw_off"]);
    }
}
