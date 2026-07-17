# AERF v0.2 conformance vectors

This directory holds the 12 conformance vectors referenced by
`make verify-vectors`. Each subdirectory is one vector; each contains
the receipt artifacts, the public keys needed to verify them, and an
`expected.json` describing the outcome a conformant verifier MUST
produce.

`manifest.json` enumerates the set so `tools/run-vectors.py` can
dispatch them. Regenerate the directory deterministically with:

    python tools/build-vectors.py

The keys are derived from fixed seeds in the builder; the artifacts
are byte-stable across re-runs.

## Outcome vocabulary

| Outcome      | Meaning |
|--------------|---------|
| `PASS`       | Verifier exits 0; all applicable checks pass. |
| `FAIL`       | Verifier exits 1; specific `reason_code` MUST appear on stderr. |
| `KNOWN_LIMIT`| Verifier exits 0 because the gap is a documented residual that the receipt layer cannot close. The runner reports it as expected behavior alongside an explanatory note (SPEC.md §12.6 and §12.7). |
