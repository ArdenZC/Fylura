use super::*;
use crate::db::Database;
use crate::global_index::filename_search::RankedFileCandidate;
use crate::global_index::models::{
    GlobalEntryInput, GlobalVolume, INDEX_STATUS_READY, PROVIDER_WINDOWS_MFT_USN,
};
use rusqlite::Connection;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_path() -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "fylura-search-source-authorization-{}-{id}.db",
        std::process::id()
    ))
}

fn source(id: &str, mount: &str, enabled: bool, authorization: &str) -> GlobalVolume {
    let mut volume = crate::global_index::tests::test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("stable-{id}");
    volume.mount_path = mount.to_string();
    volume.enabled = enabled;
    volume.scope_authorization_state = authorization.to_string();
    volume
}

fn entry(
    volume_id: &str,
    mount: &str,
    name: &str,
    extension: &str,
    relative_path: &str,
) -> GlobalEntryInput {
    let path = format!("{mount}\\{relative_path}");
    GlobalEntryInput {
        volume_id: volume_id.to_string(),
        platform_file_id: format!("test:{volume_id}:{relative_path}"),
        parent_platform_file_id: format!("parent:{volume_id}"),
        name: name.to_string(),
        path,
        extension: extension.to_string(),
        is_directory: false,
        size: 10,
        created_at_fs: Some(1),
        modified_at_fs: Some(1),
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: PROVIDER_WINDOWS_MFT_USN.to_string(),
        last_seen_at: 1,
    }
}

fn authorization_cases(
    volume_id: &str,
    mount: &str,
) -> Vec<(&'static str, &'static str, GlobalEntryInput)> {
    vec![
        (
            "exact_name",
            "exactauthguard",
            entry(
                volume_id,
                mount,
                "exactauthguard",
                "txt",
                "exactauthguard.txt",
            ),
        ),
        (
            "name_prefix",
            "prefixauthguard",
            entry(
                volume_id,
                mount,
                "prefixauthguard-tail.txt",
                "txt",
                "prefixauthguard-tail.txt",
            ),
        ),
        (
            "exact_extension",
            "extauthguard",
            entry(
                volume_id,
                mount,
                "file.extauthguard",
                "extauthguard",
                "file.extauthguard",
            ),
        ),
        (
            "extension_prefix",
            "extprefixauth",
            entry(
                volume_id,
                mount,
                "file.extprefixauthguardplus",
                "extprefixauthguardplus",
                "file.extprefixauthguardplus",
            ),
        ),
        (
            "fts",
            "authftsneedle",
            entry(
                volume_id,
                mount,
                "beforeauthftsneedleafter.txt",
                "txt",
                "beforeauthftsneedleafter.txt",
            ),
        ),
        (
            "punctuation_prefix",
            "punctauth-guard",
            entry(
                volume_id,
                mount,
                "punctauth-guard-file.txt",
                "txt",
                "punctauth-guard-file.txt",
            ),
        ),
    ]
}

fn insert_source_fixtures(db: &Database, source_id: &str, mount: &str) -> Vec<GlobalEntryInput> {
    let mut entries = authorization_cases(source_id, mount)
        .into_iter()
        .map(|(_, _, entry)| entry)
        .collect::<Vec<_>>();
    entries.push(entry(
        source_id,
        mount,
        "beforecandidateguardafter.txt",
        "txt",
        "beforecandidateguardafter.txt",
    ));
    entries.push(entry(
        source_id,
        mount,
        "path-only.txt",
        "txt",
        "pathguard\\path-only.txt",
    ));
    db.upsert_global_entries_batch(&entries)
        .expect("insert indexed rows for each source authorization state");
    entries
}

fn rowid_for(connection: &Connection, id: &str) -> i64 {
    connection
        .query_row(
            "SELECT rowid FROM global_entries WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .expect("find fixture rowid")
}

fn ids(results: &[GlobalSearchResult]) -> Vec<String> {
    results.iter().map(|result| result.id.clone()).collect()
}

#[test]
fn sqlite_search_tiers_and_filename_candidate_paths_enforce_windows_authorization() {
    let path = test_path();
    let db = Database::open(&path).expect("open isolated search authorization database");
    let authorized = source("gv_authorized", r"C:\Authorized", true, "authorized");
    let disabled = source("gv_disabled", r"R:\Disabled", false, "not_authorized");
    let legacy = source(
        "gv_legacy",
        r"E:\Legacy",
        true,
        "legacy_confirmation_required",
    );
    for volume in [&authorized, &disabled, &legacy] {
        db.upsert_global_volume(volume)
            .expect("insert fixture source with explicit consent state");
        db.update_global_volume_state(
            &volume.id,
            INDEX_STATUS_READY,
            None,
            None,
            None,
            Some(1),
            None,
        )
        .expect("make indexed fixture source ready");
    }

    let authorized_entries = insert_source_fixtures(&db, &authorized.id, &authorized.mount_path);
    let disabled_entries = insert_source_fixtures(&db, &disabled.id, &disabled.mount_path);
    let legacy_entries = insert_source_fixtures(&db, &legacy.id, &legacy.mount_path);
    db.filename_search.enable_for_test();
    db.filename_search
        .rebuild_now_for_test()
        .expect("build compact projection from authorized sources only");

    let connection = db.conn().expect("read fixture search snapshot");
    for (tier, query, authorized_entry) in
        authorization_cases(&authorized.id, &authorized.mount_path)
    {
        let results = diagnostic_search_tier(&connection, tier, query, 20)
            .unwrap_or_else(|error| panic!("run {tier} tier: {error}"));
        assert_eq!(
            ids(&results),
            vec![authorized_entry.entry_id()],
            "{tier} tier must return the authorized source only"
        );
        let complete_search = search_global_entries_on_connection(&connection, query, 20, 0)
            .unwrap_or_else(|error| panic!("run complete search for {tier}: {error}"));
        assert!(
            complete_search
                .iter()
                .all(|result| result.volume_id == authorized.id),
            "complete search for {tier} query must exclude disabled and legacy Windows sources"
        );
        assert!(
            complete_search
                .iter()
                .any(|result| result.id == authorized_entry.entry_id()),
            "complete search for {tier} query must preserve its authorized-source result"
        );
    }

    for query in ["candidateguard", "pathguard"] {
        let candidates = search_filename_trigram_candidates(&connection, query)
            .unwrap_or_else(|error| panic!("enumerate {query} candidates: {error}"));
        let candidate_ids = candidates
            .iter()
            .map(|candidate| {
                connection
                    .query_row(
                        "SELECT id FROM global_entries WHERE rowid=?1",
                        [candidate.rowid],
                        |row| row.get::<_, String>(0),
                    )
                    .expect("resolve candidate to stable entry id")
            })
            .collect::<Vec<_>>();
        let authorized_id = if query == "candidateguard" {
            authorized_entries[6].entry_id()
        } else {
            authorized_entries[7].entry_id()
        };
        assert_eq!(candidate_ids, vec![authorized_id.clone()]);

        let results = db
            .search_global_entries(query, 20, 0)
            .unwrap_or_else(|error| panic!("run filename-first search for {query}: {error}"));
        assert_eq!(ids(&results), vec![authorized_id]);
    }

    for unauthorized in [&disabled_entries[0], &legacy_entries[0]] {
        let candidate = RankedFileCandidate {
            rowid: rowid_for(&connection, &unauthorized.entry_id()),
            class: 1,
            modified_at: unauthorized.modified_at_fs,
            stable_id: Cow::Owned(unauthorized.entry_id().into_bytes()),
        };
        assert!(
            hydrate_ranked_filename_page(&connection, &[candidate]).is_err(),
            "result hydration must reject a candidate from an unauthorized Windows source"
        );
    }

    drop(connection);

    let changed_rows_before = db.filename_search.diagnostics().fallback_source_change;
    db.conn()
        .expect("obtain hooked source-state writer")
        .execute(
            "UPDATE global_volumes SET scope_authorization_state='legacy_confirmation_required' WHERE id=?1",
            [&authorized.id],
        )
        .expect("revoke Windows authorization while retaining enabled state");
    let revoked_results = db
        .search_global_entries("candidateguard", 20, 0)
        .expect("fallback search after source authorization changes");
    assert!(revoked_results.is_empty());
    assert!(
        db.filename_search.diagnostics().fallback_source_change > changed_rows_before,
        "a cached filename projection must be invalidated when an enabled source loses authorization"
    );

    drop(db);
    let _ = std::fs::remove_file(path);
}
