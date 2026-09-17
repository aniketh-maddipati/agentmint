//! Synthetic medical-benefit PA/BV lab (experimental).
//! Used by: `mint lab` CLI. Isolated from refund/mint runtime APIs.

pub mod agent;
pub mod cli;
pub mod clock;
pub mod console;
pub mod domain;
pub mod error;
pub mod inspect;
pub mod payer;
pub mod scenarios;
pub mod store;
pub mod workflow;

pub use domain::WORKFLOW_VERSION;
pub use error::{LabError, LabResult};
pub use workflow::LabEngine;
