# PA/BV Domain Notes — Medical Benefit MRI Prior Authorization

**Workflow version:** `medical-mri-pa-v1`  
**Benefit type:** Medical benefit (items/services), **not** pharmacy/prescription-drug PA.  
**Requested service (synthetic):** Outpatient MRI lumbar spine without contrast (CPT **72148**).  
**Review date:** 2026-09-17.

## Glossary

| Term | Meaning in this lab |
| ---- | ------------------- |
| Benefits verification (BV) | Gathering eligibility, plan activity, network status, service coverage posture, and whether prior authorization is required for the requested service. BV is **not** a payment guarantee. |
| Eligibility | Member is active on a plan for a given date of service. Does **not** establish coverage for a specific service. |
| Coverage | Plan rules for whether a service is covered (may still require PA). |
| Prior authorization (PA) | Payer approval required **before** furnishing certain medical items/services. Request + decision are distinct. |
| CRD (Coverage Requirements Discovery) | FHIR workflow (Da Vinci) used to discover whether PA is required and what documentation is needed. A CRD check alone does **not** start the PA decision clock. |
| DTR | Documentation Templates and Rules — collecting required clinical documentation. |
| PAS | Prior Authorization Support — electronic submission and status of a PA request. |
| Intake rejection | Administrative rejection of a submission (wrong member, missing fields). Not a coverage denial. |
| Denial | Substantive payer decision refusing authorization for the requested service (may include reason codes). |
| Appeal | Human-initiated second-round submission linked to an original denial. |
| Packet | Immutable manifest of exact document versions and content hashes bound for review/submission. |
| Review / signature | Operator/physician review of an **exact** packet version. Not an API grant of authorization. |
| Receipt-confirmed | External system acknowledged receipt of a submission. Not approval. |
| Handoff | Delivery of an outcome (e.g., approval with limitations) to a customer/downstream owner. Acknowledgment ≠ patient contact or service delivery. |

## Workflow assumptions

1. Single specialty path: outpatient diagnostic imaging PA for lumbar MRI.
2. Synthetic payer and documents only; no clearinghouse, EHR, or real payer APIs.
3. Operator roles in the lab: `customer`, `payer`, `reviewer`, `operator` (lab-only role switch).
4. BV may conclude: `pa_required`, `pa_not_required`, `inactive_member`, `not_covered`, `unclear`, or `conflict`.
5. `pa_not_required` ends in customer handoff stating authorization is not required — **not** a payment guarantee.
6. One complete appeal round after a human decides to appeal; further denial → manual disposition.
7. Historical BV results are display context only; they are never auto-applied as current coverage.

## Source links (primary / standards-adjacent)

| Source | Use | Reviewed |
| ------ | --- | -------- |
| [CMS-0057-F final rule (Interoperability & Prior Authorization)](https://www.cms.gov/files/document/cms-0057-f.pdf) | Defines PA as request + decision for medical items/services; drugs excluded from these PA API provisions. | 2026-09-17 |
| [CMS Prior Authorization API FAQ](https://www.cms.gov/priorities/burden-reduction/overview/interoperability/frequently-asked-questions/prior-authorization-api) | CRD discovers PA requirements; CRD alone does not start the decision clock; decisions may approve, deny with reason, or request more information. | 2026-09-17 |
| Da Vinci CRD / DTR / PAS IGs (HL7 FHIR) | Terminology for discovery, documentation, and submission (lab models concepts, does not implement FHIR wire formats). | 2026-09-17 (secondary summaries) |

## Facts supported by sources

- Medical-benefit PA (items/services) is distinct from prescription-drug PA under CMS-0057-F PA API scope.
- PA has two parts: provider request and payer decision.
- Coverage-requirements discovery informs whether PA is required and what documentation is needed; it is not itself a PA submission.
- Payer decisions include approval (possibly with limits), denial with reason, pending, or request for more information.
- Decision timeframes in regulation are for impacted payers after a request is received — this lab uses synthetic clocks only.

## Synthetic scenario rules (lab-only; not clinical guidance)

- Fixture CPT `72148` + diagnosis `M54.5` (low back pain) for outpatient imaging.
- Scripted BV answers and document sets are scenario-controlled.
- Fake payer ledger is independent of case storage; application code cannot open it.
- Approval may carry synthetic limitations (e.g., site of service outpatient only; auth valid N days).
- Ambiguous phrases such as “may require authorization” must remain `unclear` until clarified or reviewed.

## Customer / payer-specific decisions requiring operator validation

- Whether a particular commercial plan requires PA for MRI lumbar spine.
- Exact documentation checklists and medical-necessity criteria.
- Network / site-of-service rules.
- Appeal filing windows and forms.
- Whether “PA not required” still needs notice to scheduling/billing.

## Known exclusions

- Pharmacy / specialty-drug PA.
- Real X12 278, FHIR PAS wire, CRD/DTR runtime.
- Telephony, fax OCR, EHR write-back.
- Autonomous clinical justification or appeal-letter generation.
- Production identity, payments, or live payer credentials.
- Multi-specialty universal rule engines.
- Claiming universal exactly-once external effects.
