//! Distinct, test-only native FTS rank-stream candidate. Never drops the
//! primary-rank boundary ties; the heap applies the full production order.
use super::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
std::thread_local! { static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
pub(crate) fn enabled() -> bool {
    ENABLED.with(std::cell::Cell::get)
}
pub(crate) fn with_candidate<T>(run: impl FnOnce() -> T) -> T {
    with_enabled(true, run)
}
pub(crate) fn with_baseline<T>(run: impl FnOnce() -> T) -> T {
    with_enabled(false, run)
}
fn with_enabled<T>(enabled: bool, run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            ENABLED.with(|v| v.set(self.0));
        }
    }
    let old = ENABLED.with(|v| v.replace(enabled));
    let _reset = Reset(old);
    run()
}
pub(crate) const SQL: &str = "SELECT ge.rowid, global_entries_fts.rank, ge.modified_at_fs, ge.id
    FROM global_entries_fts CROSS JOIN global_entries ge CROSS JOIN global_volumes gv
    WHERE global_entries_fts MATCH ?1 AND global_entries_fts.rank MATCH 'bm25(8.0, 2.0, 1.0)'
      AND ge.rowid=global_entries_fts.rowid AND gv.id=ge.volume_id AND gv.enabled=1 AND ge.is_stale=0
    ORDER BY global_entries_fts.rank";
#[derive(Debug)]
struct Key {
    rowid: i64,
    rank: f64,
    modified: Option<i64>,
    id: Option<String>,
}
impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Key {}
impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank
            .total_cmp(&other.rank)
            .then_with(|| other.modified.cmp(&self.modified))
            .then_with(|| self.id.cmp(&other.id))
            .then_with(|| self.rowid.cmp(&other.rowid))
    }
}
pub(crate) fn run(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<GlobalSearchResult>, DbError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    // The native rank stream and full-row fetch must share a read snapshot even
    // for the standalone pooled API, whose previous FTS tier was one statement.
    let transaction = if conn.is_autocommit() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let read = transaction.as_deref().unwrap_or(conn);
    let mut heap = BinaryHeap::<Key>::with_capacity(limit as usize);
    {
        let mut stmt = read.prepare(SQL)?;
        let phrase = format!("\"{}\"", query.replace('"', "\"\""));
        let mut rows = stmt.query([phrase])?;
        while let Some(row) = rows.next()? {
            let key = Key {
                rowid: row.get(0)?,
                rank: row.get(1)?,
                modified: row.get(2)?,
                id: row.get(3)?,
            };
            if heap.len() == limit as usize && key.rank > heap.peek().unwrap().rank {
                break;
            }
            if heap.len() < limit as usize {
                heap.push(key);
            } else if key < *heap.peek().unwrap() {
                *heap.peek_mut().unwrap() = key;
            }
        }
    }
    let keys = heap.into_sorted_vec();
    let ids = serde_json::to_string(&keys.iter().map(|k| k.rowid).collect::<Vec<_>>())
        .map_err(|e| DbError::Validation(e.to_string()))?;
    let sql = candidate_sql(
        "json_each(?1) pick JOIN global_entries ge ON ge.rowid=pick.value",
        "1=1",
        "CAST(pick.key AS INTEGER) ASC",
        "0.0",
        "?2",
    );
    let mut stmt = read.prepare(&sql)?;
    let mut result = collect_candidates(&mut stmt, params![ids, limit])?;
    drop(stmt);
    if result.len() != keys.len() {
        return Err(DbError::Validation("fts_frontier_snapshot_changed".into()));
    }
    // Keep original IEEE BM25 values, without a JSON floating-point round trip.
    for (row, key) in result.iter_mut().zip(keys) {
        row.rank = key.rank;
    }
    if let Some(tx) = transaction {
        tx.commit()?;
    }
    Ok(result)
}
