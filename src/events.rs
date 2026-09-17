//! Structured mint.run events for stdout collection.
//! Used by: execution engine.

use std::time::Duration;

use crate::domain::ActionStatus;

#[allow(clippy::too_many_arguments)]
pub fn transition(
    action_id: uuid::Uuid,
    tenant_id: &str,
    from: ActionStatus,
    to: ActionStatus,
    provider: &str,
    operation: &str,
    latency: Duration,
    reconciliation: bool,
) {
    tracing::info!(
        action_id = %action_id,
        tenant_id,
        state_transition = %format!("{}->{}", from.as_str(), to.as_str()),
        provider,
        operation,
        latency_us = latency.as_micros() as u64,
        outcome = to.as_str(),
        reconciliation,
        "mint.action.transition"
    );
}
