# Zen Canvas Project Operating System

`docs/project/` is the current-truth and long-horizon planning layer for Zen Canvas. It answers what the product is, which architecture is authoritative, what is active now, what is planned next, and how work is allowed to move from idea to production.

This directory does not replace production code, security contracts, historical taskbooks, or closeout evidence. It provides the stable index that tells contributors which of those sources are current and how they relate.

## Default task context

For an ordinary bounded task, use this sequence:

```text
AGENTS.md
→ docs/project/STATUS.md
→ the current task brief linked from STATUS
→ inspect relevant production owners / callers / tests
```

This is a routing rule, not a recursive reading list. Read linked documents
only when the task touches their domain or an ambiguity requires them.

## Conditional reads

| Trigger | Read when triggered |
| --- | --- |
| product ownership or navigation | [`PRODUCT_MAP.md`](PRODUCT_MAP.md) |
| state, persistence or durable authority | relevant [`ARCHITECTURE_MAP.md`](ARCHITECTURE_MAP.md) section; an ADR when ambiguous |
| filesystem mutation, Restore or Safe Trash | relevant security, identity and recovery contracts |
| Tauri command or capability permissions | `docs/security/TAURI_COMMAND_PERMISSION_MATRIX.md` and affected permission authorities |
| macOS mutation | the macOS mutation threat model |
| new initiative, cross-wave scope or product direction | [`MASTER_DEVELOPMENT_PLAN.md`](MASTER_DEVELOPMENT_PLAN.md) plus [`ROADMAP.md`](ROADMAP.md) |
| large responsibility expansion | [`CODE_MAINTAINABILITY.md`](CODE_MAINTAINABILITY.md) |
| release or publication | release contracts and [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md) |
| governance/process change | [`DEVELOPMENT_WORKFLOW.md`](DEVELOPMENT_WORKFLOW.md) (start with the [delivery-first operating rule](DEVELOPMENT_WORKFLOW.md#delivery-first-operating-rule)) and [ADR-0015](DECISIONS/0015-delivery-first-engineering.md) |
| named security, remediation, design or QA contract | that contract |

For rationale/history rather than current authorization, use `docs/project/research/`. In particular, [`research/file-library-preview/`](research/file-library-preview/) preserves the external-project research and reconstructed Round 1–4 synthesis behind the File Library 2.0 / Preview Platform architecture.

`AGENTS.md` is the stable repository entry point. It must point here instead of embedding a changing project stage or baseline.

A lower-level initiative, taskbook or PR may narrow this plan but must not silently expand or contradict the Master Development Plan. If implementation appears to require a cross-Wave feature, new durable authority, schema change or safety-boundary rewrite, stop and escalate through architecture/governance review.

## Source-of-truth precedence

Separate normative safety authority from descriptive implementation truth. Normative authority defines what the product is allowed to do; implementation truth describes what the repository currently does.

### Normative authority — what is allowed

The following define the permitted safety and privacy boundary:

- security and privacy contracts;
- filesystem mutation safety, recovery and restore contracts;
- command and window permission boundaries;
- supported-platform safety contracts;
- accepted durable safety invariants.

These contracts remain authoritative even if current code violates them. A safety bug in production code must not be used to weaken a normative constraint.

### Descriptive implementation truth — what exists now

The following describe the current implementation and must be read as evidence of behavior, not permission to violate normative authority:

- current production code;
- executable tests;
- the actual database schema.

If normative safety authority and descriptive implementation truth disagree, treat the mismatch as an implementation defect or governance conflict. Do not use the current code to override the safety constraint; stop, report the conflict and fix or explicitly resolve it before proceeding.

### Project and historical context

Use the current task brief and `STATUS.md` as the bounded context. Load the
Master Plan, Roadmap, initiative record, architecture/product maps,
maintainability guidance or domain contracts only through the conditional
routing above. Research, historical taskbooks, audits, QA closeouts, archived
prompts and old PR records are evidence, not ordinary startup context.

Historical/research documents remain evidence. They do not become current execution authority merely because they contain a newer-looking date, branch name or implementation checklist.

## Project-state vocabulary

Use these terms precisely:

- **Implemented** — code or documentation exists on the stated commit.
- **Validated** — the stated checks passed on the exact stated commit.
- **Packaged** — an installer/package build succeeded on the stated commit.
- **Released** — a user-facing release/tag was actually published.

Do not describe validated or packaged work as released.

## Document ownership

- `MASTER_DEVELOPMENT_PLAN.md` changes only when the long-horizon product/architecture direction, Wave boundaries, platform strategy or explicit non-goals genuinely change. It is not a progress log.
- `STATUS.md` changes whenever the active initiative, baseline, validation state, schema, package version or release state changes.
- `PRODUCT_MAP.md` changes when product ownership or workspace boundaries change.
- `ARCHITECTURE_MAP.md` changes when durable authority, runtime ownership, persistence, platform ownership or compatibility boundaries change.
- `ROADMAP.md` changes when sequencing or authorization changes.
- `CODE_MAINTAINABILITY.md` owns repository-wide implementation cohesion, module-responsibility and decomposition rules; taskbooks may add stricter domain-specific boundaries but may not silently weaken it.
- `TECH_DEBT.md` tracks debt with explicit exit conditions rather than vague cleanup intentions.
- `RISK_REGISTER.md` tracks currently open project-level risks; historical remediation risk registers remain domain evidence.
- `DEVELOPMENT_WORKFLOW.md` changes only when the engineering process itself changes.
- `DECISIONS/` stores accepted cross-cutting architecture/governance decisions.
- `initiatives/` stores bounded current and planned initiatives. Completed initiatives remain as concise records, not as the current status source.
- `tasks/` stores bounded execution instructions and cannot silently expand higher-level authorization.
- `research/` stores evidence/rationale from external projects, platform investigations and design research. Research can explain a decision but cannot authorize implementation by itself.

## Rule against duplicate current truth

Do not create another file whose purpose is “current project status”, “active phase”, “current baseline” or “what to do next”. Update `STATUS.md` or `ROADMAP.md` instead.

`MASTER_DEVELOPMENT_PLAN.md` is intentionally not another current-status file: it records the stable long-horizon conclusions and constraints that explain why the current Roadmap/initiatives are shaped as they are.

Domain documents may describe their own state, but must not silently override the project-level state recorded here.

## Current execution precedence

For current project progress and sequencing, STATUS.md and ROADMAP.md are the
only project-level sources. The active initiative owns durable scope and
authority; the one current bounded taskbook owns execution detail for its Track.
Historical taskbooks, addenda, PR descriptions and closeout records are evidence
only. When a taskbook clarification becomes current, fold it into the current
taskbook and remove the obsolete override instead of adding another precedence
layer.
