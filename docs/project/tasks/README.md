# Project task artifacts

This directory contains current and historical task briefs, activations,
results and closeout evidence. Do not infer the active task from filenames or
directory browsing.

`docs/project/STATUS.md` owns the current task link and current execution
truth. A historical taskbook or result is evidence/reference only unless
STATUS or the current task explicitly points to it.

Future bounded task briefs should be context compressors with these sections:

- Goal
- Scope
- Do not change
- Relevant invariants
- Start here — symbols, files, callers and tests
- Read only if triggered
- Acceptance
- Validation

For an ordinary reversible feature or bug fix, keep this brief to the minimum
needed to implement and verify the user outcome. A research phase, standalone
audit, hand-built hash manifest or bespoke native environment is **not** a
mandatory taskbook section. See [delivery-first workflow](../DEVELOPMENT_WORKFLOW.md#delivery-first-operating-rule)
and [ADR-0015](../DECISIONS/0015-delivery-first-engineering.md).
