//! Shared RFC 8785 canonicalization fixtures for Rust and TypeScript.
//!
//! Material fields included in the hash:
//!   version, tenant_id, actor.subject, actor.agent_id, actor.delegated_by,
//!   actor.issuer, provider, operation, resource.resource_type,
//!   resource.resource_id, arguments, context.support_ticket_id, context.reason
//!
//! Non-material operational fields excluded:
//!   action_id, idempotency_key, created_at, expires_at
//!
//! Canonicalization version: jcs-rfc8785-v1
//! Hash: lowercase hex SHA-256 of the RFC 8785 UTF-8 bytes, prefixed with `sha256:`.
