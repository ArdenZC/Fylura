use super::*;
use rusqlite::{params, params_from_iter};
use snapshot::{Snapshot, Table};
use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Write},
    sync::{OnceLock, Weak},
};

#[derive(Clone)]
pub(crate) struct Candidate {
    pub(super) directory: PathBuf,
    pub(super) id: String,
    pub(super) source_digest: String,
    pub(super) seal: Option<Manifest>,
    pub(super) budget: BuildBudget,
    pub(super) usage: Arc<Usage>,
    scheduler: Arc<WorkScheduler>,
}
impl Candidate {
    pub(super) fn new(
        directory: PathBuf,
        id: String,
        budget: BuildBudget,
        scheduler: Arc<WorkScheduler>,
    ) -> Self {
        Self {
            directory,
            id,
            source_digest: String::new(),
            seal: None,
            budget,
            usage: Arc::new(Usage::default()),
            scheduler,
        }
    }
    pub(crate) fn path(&self) -> &Path {
        &self.directory
    }
    pub(super) fn admit(&self, phase: &str) -> Result<crate::scheduler::ResourceLease, DbError> {
        self.scheduler
            .try_acquire(WorkRequest::new(
                format!("shadow-{phase}-{}", uuid::Uuid::new_v4()),
                WorkClass::Background,
                ResourceHints::cpu_io(1, 1),
            ))
            .map_err(|_| unavailable("resource_deferred"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    format: u32,
    id: String,
    source_digest: String,
    pub(super) database_digest: String,
    pub(super) database_bytes: u64,
    effective_rows: u64,
}
#[derive(Serialize, Deserialize)]
struct Layout {
    tables: Vec<(String, Vec<String>)>,
    effective_rows: u64,
    query_revision: i64,
}

pub(crate) struct Verified {
    candidate: Candidate,
    registry: Arc<Registry>,
}
#[derive(Default)]
struct Life {
    retired: bool,
    readers: usize,
}
struct Registry {
    life: Mutex<Life>,
}

pub(crate) struct Shadow {
    pub(super) conn: std::mem::ManuallyDrop<Connection>,
    pub(super) candidate: Candidate,
    registry: Arc<Registry>,
}

#[cfg(test)]
thread_local! {pub(super) static STOP_AT:std::cell::RefCell<Option<(String,CancellationToken)>>=const {std::cell::RefCell::new(None)};}

pub(super) fn state(candidate: &Candidate, value: &str) -> Result<(), DbError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(candidate.directory.join("state"))?;
    f.write_all(value.as_bytes())?;
    f.sync_all()?;
    #[cfg(test)]
    if std::env::var("ZEN_SHADOW_CRASH_BARRIER").is_ok_and(|target| target == value) {
        // Test subprocess exit without Rust destructors; never production code.
        std::process::exit(91);
    }
    #[cfg(test)]
    STOP_AT.with(|hook| {
        if let Some((target, token)) = &*hook.borrow() {
            if target == value {
                token.cancel();
            }
        }
    });
    Ok(())
}

pub(super) fn build(
    candidate: &Candidate,
    captured: Snapshot,
    guard: &Guard,
) -> Result<(), DbError> {
    let mut charged = guard.clone();
    charged.charged = captured.tables.iter().flat_map(|t| &t.rows).try_fold(
        0u64,
        |sum, r| -> Result<u64, DbError> {
            Ok(sum.saturating_add((serde_json::to_vec(r)?.len() as u64) * 8 + 512))
        },
    )?;
    let guard = &charged;
    guard.check()?;
    let path = candidate.directory.join("index.sqlite");
    // create_new is the non-reuse boundary; SQLite opens this exclusively owned file.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let conn = Connection::open(&path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;
        PRAGMA cache_size=-256; PRAGMA mmap_size=0; PRAGMA trusted_schema=OFF;",
    )?;
    conn.pragma_update(
        None,
        "max_page_count",
        i64::try_from(guard.budget.private_bytes.saturating_sub(RESERVE) / 8192)
            .map_err(|_| unavailable("budget_overflow"))?,
    )?;
    let _progress = Progress::install(&conn, guard);
    let source_digest = digest(&captured)?;
    let layout = Layout {
        tables: captured
            .tables
            .iter()
            .map(|t| (t.name.clone(), t.columns.clone()))
            .collect(),
        effective_rows: captured.effective_rows,
        query_revision: captured.query_revision,
    };
    conn.execute_batch("BEGIN")?;
    for table in &captured.tables {
        guard.check()?;
        let columns = table
            .columns
            .iter()
            .map(|c| {
                if table.name == "files" && c == "rowid" {
                    "rowid INTEGER PRIMARY KEY".into()
                } else {
                    quote(c)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        conn.execute_batch(&format!("CREATE TABLE {} ({columns})", quote(&table.name)))?;
        let values = vec!["?"; table.columns.len()].join(",");
        let mut insert = conn.prepare(&format!(
            "INSERT INTO {} VALUES({values})",
            quote(&table.name)
        ))?;
        for row in &table.rows {
            guard.check()?;
            insert.execute(params_from_iter(row.iter()))?;
        }
    }
    conn.execute_batch(
        "CREATE TABLE shadow_layout(json TEXT NOT NULL);
        CREATE INDEX idx_file_user_tags_tag_file ON file_user_tags(tag_id,file_id);
        CREATE UNIQUE INDEX shadow_file_id ON files(id);",
    )?;
    conn.execute(
        "INSERT INTO shadow_layout VALUES(?1)",
        [serde_json::to_string(&layout)?],
    )?;
    conn.execute_batch("COMMIT")?;
    state(candidate, "CAPTURED")?;
    guard.check()?;
    state(candidate, "BUILDING")?;
    // Per-row insertion avoids an uninterruptible one-statement whole-corpus copy.
    conn.execute_batch(
        "CREATE VIRTUAL TABLE files_fts USING fts5(name,path,tokenize='trigram'); BEGIN",
    )?;
    {
        let mut select = conn.prepare("SELECT rowid,name,path FROM files ORDER BY rowid")?;
        let mut rows = select.query([])?;
        let mut insert = conn.prepare("INSERT INTO files_fts(rowid,name,path) VALUES(?1,?2,?3)")?;
        while let Some(row) = rows.next()? {
            guard.check()?;
            insert.execute(params![
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?
            ])?;
        }
    }
    conn.execute_batch("COMMIT; INSERT INTO files_fts(files_fts) VALUES('integrity-check')")?;
    if snapshot::validate_coverage(&conn)? != captured.effective_rows {
        return Err(unavailable("count_mismatch"));
    }
    guard.check()?;
    drop(_progress);
    conn.close().map_err(|(_, e)| e)?;
    // DELETE-journal SQLite, closed writer, and no dependency on WAL/SHM.
    no_sidecars(&path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?
        .sync_all()?;
    let manifest = Manifest {
        format: FORMAT,
        id: candidate.id.clone(),
        source_digest,
        database_digest: file_digest(&path, guard)?,
        database_bytes: fs::metadata(&path)?.len(),
        effective_rows: captured.effective_rows,
    };
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(candidate.directory.join("seal.json"))?;
    f.write_all(&serde_json::to_vec(&manifest)?)?;
    f.sync_all()?;
    state(candidate, "SEALED")?;
    guard.check()?;
    // The caller receives the exact expected manifest, not a status-only token.
    Ok(())
}

fn no_sidecars(path: &Path) -> Result<(), DbError> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut p = path.as_os_str().to_os_string();
        p.push(suffix);
        match fs::symlink_metadata(PathBuf::from(p)) {
            Ok(m) if m.file_type().is_symlink() || m.len() > 0 => {
                return Err(unavailable("unsealed_sidecar"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub(super) fn file_digest(path: &Path, guard: &Guard) -> Result<String, DbError> {
    let mut file = fs::File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0; 16384];
    loop {
        guard.check()?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

fn read_manifest(candidate: &Candidate) -> Result<Manifest, DbError> {
    let path = candidate.directory.join("seal.json");
    if fs::symlink_metadata(&candidate.directory)?
        .file_type()
        .is_symlink()
        || fs::symlink_metadata(&path)?.file_type().is_symlink()
        || fs::metadata(&path)?.len() > 16384
    {
        return Err(unavailable("manifest_invalid"));
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    if manifest.format != FORMAT
        || manifest.id != candidate.id
        || candidate.directory.file_name().and_then(|n| n.to_str()) != Some(&manifest.id)
    {
        return Err(unavailable("format_or_slot_mismatch"));
    }
    Ok(manifest)
}

/// Registers the expected seal at prepare completion, before exposing Candidate.
pub(super) fn bind(mut candidate: Candidate) -> Result<Candidate, DbError> {
    let seal = read_manifest(&candidate)?;
    candidate.source_digest = seal.source_digest.clone();
    candidate.seal = Some(seal);
    Ok(candidate)
}

static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<Registry>>>> = OnceLock::new();
fn registry(candidate: &Candidate) -> Result<Arc<Registry>, DbError> {
    let key = fs::canonicalize(&candidate.directory)?;
    let mut map = REGISTRY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| unavailable("registry_poisoned"))?;
    map.retain(|_, v| v.strong_count() > 0);
    if let Some(existing) = map.get(&key).and_then(Weak::upgrade) {
        return Ok(existing);
    }
    let registry = Arc::new(Registry {
        life: Mutex::new(Life::default()),
    });
    map.insert(key, Arc::downgrade(&registry));
    Ok(registry)
}
pub(crate) fn verify(candidate: &Candidate) -> Result<Verified, DbError> {
    let _single = admit_single()?;
    let _lease = candidate.admit("verify")?;
    let _qos = crate::resource_governor::scope_thread_qos(WorkClass::Background);

    let registry = registry(candidate)?;
    let life = registry
        .life
        .lock()
        .map_err(|_| unavailable("registry_poisoned"))?;
    if life.retired {
        return Err(unavailable("snapshot_expired"));
    }
    drop(life);
    checked_bytes(candidate)?;
    Ok(Verified {
        candidate: candidate.clone(),
        registry,
    })
}
fn checked_bytes(candidate: &Candidate) -> Result<(), DbError> {
    verify_bytes(candidate).map_err(|e| {
        let _ = state(candidate, "UNAVAILABLE");
        unavailable(&e.to_string())
    })
}
fn verify_bytes(candidate: &Candidate) -> Result<(), DbError> {
    let guard = Guard::new(
        candidate
            .directory
            .parent()
            .ok_or_else(|| unavailable("workspace"))?,
        Path::new("unused"),
        candidate.budget.clone(),
        CancellationToken::new(),
    );
    guard.check()?;
    let expected = candidate
        .seal
        .as_ref()
        .ok_or_else(|| unavailable("expected_seal_missing"))?;
    if &read_manifest(candidate)? != expected {
        return Err(unavailable("seal_mismatch"));
    }
    let path = candidate.directory.join("index.sqlite");
    no_sidecars(&path)?;
    if fs::metadata(&path)?.len() != expected.database_bytes
        || file_digest(&path, &guard)? != expected.database_digest
    {
        return Err(unavailable("database_digest_mismatch"));
    }
    let conn = snapshot::read_only(&path)?;
    let _progress = Progress::install(&conn, &guard);
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        return Err(unavailable("sqlite_integrity"));
    }
    let snapshot = materialized_snapshot(&conn, &guard)?;
    if digest(&snapshot)? != expected.source_digest
        || snapshot.effective_rows != expected.effective_rows
        || snapshot::validate_coverage(&conn)? != expected.effective_rows
    {
        return Err(unavailable("semantic_mismatch"));
    }
    drop(snapshot);
    verify_fts(&conn, &guard)?;
    state(candidate, "VERIFIED_SHADOW")?;
    Ok(())
}

fn materialized_snapshot(conn: &Connection, guard: &Guard) -> Result<Snapshot, DbError> {
    let text: String = conn.query_row("SELECT json FROM shadow_layout", [], |r| r.get(0))?;
    let layout: Layout = serde_json::from_str(&text)?;
    let mut tables = Vec::new();
    let mut charged = guard.clone();
    for (name, columns) in layout.tables {
        let fields = columns
            .iter()
            .map(|n| quote(n))
            .collect::<Vec<_>>()
            .join(",");
        let order = if name == "files" { "id" } else { "rowid" };
        let mut stmt = conn.prepare(&format!(
            "SELECT {fields} FROM {} ORDER BY {order}",
            quote(&name)
        ))?;
        let mut rows = stmt.query([])?;
        let mut table = Table {
            name,
            columns,
            rows: Vec::new(),
        };
        while let Some(row) = rows.next()? {
            let cells = (0..table.columns.len())
                .map(|i| cell(row, i))
                .collect::<Result<Vec<_>, _>>()?;
            charged.charged = charged
                .charged
                .saturating_add((serde_json::to_vec(&cells)?.len() as u64) * 8 + 512);
            if charged.charged > guard.budget.staging_bytes {
                return Err(unavailable("staging_budget"));
            }
            charged.check()?;
            table.rows.push(cells);
        }
        tables.push(table);
    }
    Ok(Snapshot {
        tables,
        effective_rows: layout.effective_rows,
        query_revision: layout.query_revision,
    })
}

/// Re-tokenize frozen text with bundled SQLite, then compare every indexed
/// token position. This checks FTS postings, not just its content table/count.
fn verify_fts(conn: &Connection, guard: &Guard) -> Result<(), DbError> {
    // READ_ONLY file flags still prohibit main writes. Only TEMP vocab is created.
    conn.execute_batch("PRAGMA query_only=OFF; CREATE VIRTUAL TABLE temp.shadow_vocab USING fts5vocab(main,files_fts,instance)")?;
    let reference = Connection::open_in_memory()?;
    reference.execute_batch("PRAGMA temp_store=MEMORY; CREATE VIRTUAL TABLE files_fts USING fts5(name,path,tokenize='trigram');
        CREATE VIRTUAL TABLE shadow_vocab USING fts5vocab(files_fts,instance)")?;
    let _progress = Progress::install(&reference, guard);
    let mut source = conn.prepare("SELECT rowid,name,path FROM files ORDER BY rowid")?;
    let mut rows = source.query([])?;
    while let Some(row) = rows.next()? {
        guard.check()?;
        reference.execute(
            "INSERT INTO files_fts(rowid,name,path) VALUES(?1,?2,?3)",
            params![
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?
            ],
        )?;
    }
    let sql = "SELECT term,doc,col,offset FROM shadow_vocab ORDER BY term,doc,col,offset";
    let mut a = conn.prepare(sql)?;
    let mut b = reference.prepare(sql)?;
    let mut aa = a.query([])?;
    let mut bb = b.query([])?;
    loop {
        guard.check()?;
        match (aa.next()?, bb.next()?) {
            (None, None) => break,
            (Some(a), Some(b)) => {
                for i in 0..4 {
                    if cell(a, i)? != cell(b, i)? {
                        return Err(unavailable("fts_corpus_mismatch"));
                    }
                }
            }
            _ => return Err(unavailable("fts_corpus_mismatch")),
        }
    }
    // Also compare stored text, including sub-trigram strings with no postings.
    if conn.query_row(
        "SELECT (SELECT COUNT(*) FROM files)!=(SELECT COUNT(*) FROM files_fts)
        OR EXISTS(SELECT 1 FROM files f LEFT JOIN files_fts t ON t.rowid=f.rowid
            WHERE t.rowid IS NULL OR t.name!=f.name OR t.path!=f.path)",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(unavailable("fts_content_mismatch"));
    }
    conn.execute_batch("DROP TABLE temp.shadow_vocab; PRAGMA query_only=ON")?;
    Ok(())
}

pub(crate) fn open_shadow(verified: &Verified) -> Result<Shadow, DbError> {
    let _single = admit_single()?;
    let candidate = &verified.candidate;
    let _lease = candidate.admit("open")?;
    let _qos = crate::resource_governor::scope_thread_qos(WorkClass::Background);

    let life = verified
        .registry
        .life
        .lock()
        .map_err(|_| unavailable("registry_poisoned"))?;
    if life.retired {
        return Err(unavailable("snapshot_expired"));
    }
    drop(life);
    checked_bytes(&verified.candidate)?;
    let conn = snapshot::read_only(&verified.candidate.directory.join("index.sqlite"))?;
    conn.execute_batch("BEGIN DEFERRED")?;
    conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_, i64>(0))?;
    let mut life = verified
        .registry
        .life
        .lock()
        .map_err(|_| unavailable("registry_poisoned"))?;
    life.readers += 1;
    Ok(Shadow {
        conn: std::mem::ManuallyDrop::new(conn),
        candidate: verified.candidate.clone(),
        registry: verified.registry.clone(),
    })
}
impl Drop for Shadow {
    fn drop(&mut self) {
        // Close the SQLite read txn before releasing the process-local pin.
        let _ = self.conn.execute_batch("ROLLBACK");
        // No fields access conn after this explicit close; Windows handles must
        // close before the last pin is released and retirement can delete.
        unsafe {
            std::mem::ManuallyDrop::drop(&mut self.conn);
        }
        if let Ok(mut life) = self.registry.life.lock() {
            life.readers = life.readers.saturating_sub(1);
        }
    }
}
/// Recovery has no authenticated source handle; preserve every state and require
/// an explicit new capture. No resumed batches or manifest-only authority.
pub(crate) fn recover_candidate(path: &Path) -> Result<Candidate, DbError> {
    if fs::symlink_metadata(path).is_err() {
        return Err(unavailable("candidate_missing"));
    }
    Err(unavailable("recovery_requires_fresh_capture"))
}
pub(crate) fn retire_candidate(verified: &Verified) -> Result<(), DbError> {
    let _single = admit_single()?;
    let mut life = verified
        .registry
        .life
        .lock()
        .map_err(|_| unavailable("registry_poisoned"))?;
    life.retired = true;
    let readers = life.readers;
    drop(life);
    state(&verified.candidate, "RETIRED")?;
    if readers != 0 {
        return Err(unavailable("retained_open_handles"));
    }
    fs::remove_dir_all(&verified.candidate.directory)?;
    Ok(())
}
