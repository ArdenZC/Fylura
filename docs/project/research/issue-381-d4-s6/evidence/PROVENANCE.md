# D4-S6 Evidence Provenance

- Frozen parent: D4-S5 HEAD `59e71da8e37dc7b1f1828b71af06596d178e5611`, Tree `9439b7264c34130f5716e482a35fffc0b225ce01`.
- Research branch: `research/issue-381-d4-s6`.
- Reproduction command from repository root: `python3 -B docs/project/research/issue-381-d4-s6/harness/run.py`.
- Captured stdout command: `python3 -B docs/project/research/issue-381-d4-s6/harness/run.py > docs/project/research/issue-381-d4-s6/evidence/publication-fence-run.log`.
- The captured stdout is byte-identical to `publication-fence-results.json`; the comparison succeeded before this file was written.
- Runtime: Python `sqlite3`, SQLite `3.53.1`, SQLite source ID and Schema `user_version=37` are recorded in the raw JSON.
- `INPUTS.json` records the frozen parent plus SHA-256 of the schema and the exact D4-S2 through D4-S6 harness inputs. `SHA256SUMS` records the review documents, SQL, harness, input manifest, raw structured output, and raw stdout.
- The initial failure cases run against D4-S5's unchanged SQL without installing D4-S6 SQL. The partial-Seal case deliberately makes the S4 preflight helper return one of two source evidence rows, records the actual old commit, then restores the original helper. The repaired cases install only the new D4-S6 research SQL.
- The event trace is written by research-only SQLite triggers inside the publication transaction. Successful traces show Root/Source Seal insert events before pointer CAS. Failed transaction trace writes are rolled back.
- No historical log was reconstructed. The run log in this directory is the raw stdout from the D4-S6 command above.
