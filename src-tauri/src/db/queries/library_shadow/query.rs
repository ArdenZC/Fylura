use super::super::{
    build_fts_query,
    library::{self, FileQuerySpecV2, LibraryCursor, LibrarySortKind},
};
use super::*;
use rusqlite::{params_from_iter, types::Value};

/// Comparison-only DTO: never an operation/tag/restore selection authority.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct ShadowRow {
    pub file_id: String,
    pub source_uid: String,
    pub path: String,
    pub name: String,
    pub size: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub confidence: f64,
    pub rank: Option<f64>,
    pub tags: Vec<String>,
}
#[derive(Debug)]
pub(crate) struct ShadowPage {
    pub candidate_id: String,
    pub seal_digest: String,
    pub rows: Vec<ShadowRow>,
    pub total: i64,
    pub next_cursor: Option<String>,
    pub fingerprint: String,
}
#[derive(Debug)]
pub(crate) struct ShadowSelection {
    pub file_ids: Vec<String>,
    pub excluded_count: usize,
    pub missing_count: usize,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    candidate_id: String,
    seal_digest: String,
    tuple: LibraryCursor,
}

struct Parts {
    cte: String,
    from: String,
    conditions: Vec<String>,
    params: Vec<Value>,
}
impl Shadow {
    fn guard(&self) -> Guard {
        Guard::new(
            self.candidate
                .directory
                .parent()
                .unwrap_or(&self.candidate.directory),
            Path::new("unused"),
            self.candidate.budget.clone(),
            CancellationToken::new(),
        )
    }
    fn parts(&self, spec: &FileQuerySpecV2) -> Result<Parts, DbError> {
        let resolved = library::resolve_scope(&self.conn, &spec.scope)?;
        if resolved.health.state == "invalid_reference" {
            return Err(unavailable("invalid_reference"));
        }
        if resolved.health.state == "partial" {
            return Err(unavailable("scope_unavailable"));
        }
        let root_ids = resolved
            .health
            .roots
            .into_iter()
            .filter(|r| r.available)
            .map(|r| r.id)
            .collect::<Vec<_>>();
        let mut conditions = vec!["f.is_stale=0".to_string()];
        let mut params = Vec::new();
        let (cte, from) = if let Some(text) = spec.text.as_deref() {
            params.push(Value::Text(
                build_fts_query(text).ok_or_else(|| unavailable("text_invalid"))?,
            ));
            ("WITH fts_matches AS MATERIALIZED (SELECT rowid,bm25(files_fts,6.0,1.5) AS rank FROM files_fts WHERE files_fts MATCH ?)".into(),"files f JOIN fts_matches fm ON fm.rowid=f.rowid".into())
        } else {
            (String::new(), "files f".into())
        };
        if root_ids.is_empty() {
            conditions.push("0".into());
        } else {
            conditions.push(format!("EXISTS(SELECT 1 FROM library_source_root_claims c WHERE c.observed_path=f.path AND c.claim_state='present' AND c.root_id IN ({}))",vec!["?";root_ids.len()].join(",")));
            params.extend(root_ids.into_iter().map(Value::Text));
        }
        library::append_filters(&self.conn, &mut conditions, &mut params, &spec.filters)?;
        if library::query_has_missing_tags(&self.conn, &spec.filters)? {
            conditions.push("0".into());
        }
        Ok(Parts {
            cte,
            from,
            conditions,
            params,
        })
    }

    pub(crate) fn query(
        &self,
        spec: FileQuerySpecV2,
        page_size: u32,
        cursor: Option<&str>,
    ) -> Result<ShadowPage, DbError> {
        let _lease = self.candidate.admit("query")?;
        let _qos = crate::resource_governor::scope_thread_qos(WorkClass::Background);
        if !(1..=200).contains(&page_size) {
            return Err(unavailable("page_size"));
        }
        let guard = self.guard();
        guard.check()?;
        let _progress = Progress::install(&self.conn, &guard);
        let (spec, _, fingerprint) = library::canonicalize_file_query_spec(spec)?;
        let mut parts = self.parts(&spec)?;
        let total: i64 = self.conn.query_row(
            &format!(
                "{} SELECT COUNT(*) FROM {} WHERE {}",
                parts.cte,
                parts.from,
                parts.conditions.join(" AND ")
            ),
            params_from_iter(parts.params.iter()),
            |r| r.get(0),
        )?;
        let seal_digest = digest(
            self.candidate
                .seal
                .as_ref()
                .ok_or_else(|| unavailable("seal_missing"))?,
        )?;
        if let Some(encoded) = cursor {
            if encoded.len() > 12000 {
                return Err(unavailable("snapshot_expired"));
            }
            let c: Cursor =
                serde_json::from_str(encoded).map_err(|_| unavailable("snapshot_expired"))?;
            if c.candidate_id != self.candidate.id
                || c.seal_digest != seal_digest
                || c.tuple.fingerprint != fingerprint
                || c.tuple.total_count != total
                || c.tuple.version != 1
                || c.tuple.sort_kind != spec.sort.kind
                || c.tuple.direction != spec.sort.direction
            {
                return Err(unavailable("snapshot_expired"));
            }
            // Recompute the complete tuple on this same frozen candidate. Legal
            // JSON tampering must not skip arbitrary rows or forge rank bits.
            let mut anchor_conditions = parts.conditions.clone();
            anchor_conditions.push("f.id=?".into());
            let mut anchor_params = parts.params.clone();
            anchor_params.push(Value::Text(c.tuple.file_id.clone()));
            let anchor = self.rows(&parts, &anchor_conditions, &anchor_params, 1, &spec)?;
            let row = anchor
                .first()
                .ok_or_else(|| unavailable("snapshot_expired"))?;
            if serde_json::to_string(&tuple(row, &spec, &fingerprint, total))?
                != serde_json::to_string(&c.tuple)?
            {
                return Err(unavailable("snapshot_expired"));
            }
            library::append_cursor_condition(
                &mut parts.conditions,
                &mut parts.params,
                &c.tuple,
                &spec.sort,
            )?;
        }
        let mut rows = self.rows(
            &parts,
            &parts.conditions,
            &parts.params,
            page_size + 1,
            &spec,
        )?;
        let more = rows.len() > page_size as usize;
        rows.truncate(page_size as usize);
        let next_cursor = if more {
            let row = rows.last().ok_or_else(|| unavailable("page_empty"))?;
            Some(serde_json::to_string(&Cursor {
                candidate_id: self.candidate.id.clone(),
                seal_digest: seal_digest.clone(),
                tuple: tuple(row, &spec, &fingerprint, total),
            })?)
        } else {
            None
        };
        guard.check()?;
        Ok(ShadowPage {
            candidate_id: self.candidate.id.clone(),
            seal_digest,
            rows,
            total,
            next_cursor,
            fingerprint,
        })
    }

    fn rows(
        &self,
        parts: &Parts,
        conditions: &[String],
        params: &[Value],
        limit: u32,
        spec: &FileQuerySpecV2,
    ) -> Result<Vec<ShadowRow>, DbError> {
        let rank = if spec.text.is_some() {
            "fm.rank"
        } else {
            "NULL"
        };
        let sql=format!("{} SELECT f.id,(SELECT c.source_uid FROM library_source_root_claims c WHERE c.observed_path=f.path AND c.claim_state='present' LIMIT 1),f.path,f.name,f.size,f.mtime,f.ctime,f.confidence,{rank}
            FROM {} WHERE {} ORDER BY {} LIMIT ?",parts.cte,parts.from,conditions.join(" AND "),library::order_clause(&spec.sort,spec.text.is_some()));
        let mut values = params.to_vec();
        values.push(Value::Integer(i64::from(limit)));
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params_from_iter(values.iter()))?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let mut tags = self
                .conn
                .prepare("SELECT tag_id FROM file_user_tags WHERE file_id=?1 ORDER BY tag_id")?;
            let tags = tags
                .query_map([&id], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            result.push(ShadowRow {
                file_id: id,
                source_uid: row.get(1)?,
                path: row.get(2)?,
                name: row.get(3)?,
                size: row.get(4)?,
                mtime: row.get(5)?,
                ctime: row.get(6)?,
                confidence: row.get(7)?,
                rank: row.get(8)?,
                tags,
            });
        }
        Ok(result)
    }

    /// Comparison IDs only. AllMatching consumes the full bounded candidate,
    /// never a rendered page. No implicit conversion to LibrarySelectionV1.
    pub(crate) fn select_matching(
        &self,
        spec: FileQuerySpecV2,
        excluded: &[String],
    ) -> Result<ShadowSelection, DbError> {
        if excluded.len() > 100000 {
            return Err(unavailable("selection_limit"));
        }
        let guard = self.guard();
        let mut cursor = None;
        let mut ids = Vec::new();
        let mut excluded_count = 0;
        loop {
            guard.check()?;
            let page = self.query(spec.clone(), 200, cursor.as_deref())?;
            for row in page.rows {
                if excluded.contains(&row.file_id) {
                    excluded_count += 1;
                } else {
                    ids.push(row.file_id);
                }
            }
            if ids.len() + excluded_count > 100000 {
                return Err(unavailable("selection_limit"));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(ShadowSelection {
            file_ids: ids,
            excluded_count,
            missing_count: 0,
        })
    }
    pub(crate) fn select_explicit(&self, ids: &[String]) -> Result<ShadowSelection, DbError> {
        if ids.len() > 100000 {
            return Err(unavailable("selection_limit"));
        }
        let mut unique = ids.to_vec();
        unique.sort();
        unique.dedup();
        let total = unique.len();
        let guard = self.guard();
        let mut found = Vec::new();
        for id in unique {
            guard.check()?;
            if self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM files WHERE id=?1)",
                [&id],
                |r| r.get::<_, bool>(0),
            )? {
                found.push(id);
            }
        }
        Ok(ShadowSelection {
            missing_count: total - found.len(),
            file_ids: found,
            excluded_count: 0,
        })
    }
}
fn tuple(row: &ShadowRow, spec: &FileQuerySpecV2, fingerprint: &str, total: i64) -> LibraryCursor {
    let mut c = LibraryCursor {
        version: 1,
        fingerprint: fingerprint.into(),
        revision: 0,
        total_count: total,
        sort_kind: spec.sort.kind.clone(),
        direction: spec.sort.direction.clone(),
        file_id: row.file_id.clone(),
        last_i64: None,
        last_text: None,
        last_f64_bits: None,
        last_rank_bits: None,
        last_mtime: None,
    };
    match spec.sort.kind {
        LibrarySortKind::Modified => c.last_i64 = Some(row.mtime),
        LibrarySortKind::Created => c.last_i64 = Some(row.ctime),
        LibrarySortKind::Name => c.last_text = Some(row.name.clone()),
        LibrarySortKind::Size => c.last_i64 = Some(row.size),
        LibrarySortKind::Confidence => c.last_f64_bits = Some(row.confidence.to_bits()),
        LibrarySortKind::Relevance => {
            c.last_rank_bits = Some(row.rank.unwrap_or(0.).to_bits());
            c.last_mtime = Some(row.mtime);
            c.last_text = Some(row.name.clone());
        }
    }
    c
}
