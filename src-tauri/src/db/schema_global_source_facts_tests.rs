use super::*;

fn fixture() -> Connection {
    let c = Connection::open_in_memory().unwrap();
    // Small relational fixture includes nullable legacy ID / MAX. Full product
    // integration uses Database::open and the real producer in Global tests.
    c.execute_batch("CREATE TABLE global_volumes(id TEXT PRIMARY KEY);
        INSERT INTO global_volumes VALUES('v'),('null'),('empty');
        CREATE TABLE global_entries(id TEXT PRIMARY KEY,volume_id TEXT NOT NULL,
        is_stale INTEGER NOT NULL,last_seen_at INTEGER);
        CREATE INDEX idx_global_entries_active_source_facts ON global_entries(volume_id,last_seen_at,id) WHERE is_stale=0;
        INSERT INTO global_entries VALUES('seed','v',0,10),(NULL,'null',0,NULL),('stale','v',1,999);") .unwrap();
    install(&c).unwrap();
    c
}
fn assert_exact(c: &Connection) {
    let mut stmt = c
        .prepare(
            "SELECT volume_id,COUNT(*),COUNT(id),MAX(last_seen_at) FROM global_entries
        WHERE is_stale=0 GROUP BY volume_id ORDER BY volume_id",
        )
        .unwrap();
    let expected = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut stmt = c.prepare("SELECT volume_id,active_rows,active_ids,max_last_seen_at FROM global_entry_source_facts WHERE active_rows>0 ORDER BY volume_id").unwrap();
    let actual = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(c.query_row("SELECT COUNT(*) FROM global_entry_source_facts WHERE active_rows=0 AND (active_ids<>0 OR max_last_seen_at IS NOT NULL)",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}
#[test]
fn source_facts_all_mutation_shapes_and_null_max_are_exact() {
    let c = fixture();
    assert_exact(&c);
    for sql in [
        "INSERT INTO global_entries VALUES('a','v',0,10)",
        "INSERT INTO global_entries VALUES(NULL,'v',0,11)",
        "UPDATE global_entries SET last_seen_at=3 WHERE id='a'",
        "UPDATE global_entries SET id=NULL WHERE id='seed'",
        "UPDATE global_entries SET id='renamed',last_seen_at=NULL WHERE id='a'",
        "UPDATE global_entries SET volume_id='other' WHERE id='renamed'",
        "UPDATE global_entries SET is_stale=1 WHERE volume_id='v'",
        "UPDATE global_entries SET is_stale=0,last_seen_at=55 WHERE id='stale'",
        "UPDATE global_entries SET last_seen_at=20 WHERE id='stale'",
        "DELETE FROM global_entries WHERE id='stale'",
        "UPDATE global_entries SET volume_id='orphan',is_stale=0,last_seen_at=100",
        "DELETE FROM global_entries",
    ] {
        c.execute_batch(sql).unwrap();
        assert_exact(&c);
    }
}
#[test]
fn source_facts_randomized_updates_match_canonical_after_every_statement() {
    let c = fixture();
    let mut seed = 177_u64;
    for step in 0..2500 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = (seed >> 32) % 71;
        let volume = (seed >> 28) % 5;
        let seen = (seed >> 24) % 49;
        let sql=match seed%6 {
            0=>format!("DELETE FROM global_entries WHERE id='r{id}'"),
            1=>format!("UPDATE global_entries SET volume_id='v{volume}' WHERE id='r{id}'"),
            2=>format!("UPDATE global_entries SET is_stale=1-is_stale WHERE id='r{id}'"),
            3=>format!("UPDATE global_entries SET last_seen_at={} WHERE id='r{id}'",if seen.is_multiple_of(7) {"NULL".into()} else {seen.to_string()}),
            _=>format!("INSERT INTO global_entries VALUES('r{id}','v{volume}',{}, {seen}) ON CONFLICT(id) DO UPDATE SET volume_id=excluded.volume_id,is_stale=excluded.is_stale,last_seen_at=excluded.last_seen_at",step%2),
        };
        c.execute_batch(&sql).unwrap();
        assert_exact(&c);
    }
}
#[test]
fn source_facts_ddl_rollback_signature_and_repair_are_bounded() {
    let c = fixture();
    c.execute_batch("BEGIN; UPDATE global_entries SET is_stale=1; ROLLBACK;")
        .unwrap();
    assert_exact(&c);
    c.execute_batch("DROP TRIGGER global_entry_source_facts_ad")
        .unwrap();
    assert!(validate(&c).is_err());
    // Migration/explicit repair is caller-write-transaction owned, not startup.
    c.execute_batch("BEGIN").unwrap();
    install(&c).unwrap();
    c.execute_batch("ROLLBACK").unwrap();
    assert!(validate(&c).is_err());
    c.execute_batch("BEGIN").unwrap();
    install(&c).unwrap();
    c.execute_batch("COMMIT").unwrap();
    assert_exact(&c);
    c.execute_batch("CREATE TEMP TABLE global_entries(x)")
        .unwrap();
    assert!(!materialized_relation_available(&c).unwrap());
}
