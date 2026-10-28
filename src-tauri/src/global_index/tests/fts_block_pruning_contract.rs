//! #474 architectural safety model, NOT an optimized FTS5 implementation.
//! Scores are taken from SQLite or synthesized before blocks are assembled.
//! An actual sidecar must prove conservative score bounds without first
//! scoring every FTS hit. This module does NOT qualify latency or memory.
use super::*;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Clone, Debug)]
struct Posting {
    id: String,
    score: f64,
    modified_at_fs: Option<i64>,
    enabled: bool,
    stale: bool,
}

impl PartialEq for Posting {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Posting {}

impl PartialOrd for Posting {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Posting {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.modified_at_fs.cmp(&self.modified_at_fs))
            .then_with(|| self.id.cmp(&other.id))
    }
}

#[derive(Clone)]
struct OracleBlock {
    /// Conservative lower bound on FTS5's *negative* BM25 rank.
    lower_score_bound: f64,
    generation: u64,
    postings: Vec<Posting>,
}

#[derive(Debug, Default)]
struct BlockStats {
    visited: usize,
    skipped: usize,
    inspected_postings: usize,
}

fn build_oracle_blocks(rows: &[Posting], block_size: usize, generation: u64) -> Vec<OracleBlock> {
    assert!(block_size > 0);
    assert!(rows.iter().all(|row| row.score.is_finite()));
    rows.chunks(block_size)
        .map(|chunk| OracleBlock {
            lower_score_bound: chunk
                .iter()
                .map(|row| row.score)
                .fold(f64::INFINITY, f64::min),
            generation,
            postings: chunk.to_vec(),
        })
        .collect()
}

fn oracle_bounds_valid(blocks: &[OracleBlock]) -> bool {
    blocks.iter().all(|block| {
        block.postings.iter().all(|row| {
            row.score.is_finite()
                && block.lower_score_bound.total_cmp(&row.score) != Ordering::Greater
        })
    })
}

fn exhaustive_top_k(rows: &[Posting], limit: usize) -> Vec<Posting> {
    let mut eligible = rows
        .iter()
        .filter(|row| row.enabled && !row.stale)
        .cloned()
        .collect::<Vec<_>>();
    eligible.sort();
    eligible.truncate(limit);
    eligible
}

fn bounded_top_k(
    blocks: &[OracleBlock],
    generation: u64,
    limit: usize,
) -> Result<(Vec<Posting>, BlockStats), &'static str> {
    if blocks.iter().any(|block| block.generation != generation) {
        return Err("sidecar_generation_mismatch");
    }
    if limit == 0 {
        return Ok((Vec::new(), BlockStats::default()));
    }
    let mut heap = BinaryHeap::<Posting>::with_capacity(limit);
    let mut stats = BlockStats::default();

    for block in blocks {
        // FTS5 BM25 is negative: smaller is *better*. Strict comparison
        // is essential. Equality must traverse the entire tied block so
        // modification-time/id secondary order can still change the winner.
        if heap.len() == limit
            && block
                .lower_score_bound
                .total_cmp(&heap.peek().expect("full heap has worst").score)
                == Ordering::Greater
        {
            stats.skipped += 1;
            continue;
        }
        stats.visited += 1;
        for row in &block.postings {
            stats.inspected_postings += 1;
            if !row.enabled || row.stale {
                continue;
            }
            if heap.len() < limit {
                heap.push(row.clone());
            } else if row < heap.peek().expect("full heap has worst") {
                heap.pop();
                heap.push(row.clone());
            }
        }
    }
    Ok((heap.into_sorted_vec(), stats))
}

fn row(id: usize, score: f64, enabled: bool, stale: bool) -> Posting {
    Posting {
        id: format!("global-entry-{id:06}"),
        score,
        modified_at_fs: Some((id % 197) as i64),
        enabled,
        stale,
    }
}

#[test]
fn exact_block_bound_respects_ties_visibility_and_generation() {
    // A high-selectivity score distribution should skip blocks only when
    // a mathematically valid lower score bound is strictly worse.
    let mut rows = (0..8192)
        .map(|i| {
            let score = if i < 256 { -10.0 } else { -0.2 };
            row(i, score, i % 11 != 0, i % 13 == 0)
        })
        .collect::<Vec<_>>();
    let blocks = build_oracle_blocks(&rows, 128, 17);
    assert!(oracle_bounds_valid(&blocks));
    let exact = exhaustive_top_k(&rows, 80);
    let (pruned, stats) = bounded_top_k(&blocks, 17, 80).expect("valid snapshot");
    assert_eq!(pruned, exact);
    assert!(
        stats.skipped > 0,
        "strong score bounds must skip weak blocks"
    );
    assert!(
        stats.inspected_postings < rows.len() / 4,
        "pruned model should avoid most postings on separable scores"
    );
    assert_eq!(
        bounded_top_k(&blocks, 18, 80).unwrap_err(),
        "sidecar_generation_mismatch",
        "an invalidated index must never serve an older visibility snapshot"
    );

    // FTS5 negative-BM25 ties defeat rank-only pruning even if a newer
    // document would beat the 80th result by the secondary keys.
    let tied = (0..2048)
        .map(|i| row(i, -0.8, i % 9 != 0, i % 31 == 0))
        .collect::<Vec<_>>();
    let tie_blocks = build_oracle_blocks(&tied, 128, 18);
    let (tie_top, ties) = bounded_top_k(&tie_blocks, 18, 80).expect("tie snapshot");
    assert_eq!(tie_top, exhaustive_top_k(&tied, 80));
    assert_eq!(ties.skipped, 0, "equal BM25 blocks cannot be skipped");

    let invisible = (0..512)
        .map(|i| row(i, -1.0, false, i % 2 == 0))
        .collect::<Vec<_>>();
    let (invisible_top, _) = bounded_top_k(&build_oracle_blocks(&invisible, 64, 19), 19, 80)
        .expect("disabled volume is not visible");
    assert!(invisible_top.is_empty());

    // A score-reducing update makes a previously stored bound unsafe.
    // A production writer must rebuild/recompute before admitting this
    // generation. The model only tests the rejection obligation.
    rows[500].score = -100.0;
    let mut stale_blocks = blocks.clone();
    stale_blocks[3].postings[500 - 3 * 128].score = -100.0;
    assert!(!oracle_bounds_valid(&stale_blocks));
    assert_ne!(
        exhaustive_top_k(&rows, 80),
        pruned,
        "the changed score should materially alter the reference top 80"
    );
    let repaired = build_oracle_blocks(&rows, 128, 18);
    let (repaired_top, _) = bounded_top_k(&repaired, 18, 80).expect("rebuilt generation");
    assert_eq!(repaired_top, exhaustive_top_k(&rows, 80));
}

#[test]
fn oracle_block_model_preserves_real_sqlite_fts5_scores_and_order() {
    let path = test_db_path();
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open isolated FTS5 contract database");
    db.upsert_global_volume(&test_volume())
        .expect("insert active source");
    let mut disabled = test_volume();
    disabled.id = "gv_disabled".to_string();
    disabled.stable_volume_id = "disabled-volume".to_string();
    disabled.enabled = false;
    db.upsert_global_volume(&disabled)
        .expect("insert disabled source");

    let mut inputs = Vec::new();
    let mut stale_ids = Vec::new();
    for i in 0..192 {
        let name = if i % 4 == 0 {
            format!("memo_{i:04}_report_report.txt")
        } else {
            format!("memo_{i:04}_report.txt")
        };
        let path = format!(r"C:\Global\Teams\{name}");
        let mut input = test_entry(&path, &name, false);
        input.modified_at_fs = Some(100 + (i / 3) as i64);
        if i % 17 == 0 {
            input.volume_id = disabled.id.clone();
        }
        if i % 19 == 0 {
            stale_ids.push(input.entry_id());
        }
        inputs.push(input);
    }
    db.upsert_global_entries_batch(&inputs)
        .expect("write fixture through real FTS5 triggers");
    let conn = db.conn().expect("read fixture through pooled connection");
    for id in stale_ids {
        conn.execute("UPDATE global_entries SET is_stale=1 WHERE id=?1", [id])
            .expect("mark fixture entry stale");
    }

    // This is an ORACLE MATERIALIZATION and pays the complete FTS5 cost.
    // It proves ordering only, NOT a viable indexed retrieval speedup.
    let mut stmt = conn
        .prepare(
            "SELECT ge.id, bm25(global_entries_fts, 8.0, 2.0, 1.0),
                    ge.modified_at_fs, gv.enabled, ge.is_stale
             FROM global_entries_fts
             CROSS JOIN global_entries ge
             CROSS JOIN global_volumes gv
             WHERE global_entries_fts MATCH ?1
               AND ge.rowid=global_entries_fts.rowid
               AND gv.id=ge.volume_id
             ORDER BY ge.rowid ASC",
        )
        .expect("prepare full SQLite FTS5 score oracle");
    let ranked = stmt
        .query_map(["\"report\""], |r| {
            Ok(Posting {
                id: r.get(0)?,
                score: r.get(1)?,
                modified_at_fs: r.get(2)?,
                enabled: r.get::<_, i64>(3)? != 0,
                stale: r.get::<_, i64>(4)? != 0,
            })
        })
        .expect("evaluate every FTS5 oracle score")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect real FTS5 BM25 scores");
    assert_eq!(ranked.len(), 192);
    let expected = exhaustive_top_k(&ranked, 80);
    let blocks = build_oracle_blocks(&ranked, 32, 99);
    let (actual, stats) = bounded_top_k(&blocks, 99, 80).expect("same-generation oracle");
    assert_eq!(actual, expected);

    let production = db
        .search_global_entries("report", 80, 0)
        .expect("production Search oracle");
    assert_eq!(
        actual.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        production.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
    );
    for (a, p) in actual.iter().zip(&production) {
        assert_eq!(a.score.to_bits(), p.rank.to_bits());
    }
    assert!(stats.visited > 0);
}
