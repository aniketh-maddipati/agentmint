# Lab JSON Schema (source of truth: Rust types)

Generated JSON Schema 2020-12 and OpenAPI 3.1 for the PA/BV lab contract.

Do not edit these files by hand. Refresh with:

```bash
cargo test -q lab::schema::write_committed_schemas -- --ignored
```

Fixture-only guided-mode fields must never appear in these schemas. REST serving of `openapi.json` is a later PR.
