# D4-S6 Cursor Scope Contract

## Global File Library

A global cursor is bound to the current active Manifest ID/revision, the complete effective Root/Source Seal vector, its normalized FTS expression/filter, stable sort definition, and current ranked result digest. Any root or source authority change that changes the effective global collection rejects the cursor. A Watcher change to root A rejects the old global token even if root B remains valid.

## Root-filtered query

A cursor limited to root B may exclude the global pointer revision only when both the SQL query and token use the same root-B scope:

- Search joins the current `d4s5_effective_root_members` relation and filters `root_id = 'root-b'` before returning rows.
- Token includes B's selected generation, Root Seal/current Root Authority revision, source/path identities, Source Seal/current Source Authority revisions, the normalized FTS expression, and the exact visible ranked FTS rows.
- The ranked rows include `bm25(files_fts)` values. Since BM25 uses the whole SQLite FTS corpus, any corpus mutation that changes B's result scores or ordering changes the token and rejects the cursor, even if the mutation came from another root.
- Every page re-applies the current visibility relation. Cursor validation never replays a captured old FTS or Manifest view.

In the synthetic overlap test, root A's Watcher revision changed while root B retained the same shared source and companion. Root B's visible FTS row and BM25 score were byte-for-byte stable, so the root-B cursor remained valid. The global cursor was invalidated. After root A published a new generation, the global pointer changed but the root-B generation and source seals and ranked result remained identical, so the root-B cursor remained valid. When the shared source identity was invalidated, the root-B query removed that source and the old root-B cursor was rejected.

This is a correctness contract, not a performance claim. Search-index generation, FTS mutation concurrency, and SQLite 3.51.3 BM25 behavior still require separate product validation.
