# #381 D4-S8R — Traceability-only correction

This package corrects the production/test boundary in the D4-S8 domain map without changing any frozen S8 file or source count.

- `PRODUCTION-DOMAIN-ENTRY-MAP.csv`: the same 17 domains, with production/schema stale, write and direct `files`-read IDs only; all 209 registered command IDs remain inventoried; the FE-011 browser mock is excluded.
- `REGRESSION-ENTRY-MAP.csv`: every stale/write/read test or fixture ID from the frozen ledgers plus the FE-011 test/demo consumer.
- `TAURI-COMMAND-OWNERSHIP.csv`: all 209 command IDs and original registration/definition locations, domain memberships, ownership and visibility boundary.
- `D16-ONLY-COMMAND-TRACEABILITY.csv`: the exact 40 commands mapped only to D-16 in frozen S8, classified by source module and registry metadata.
- `evidence/ID-SET-PROOF.json`: exact source and map ID arrays, counts, missing/extra checks and deterministic set digests.
- `tools/`: deterministic ledger projection and independent static validator.
- `evidence/INPUTS.json`, `evidence/SHA256SUMS`: frozen inputs and checksums of all S8R files other than the checksum manifest itself.

D-16 is an IPC registration/transport surface, not a blanket D4 implementation owner. Consult the reconciliation report for the 40 handler dispositions and limits. D4 production remains NO-GO; #380 database-generation isolation is still a prerequisite.
