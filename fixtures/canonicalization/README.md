//! Shared RFC 8785 canonicalization fixtures for Rust and TypeScript.
//!
//! Material fields included in the hash:
//!   version, tenant_id, actor.subject, actor.agent_id, actor.delegated_by,
//!   actor.issuer, provider, operation, resource.resource_type,
//!   resource.resource_id, arguments, context.support_ticket_id, context.reason
//!
//! Non-material operational fields excluded:
//!   action_id, created_at, expires_at
//!
//! Provider idempotency is derived internally as mint:{action_id}:{operation}:v1.
