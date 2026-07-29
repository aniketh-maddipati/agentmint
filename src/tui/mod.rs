//! Pure, render-independent interaction core for the terminal agent-run debugger.
//! Used by: future Ratatui rendering, terminal I/O, and executor-integration phases.

pub mod action;
pub mod input;
pub mod model;

pub use action::{reduce, Action, Effect, Transition};
pub use input::translate;
pub use model::{
    AppState, DisplayMode, FocusRegion, ForkState, OperationId, PreviewState, RunLifecycle,
    Viewport,
};
