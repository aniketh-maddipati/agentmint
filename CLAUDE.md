# AgentMint Development Standards

## Standards
- No .unwrap() - use ? operator
- No inline comments - code is self-explanatory
- No duplication - extract shared logic
- One function does one thing
- Early returns, no deep nesting
- Names tell the story

## Rust Best Practices
- Borrow (&) by default, clone only when necessary
- Use &str for function params, String for struct fields
- Arc for shared state across threads
- Mutex only around the data that needs it
- Prefer iterators over collect() unless needed

## Testability
- All business logic in pure functions
- Inject dependencies, no hardcoded values
- Each module gets #[cfg(test)] mod tests at bottom

## Latency Measurement
- Use std::time::Instant for timing
- In proxy handler: measure verify_token + jti_check + audit_log separately
- Print to tracing: "verify: {}μs | jti: {}μs | audit: {}μs | total: {}μs"
- Return X-Verify-Time-Us header in response

## File Header Only
//! What this does.
//! Used by: dependents.

## Build Order

1. src/error.rs (40 lines) + tests
2. src/token/claims.rs (20 lines) + tests
3. src/token/sign.rs (30 lines)
4. src/token/verify.rs (45 lines) + tests
5. src/token/mod.rs (5 lines)
6. src/jti/memory.rs (35 lines) + tests
7. src/jti/mod.rs (5 lines)
8. src/audit/sqlite.rs (55 lines) + tests
9. src/audit/mod.rs (5 lines)
10. src/state.rs (30 lines)
11. src/handlers/health.rs (8 lines)
12. src/handlers/mint.rs (45 lines)
13. src/handlers/proxy.rs (70 lines) - include timing
14. src/handlers/audit.rs (15 lines)
15. src/handlers/mod.rs (8 lines)
16. src/server.rs (25 lines)
17. src/main.rs (20 lines)

After each module: cargo build && cargo test
