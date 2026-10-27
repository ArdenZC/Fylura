# U4 File Library: Inspector, user notes and file favorites

Issue [#482](https://github.com/ArdenZC/Zen-Canvas/issues/482). Base:
`b3ace0cda44d0e9db8e67448af88fb056d34232d`, tree
`0654351258c9c01529df1b6f234304b02d5cf7b0`. Latest base normal CI
[40461380013](https://github.com/ArdenZC/Zen-Canvas/actions/runs/40461380013)
passed. U3 postmerge run 40454962565 failed the W2-11 integrated browser
performance gate; its dependent performance/platform summary jobs also failed.
Later master CI passed; U4 does not weaken the existing gate.

## User outcome and scope

Existing pane-local Inspector puts name, type, size, times, location and
availability before user notes and managed classification. Classification and
Content Understanding are collapsible. Notes support explicit Save, Discard
and Delete; favorites are file-level, separate from U1 folder Places. The
Notes and file favorites entry in Library returns to canonical managed IDs,
including unavailable records, without resetting query, sorting or other panes.
Browse displays an explicit explanation that durable notes/favorites require
verified managed identity and browsing does not enroll files in the index.

Inspector selected-item preview and Previous/Next reuse the existing Preview
controller, Floating/Pinned hosts, provider registry and Read Gate. The bounded
selection neighborhood does not change selection, ordering, tabs or pane focus.
All-matching previews are explicitly limited to selected loaded items. Existing
selection summaries retain backend totals; Browse unknown/partial size and
unknown availability remain explicit. Compact Inspector uses the existing
SideSheet; Escape and existing modal focus return remain in effect. English and
Chinese strings use the existing dictionary, with shared light/dark tokens.

## Persistence contract and compatibility

SQLite remains the durable authority. No DDL, schema bump, migration, backfill,
new index, scheduler, scan enrollment, AI call or filesystem mutation is added.
The existing `app_settings` table stores one versioned JSON value per canonical
SourceUID under `file_personal_metadata:v1:<SourceUID>`. This is a private
metadata namespace, never U1 Places or the typed application settings document.
The value has exactly `version: 1`, `note` (at most 16,000 Unicode scalars),
`favorite` and positive `revision`. Unknown fields/versions and invalid records
fail closed. Notes are plain text, never executed or interpreted as markup.

The Main-only command accepts a managed file ID and resolves its latest legacy
attribution through existing identity authority. Writes require a non-stale
record and a complete live physical identity matching that SourceUID. An
IMMEDIATE SQLite transaction serializes identity resolution and revision CAS;
reads use deferred transactions. Conflicting edits preserve the draft, disable
blind retries and offer Reload. A reused live path cannot read or overwrite the
old physical object's note. Unverified/conflicted IDs and disposable Browse refs
cannot write. Missing IDs fail explicitly; stale attributed records retain
read-only metadata. A failed live identity check hides saved content and asks
for reload instead of presenting a prior object's note as current.

SourceUID keys naturally survive supported canonical ID/path updates from
Rename, Move and Restore. Scan's existing attached-facts preservation also
recognizes this namespace, so a neutral row with personal facts cannot be
recycled onto a different object. The backend list joins the latest verified
attribution back to current managed IDs, uses deterministic name/ID ordering and
returns 40 entries plus a `hasMore` lookahead. It never lists Browse refs.
There is no polling, global renderer cache or idle preview demand.

Older builds ignore the namespace and continue to read Schema44. Rolling back
application code needs no database downgrade and must preserve these values.
Deleting a note and unfavoriting clears user content but retains a small
revision tombstone, preventing ABA stale writes. Orphaned SourceUID records stay
inert and cannot appear without a matching managed attribution; they are not
reassigned by path. Explicit database maintenance may remove only keys in this
namespace once their lineage has been deliberately retired and concurrent
writers stopped. No automatic cleanup can erase recovery lineage or reset
revisions while an editor is open. This bounded compatibility policy avoids a
new schema conflicting with the independent Schema44 / Global Search work.

The new command is registered in Rust/build input, Main capability, permission
matrix, frontend facade, browser mock and permission contract tests. Search
Window receives no new capability. Global Search production code, benchmark
and roadmap are unchanged.

## Validation and evidence

Focused frontend tests cover CRUD/favorite isolation, CAS failure and late
responses after switching files. Backend tests cover SQLite reopen, CAS/ABA,
size bounds, canonical operation-finalizer rename, stale/read-only recovery,
physical replacement, missing/Browse IDs and conflicted attribution. Existing
normal PR CI remains required. `npm run test:browser:u4` runs real Chromium with
deterministic browser mocks; screenshots and checks are stored with this task.
These mocks simulate metadata durability in browser storage only and are not
native SQLite or filesystem acceptance.

Windows/macOS native GUI, actual filesystem Restore and Owner acceptance remain
UNVERIFIED. No installer, release, tag, Full Validation or million-file campaign
is requested or qualified. Exact candidate HEAD/tree, diff, final required CI
and readiness verdict belong in the single Draft PR description.
