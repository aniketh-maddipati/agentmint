# Notes on the Task API

I built a small demo where an agent can only refund a Stripe charge if a Parallel Task result backs the claim. The rule is `disruption == true`, confidence medium or high, and 2+ citation domains. I made 4 `base` runs with `parallel-web` 1.3.5.

## What worked

- Create + blocking result worked first try:

  ```python
  run = client.task_run.create(input=q, task_spec={"output_schema": SCHEMA}, processor="base")
  res = client.task_run.result(run.run_id, api_timeout=1800)
  ```

- With a JSON schema, `output.content` comes back as a dict. Every field gets a `basis` entry I can gate on directly.
- The negative case was sharp. For FedEx / Phoenix / 2024-05-14 it found a test-environment API notice and answered `false` with a citation.
- Latency was 65 to 110s.

## Docs vs. what I got

- The Research Basis guide doesn't list `citation.title`, `mcp_tool_calls`, `beta_fields` or `output_schema`. They're all in the response, and the API reference has them.
- The HTTP param is `timeout`, but the SDK calls it `api_timeout`.

## Confidence and citations

- I got `medium` on all 4 runs, with 1 source or 2.
- The same Dallas query gave 2 citation domains, then 1 on the rerun. My policy flipped from pass to refuse.
- The two citations can back different parts of the claim:

  ```
  supplychaindive.com  -> FedEx suspended service in Florida
  nytimes.com          -> Ian made landfall near Fort Myers (no FedEx mention)
  ```

- One `reasoning` string cited "Reuters reporting" that wasn't in `citations`.

## Asks

1. A stable ID or hash for a result's basis, so I can sign exactly what I relied on:

   ```json
   {"basis_id": "bas_...", "basis_sha256": "..."}
   ```

2. Per citation, which claim it supports. Or flag sources named in `reasoning` that aren't cited.
3. Say what drives `confidence`, or expose the source count and agreement behind it.
