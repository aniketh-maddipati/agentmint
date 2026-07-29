//! Minimal scrollback-friendly terminal interaction for turn.
//! Used by: the turn loop.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use colored::Colorize;
use crossterm::cursor::MoveToColumn;
use crossterm::event::{
    poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use crossterm::{execute, queue};

use crate::turnrt::belief::BeliefRecord;

const SCRUB_SCROLL_TICKS_PER_STEP: i16 = 3;
const SCRUB_DRAG_COLUMNS_PER_STEP: i16 = 8;
const INPUT_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq)]
pub enum PauseTrigger {
    Key,
    Mouse,
}

impl PauseTrigger {
    pub fn label(&self) -> &'static str {
        match self {
            PauseTrigger::Key => "key",
            PauseTrigger::Mouse => "mouse",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GearView {
    pub gear: usize,
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
    paused: bool,
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
            paused: false,
            frontier: 0,
            scroll_accumulator: 0,
            drag_accumulator: 0,
        }
    }

    pub fn set_frontier(&mut self, frontier: usize) {
        self.frontier = frontier;
        self.anchor = self.anchor.min(frontier);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }

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
            if matches!(event.kind, MouseEventKind::Down(_)) {
                self.pause_requested = Some(PauseTrigger::Mouse);
            }
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

pub fn apply_event(state: &mut InputState, event: Event) {
    match event {
        Event::Key(key) => state.on_key(key),
        Event::Mouse(mouse) => {
            let paused = state.paused;
            state.on_mouse(mouse, paused);
        }
        _ => {}
    }
}

pub fn spawn_input_thread(
    input: Arc<Mutex<InputState>>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match poll(INPUT_POLL) {
                Ok(true) => match read() {
                    Ok(event) => {
                        let mut guard = input.lock().expect("input lock");
                        apply_event(&mut guard, event);
                        if guard.quit {
                            break;
                        }
                    }
                    Err(_) => break,
                },
                Ok(false) => {}
                Err(_) => break,
            }
        }
    })
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

pub fn setup_terminal(control: &dyn TerminalControl) -> std::io::Result<()> {
    control.enable_raw_mode()?;
    if let Err(error) = control.enable_mouse_capture() {
        let _ = control.disable_raw_mode();
        return Err(error);
    }
    Ok(())
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
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        cleanup.cleanup();
        previous(info);
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
    let logit = format_logit(view.logit);
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

pub struct StreamPrinter<W: Write> {
    writer: W,
    tty: bool,
}

impl<W: Write> StreamPrinter<W> {
    pub fn new(writer: W, tty: bool) -> Self {
        Self { writer, tty }
    }

    pub fn delta(&mut self, text: &str) -> std::io::Result<()> {
        if self.tty {
            write!(self.writer, "{}", text.dimmed())?;
        } else {
            write!(self.writer, "{}", text)?;
        }
        self.writer.flush()
    }

    pub fn belief(&mut self, belief: &BeliefRecord) -> std::io::Result<()> {
        writeln!(self.writer, "\n{}", format_belief_line(belief, self.tty))?;
        self.writer.flush()
    }

    pub fn status(&mut self, staged: &str) -> std::io::Result<()> {
        if !self.tty {
            return Ok(());
        }

        queue!(self.writer, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        write!(self.writer, "» {}_ (lands at gear boundary)", staged)?;
        self.writer.flush()
    }
}

fn format_belief_line(belief: &BeliefRecord, colorize: bool) -> String {
    let diverges = belief
        .logit
        .map(|logit| (belief.said - logit).abs() > 0.25)
        .unwrap_or(false);
    let marker = match (diverges, colorize) {
        (true, true) => " ⚠".yellow().to_string(),
        (true, false) => " ⚠".to_owned(),
        (false, _) => String::new(),
    };
    format!(
        "◆ believes: \"{}\" {:.2} said · {}{}",
        belief.claim,
        belief.said,
        format_logit(belief.logit),
        marker
    )
}

fn format_logit(logit: Option<f32>) -> String {
    logit
        .map(|value| format!("{value:.2} logit"))
        .unwrap_or_else(|| "n/a logit".to_owned())
}

pub fn render_pause_card<W: Write>(
    writer: &mut W,
    reason: &str,
    belief: Option<&BeliefRecord>,
    command: &str,
) -> std::io::Result<()> {
    writeln!(writer, "\n⏸ {}", reason)?;
    match belief {
        Some(belief) => writeln!(
            writer,
            "belief: \"{}\" {:.2} said · {}",
            belief.claim,
            belief.said,
            format_logit(belief.logit)
        )?,
        None => writeln!(writer, "belief: unavailable")?,
    }
    if !command.is_empty() {
        writeln!(writer, "tool: {}", command)?;
    }
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
    use crate::turnrt::belief::BeliefRecord;

    fn belief(said: f32, logit: Option<f32>) -> BeliefRecord {
        BeliefRecord {
            claim: "did the thing".to_owned(),
            confidence: said,
            source: "tests".to_owned(),
            said,
            logit,
            raw: String::new(),
            parse_error: None,
        }
    }

    #[test]
    fn scrub_input() {
        let mut input = InputState::new();
        input.set_frontier(7);
        for _ in 0..3 {
            input.on_mouse(
                MouseEvent {
                    kind: MouseEventKind::ScrollRight,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                },
                true,
            );
        }
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
    fn input_thread_translation() {
        let mut input = InputState::new();
        input.set_frontier(5);

        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        );
        assert_eq!(
            input.pause_requested.as_ref().map(PauseTrigger::label),
            Some("key")
        );

        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE)),
        );
        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)),
        );
        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
        );
        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)),
        );
        assert_eq!(input.live_input, "ue");
        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        );
        assert_eq!(input.staged_correction.as_deref(), Some("ue"));
        assert!(input.live_input.is_empty());

        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
        );
        assert!(input.approved);

        apply_event(
            &mut input,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 3,
                row: 3,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert_eq!(
            input.pause_requested.as_ref().map(PauseTrigger::label),
            Some("mouse")
        );

        input.set_paused(true);
        for _ in 0..3 {
            apply_event(
                &mut input,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollLeft,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                }),
            );
        }
        assert_eq!(input.anchor, 0);
        input.move_anchor(4);
        for _ in 0..3 {
            apply_event(
                &mut input,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollLeft,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                }),
            );
        }
        assert_eq!(input.anchor, 3);

        apply_event(
            &mut input,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );
        assert!(input.quit);
    }

    #[test]
    fn non_tty_clean() {
        let mut buffer = Vec::new();
        {
            let mut printer = StreamPrinter::new(&mut buffer, false);
            printer.delta("thinking about it").expect("delta");
            printer.belief(&belief(0.9, Some(0.2))).expect("belief");
            printer.status("use --release").expect("status");
        }
        let mut card = Vec::new();
        render_pause_card(&mut card, "policy fired", Some(&belief(0.9, Some(0.2))), "rm -rf x")
            .expect("card");
        buffer.extend_from_slice(&card);
        assert!(
            !buffer.contains(&0x1b),
            "non-tty output must not contain ESC bytes: {:?}",
            String::from_utf8_lossy(&buffer)
        );
        assert!(String::from_utf8_lossy(&buffer).contains("⚠"));
    }

    #[test]
    fn panic_restores_terminal() {
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
                self.output.lock().expect("lock").push("mouse_off");
                Ok(())
            }
        }

        let output = Arc::new(Mutex::new(Vec::new()));
        let cleanup = Arc::new(CleanupHandle::new(Arc::new(FakeControl {
            output: output.clone(),
        })));
        let previous = std::panic::take_hook();
        install_panic_cleanup(cleanup);
        let result = std::panic::catch_unwind(|| {
            panic!("guarded section blew up");
        });
        std::panic::set_hook(previous);
        assert!(result.is_err());
        let values = output.lock().expect("lock").clone();
        assert_eq!(values, vec!["mouse_off", "raw_off"]);
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
