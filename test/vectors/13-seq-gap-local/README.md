# 13-seq-gap-local

This fixture is project-local only and is **not** an official AERF conformance
vector.

It mirrors the three-receipt pattern from `02-chain-happy-path`, but adds
signed `seq` fields so the chain verifier can exercise deletion detection with
sequence continuity in play.

The test verifies two things:

1. The full three-receipt chain is valid end to end.
2. Verifying only receipt `00` and receipt `02` fails explicitly at the current
   verifier ordering. Today that means `hash_link_mismatch`, because the link
   check fires before the seq check on a middle-receipt deletion.
