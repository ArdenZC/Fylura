use super::*;

#[test]
fn schema36_to37_preserves_populated_manual_history_and_reopens() {
    for populated in [false, true] {
        let f = Fixture::new();
        let path = f.db().path().with_file_name("legacy36.sqlite3");
        let db = Database::open(&path).unwrap();
        let conn = db.conn().unwrap();
        crate::db::drop_source_writer_schema_for_test(&conn);
        conn.execute_batch("DROP TABLE automation_trigger_state; DROP TABLE automation_runs; DROP TABLE automation_intents; ALTER TABLE scan_roots DROP COLUMN library_change_revision; ALTER TABLE files DROP COLUMN filesystem_observation_key;").unwrap();
        crate::db::schema_automation::ensure_automation_schema(&conn).unwrap();
        conn.execute_batch("PRAGMA user_version=36; INSERT INTO app_settings(key,value) VALUES('migration-preserved','unchanged');").unwrap();
        if populated {
            conn.execute("INSERT INTO automation_intents(id,revision,title,workflow_kind,scope_query_json,scope_fingerprint,trigger_json,policy_json,enabled,created_at,updated_at) VALUES('legacy',7,'Legacy','organize_plan',?1,'fingerprint','{\"version\":1,\"kind\":\"manual\"}','{\"version\":1,\"review\":\"required\",\"autoExecute\":false}',1,123,124)",[serde_json::to_string(&draft().scope_query).unwrap()]).unwrap();
            conn.execute_batch("INSERT INTO automation_runs(id,request_key,intent_id,intent_revision,trigger_kind,scope_fingerprint,status,requires_plan_refresh,created_at,completed_at,result_plan_id) VALUES('legacy-run','preserved-request-key','legacy',7,'manual','fingerprint','completed',0,123,124,'historical-plan');").unwrap();
        }
        drop(conn);
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            42
        );
        super::super::tests::assert_dormant_schema42(&db.conn().unwrap());
        assert_eq!(
            db.conn()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key='migration-preserved'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "unchanged"
        );
        if populated {
            let i = db.get_automation_intent("legacy").unwrap();
            assert_eq!(i.revision, 7);
            assert_eq!(i.trigger.version, 2);
            assert_eq!(i.trigger.kind, "manual");
            let run = db.list_automation_runs(None).unwrap().remove(0);
            assert_eq!(run.request_key, "preserved-request-key");
            assert_eq!(run.result_plan_id.as_deref(), Some("historical-plan"));
            assert_eq!(run.trigger_context, json!({"version":1,"kind":"manual"}));
        }
        let states: i64 = db
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM automation_trigger_state", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(states, 0);
        drop(db);
        let reopened = Database::open(&path).unwrap();
        super::super::tests::assert_dormant_schema42(&reopened.conn().unwrap());
    }
}

#[test]
fn schema37_rejects_invalid_trigger_shapes_and_calendar_ranges() {
    let f = Fixture::new();
    let i = intent(&f, trigger(json!({"version":2,"kind":"manual"})));
    for invalid in [
        json!({"version":1,"kind":"manual"}),
        json!({"version":2,"kind":"startup"}),
        json!({"version":2,"kind":"manual","timeZone":null}),
        json!({"version":2,"kind":"schedule","timeZone":"UTC","localTime":"24:00","weekdays":[1]}),
        json!({"version":2,"kind":"schedule","timeZone":"UTC","localTime":"09:00:00","weekdays":[1]}),
        json!({"version":2,"kind":"schedule","timeZone":"UTC","localTime":"09:00","weekdays":[0]}),
        json!({"version":2,"kind":"schedule","timeZone":"UTC","localTime":"09:00","weekdays":[1,1]}),
        json!({"version":2,"kind":"schedule","timeZone":"UTC","localTime":"09:00","weekdays":[2,1]}),
    ] {
        assert!(f
            .db()
            .conn()
            .unwrap()
            .execute(
                "UPDATE automation_intents SET trigger_json=?2 WHERE id=?1",
                params![i.id, invalid.to_string()]
            )
            .is_err());
    }
}
