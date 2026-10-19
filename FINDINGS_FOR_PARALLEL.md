# Task API findings (base processor, 4 runs, Python SDK parallel-web 1.3.5)

## Worked well
- `task_run.create` + `task_run.result(run_id, api_timeout=1800)` worked first time. Latency was 65–110 s.
- With a JSON `output_schema`, `output.content` came back as a parsed dict. Every top-level field had a `basis` entry with `reasoning`, `confidence` and `citations[].url/title/excerpts`.
- The negative case was precise. For FedEx/Phoenix/2024-05-14 it found a May 14 *test-environment* API notice and a May 18–19 production interruption, and answered `false`.

## Docs vs. actual
- The Research Basis guide's tables leave out `citation.title`, `output.mcp_tool_calls`, `output.beta_fields` and `output.output_schema`, which were all present (null where not applicable). The API reference does document them.
- The HTTP query parameter is `timeout`; the SDK calls it `api_timeout`.
- `output.output_schema` is null for a user-supplied schema. This is documented ("auto schema only") but surprising.

## Confidence and citations
- The "no disruption" answer was still cited (developer.fedex.com).
- Confidence was `medium` on all 4 runs and did not track the evidence. The same Dallas query returned 2 citation domains once and 1 the next time, both `medium`.
- Citations can back different sub-claims. In the Hurricane Ian run, one citation showed FedEx suspensions while the other (NYT) only placed the storm at Fort Myers. One reasoning string cited "Reuters reporting" that was not in `citations`.

## Suggestions
1. Return a stable `basis_id` or hash for each result's basis (and for each `FieldBasis`), so downstream approvals and receipts can reference exactly what was relied on.
2. Mark per citation which claim it supports (e.g. `supports: "carrier disruption" | "location/date"`), or flag sources named in `reasoning` that are absent from `citations`.
3. Document what drives `confidence`, and consider exposing the counts behind it (independent sources, agreement), since `medium` covered both 1-source and 2-source evidence.
