# File-first product delivery

Status: **ACTIVE — IMPLEMENTATION**

Authority: [ADR-0016](../DECISIONS/0016-file-first-four-core-jobs.md) and the Owner's 2026-11-02 M0-B/C → M1 → M2 instruction. This short record satisfies the existing project-state contract; it adds no review or management system.

- M0-A is merged in #534; M0-B/C merged in #535 as `6392e66` with required CI success. M0-B removes automatic legacy Managed Scan submission from app startup; UI dormancy already uses the browser-only compatibility fixture. Preserve data, roots, settings, Query V2, Saved Views, tags, APIs and shared services. Review dormancy on 2027-01-04.
- M0-C archives contradictory STATUS history, aligns M0–M5 and makes startup reading AGENTS → STATUS → ROADMAP → corresponding Issue/code.
- M1 code is complete: #536 is merged; final workflow closeout fixes partial/recovery result feedback and exercises cancel/reissue without a new P0/P1 code defect. M2 completes one-off folder → metadata AI → grouped review/edit → batch execution → History/Restore, reusing existing authorities without managed-root admission. Direct files only, subfolders stay put, up to 100 entries; M3 proceeds on existing cleanup findings/Safe Trash.
- Run focused checks and existing routed CI. File authorization, mutation, Journal/Restore and branch protection stay binding. Owner has authorized safe merge after required checks; no additional Owner gate or Release.
- #512 tracks features; #487 owns unverified native GUI; independent Search and historical investigations keep their existing owners. Browser tests are not native qualification.

Changing delivery state belongs only in [STATUS](../STATUS.md) / [ROADMAP](../ROADMAP.md).
