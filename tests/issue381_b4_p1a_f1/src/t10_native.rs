//! Hosted, native-filesystem T10 research. Every fixture is disposable and
//! rooted beneath T10_SANDBOX_ROOT when the hosted workflow sets it.

use crate::db::{
    DirectorySyncAttempt, ManifestExpectation, PRODUCT_BUSY_TIMEOUT_MS, PRODUCT_POOL_SIZE,
    create_native_test_slot, open_connection, open_product_pool, pool_get,
    open_readonly, query_pool_settings, query_rows, read_only_slot_tx, sha256_bytes, suffix,
    verify_manifest_file, verify_seal,
};
use crate::model::{Document, documents, file_sha256};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type T10Result<T> = Result<T, Box<dyn Error>>;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> T10Result<Self> {
        let base = std::env::var_os("T10_SANDBOX_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        if !base.is_absolute() {
            return Err("T10 sandbox root must be absolute".into());
        }
        fs::create_dir_all(&base)?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "issue381-t10-{label}-{}-{nonce}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn cleanup(&self) -> io::Result<()> {
        fs::remove_dir_all(&self.0)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct CaseReport {
    status: &'static str,
    details: Value,
}

fn report(status: &'static str, details: Value) -> CaseReport {
    CaseReport { status, details }
}

fn hosted_native_target() -> bool {
    matches!(std::env::consts::OS, "windows" | "macos")
        && std::env::var("GITHUB_ACTIONS").is_ok_and(|value| value == "true")
        && std::env::var("T10_HOSTED_RUNNER").is_ok_and(|value| value == "true")
}

fn emit_case(case_id: &'static str, status: &'static str, details: Value) {
    println!(
        "{}",
        json!({
            "record_type":"t10_case_result",
            "case_id":case_id,
            "status":status,
            "details":details
        })
    );
}

fn error_details(error: &(dyn Error + 'static)) -> Value {
    if let Some(error) = error.downcast_ref::<io::Error>() {
        return json!({
            "error_class":"io",
            "error_kind":format!("{:?}", error.kind()),
            "raw_os_error":error.raw_os_error()
        });
    }
    if let Some(error) = error.downcast_ref::<rusqlite::Error>() {
        return match error {
            rusqlite::Error::SqliteFailure(code, _) => json!({
                "error_class":"sqlite",
                "sqlite_error_code":format!("{:?}", code.code),
                "sqlite_extended_code":code.extended_code
            }),
            _ => json!({"error_class":"rusqlite","error_variant":"other_rusqlite_error"}),
        };
    }
    let mut summary = error.to_string();
    for variable in ["T10_SANDBOX_ROOT", "RUNNER_TEMP", "GITHUB_WORKSPACE"] {
        if let Some(path) = std::env::var_os(variable) {
            summary = summary.replace(&path.to_string_lossy().to_string(), "<runner-temp-or-workspace>");
        }
    }
    json!({"error_class":"harness","error_summary":summary})
}

fn run_case<F>(case_id: &'static str, test: F) -> (bool, bool)
where
    F: FnOnce() -> T10Result<CaseReport>,
{
    let result = match test() {
        Ok(result) => result,
        Err(error) => report("FAIL", error_details(&*error)),
    };
    let result = if !hosted_native_target() && result.status != "FAIL" {
        report(
            "ENVIRONMENT_UNAVAILABLE",
            json!({
                "executed_probe_status":result.status,
                "native_hosted_qualification":"not counted; --t10-native requires a GitHub-hosted Windows or macOS runner",
                "executed_os":std::env::consts::OS,
                "details":result.details
            }),
        )
    } else {
        result
    };
    let failed = result.status == "FAIL";
    let unavailable = result.status == "ENVIRONMENT_UNAVAILABLE";
    emit_case(case_id, result.status, result.details);
    (failed, unavailable)
}

fn io_observation(operation: &'static str, result: io::Result<()>) -> Value {
    match result {
        Ok(()) => json!({
            "operation":operation,
            "outcome":"ALLOWED",
            "error_kind":null,
            "raw_os_error":null
        }),
        Err(error) => json!({
            "operation":operation,
            "outcome":"DENIED",
            "error_kind":format!("{:?}", error.kind()),
            "raw_os_error":error.raw_os_error()
        }),
    }
}

fn command_text(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(target_os = "windows")]
fn os_version() -> Option<String> {
    command_text("cmd", &["/c", "ver"])
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    command_text("sw_vers", &["-productVersion"])
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn os_version() -> Option<String> {
    command_text("uname", &["-sr"])
}

fn lock_packages(path: &Path) -> T10Result<BTreeMap<(String, String), String>> {
    let content = fs::read_to_string(path)?;
    let mut packages = BTreeMap::new();
    for package in content.split("[[package]]").skip(1) {
        let mut name = None;
        let mut version = None;
        let mut source = None;
        let mut checksum = None;
        for line in package.lines().map(str::trim) {
            if let Some(value) = line.strip_prefix("name = ") {
                name = value.trim_matches('"').to_owned().into();
            } else if let Some(value) = line.strip_prefix("version = ") {
                version = value.trim_matches('"').to_owned().into();
            } else if let Some(value) = line.strip_prefix("source = ") {
                source = value.trim_matches('"').to_owned().into();
            } else if let Some(value) = line.strip_prefix("checksum = ") {
                checksum = value.trim_matches('"').to_owned().into();
            }
            if line.starts_with("[[") && name.is_some() {
                break;
            }
        }
        if source.as_deref().is_some_and(|value| value.starts_with("registry+")) {
            if let (Some(name), Some(version), Some(checksum)) = (name, version, checksum) {
                packages.insert((name, version), checksum);
            }
        }
    }
    Ok(packages)
}

fn lock_parity() -> T10Result<Value> {
    let harness_lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
    let product_lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/Cargo.lock");
    let harness = lock_packages(&harness_lock)?;
    let product = lock_packages(&product_lock)?;
    let missing_or_mismatched = harness
        .iter()
        .filter(|(package, checksum)| product.get(*package) != Some(*checksum))
        .map(|((name, version), _)| format!("{name}@{version}"))
        .collect::<Vec<_>>();
    Ok(json!({
        "harness_registry_package_count":harness.len(),
        "product_registry_package_count":product.len(),
        "every_harness_registry_version_and_checksum_in_product_lock":missing_or_mismatched.is_empty(),
        "mismatches":missing_or_mismatched,
        "product_lock_scope":"src-tauri/Cargo.lock",
        "harness_lock_scope":"tests/issue381_b4_p1a_f1/Cargo.lock"
    }))
}

fn source_identity() -> T10Result<Value> {
    let connection = Connection::open_in_memory()?;
    let sqlite_version: String =
        connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
    let sqlite_source_id: String =
        connection.query_row("SELECT sqlite_source_id()", [], |row| row.get(0))?;
    let fts5: i64 = connection.query_row(
        "SELECT sqlite_compileoption_used('ENABLE_FTS5')",
        [],
        |row| row.get(0),
    )?;
    connection.execute_batch(
        "CREATE VIRTUAL TABLE temp.trigram_probe USING fts5(text,tokenize='trigram');\
         INSERT INTO temp.trigram_probe(text) VALUES('native trigram qualification');",
    )?;
    let trigram_matches: i64 = connection.query_row(
        "SELECT COUNT(*) FROM temp.trigram_probe WHERE trigram_probe MATCH 'native'",
        [],
        |row| row.get(0),
    )?;
    let sqlite_override_variables = std::env::vars_os()
        .filter_map(|(key, _)| {
            let key = key.to_string_lossy();
            (key.starts_with("SQLITE_") || key.starts_with("LIBSQLITE3_")).then(|| key.into_owned())
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "sqlite_version":sqlite_version,
        "sqlite_source_id":sqlite_source_id,
        "sqlite_fts5_compile_option":fts5,
        "bundled_sqlite_expected":true,
        "rusqlite":"0.39.0",
        "libsqlite3_sys":"0.37.0",
        "trigram_fts_smoke_matches":trigram_matches,
        "sqlite_system_override_variables":sqlite_override_variables
    }))
}

#[derive(Clone)]
struct NativeFixture {
    authority_path: PathBuf,
    slot_path: PathBuf,
    manifest_path: PathBuf,
    candidate_path: PathBuf,
    candidate_manifest_path: PathBuf,
    slot_id: String,
    generation: i64,
    manifest_sha256: String,
    candidate_sha256: String,
    manifest: crate::db::SlotManifest,
    candidate_manifest: crate::db::SlotManifest,
}

fn fixture(temp: &TempRoot, suffix_name: &str, docs: &[Document]) -> T10Result<NativeFixture> {
    let slot_id = format!("E0-{suffix_name}");
    let e0_path = temp.path(&format!("{suffix_name}-E0.sqlite"));
    let e1_path = temp.path(&format!("{suffix_name}-E1.sqlite"));
    let e0 = create_native_test_slot(&e0_path, &slot_id, docs)?;
    let e1 = create_native_test_slot(&e1_path, &format!("E1-{suffix_name}"), docs)?;
    let authority_path = temp.path(&format!("{suffix_name}-authority.sqlite"));
    let authority = open_connection(&authority_path)?;
    authority.pragma_update(None, "journal_mode", "WAL")?;
    authority.pragma_update(None, "synchronous", "NORMAL")?;
    authority.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    authority.execute_batch(
        "CREATE TABLE t10_authority(singleton INTEGER PRIMARY KEY CHECK(singleton=1),slot_id TEXT NOT NULL,generation INTEGER NOT NULL,manifest_sha256 TEXT NOT NULL);\
         CREATE TABLE t10_slots(slot_id TEXT NOT NULL,generation INTEGER NOT NULL,state TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,PRIMARY KEY(slot_id,generation));\
         CREATE TABLE t10_leases(lease_id TEXT PRIMARY KEY,slot_id TEXT NOT NULL,generation INTEGER NOT NULL,process_id INTEGER NOT NULL,nonce TEXT NOT NULL,active INTEGER NOT NULL CHECK(active IN (0,1)));\
         CREATE TABLE user_facts(legacy_file_id TEXT NOT NULL,fact_key TEXT NOT NULL,fact_value TEXT NOT NULL,PRIMARY KEY(legacy_file_id,fact_key));\
         INSERT INTO user_facts VALUES('fixture-001','tag','retain');\
         INSERT INTO user_facts VALUES('fixture-001','note','synthetic user fact');",
    )?;
    authority.execute(
        "INSERT INTO t10_authority VALUES(1,?1,1,?2)",
        params![slot_id, e0.manifest_sha256],
    )?;
    authority.execute(
        "INSERT INTO t10_slots VALUES(?1,1,'PUBLISHED',?2)",
        params![slot_id, e0.manifest_sha256],
    )?;
    authority.execute(
        "INSERT INTO t10_slots VALUES(?1,1,'RETIRED',?2)",
        params![format!("E1-{suffix_name}"), e1.manifest_sha256],
    )?;
    drop(authority);
    Ok(NativeFixture {
        authority_path,
        slot_path: e0.path,
        manifest_path: e0.manifest_path,
        candidate_path: e1.path,
        candidate_manifest_path: e1.manifest_path,
        slot_id,
        generation: 1,
        manifest_sha256: e0.manifest_sha256,
        candidate_sha256: e1.manifest_sha256,
        manifest: e0.manifest,
        candidate_manifest: e1.manifest,
    })
}

fn retire_fixture(fixture: &NativeFixture) -> T10Result<()> {
    let mut conn = open_connection(&fixture.authority_path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next = format!("E1-{}", fixture.slot_id.trim_start_matches("E0-"));
    tx.execute(
        "UPDATE t10_authority SET slot_id=?1,generation=1,manifest_sha256=(SELECT manifest_sha256 FROM t10_slots WHERE slot_id=?1 AND generation=1) WHERE singleton=1",
        [&next],
    )?;
    tx.execute(
        "UPDATE t10_slots SET state='PUBLISHED' WHERE slot_id=?1 AND generation=1 AND state='RETIRED'",
        [&next],
    )?;
    tx.execute(
        "UPDATE t10_slots SET state='RETIRING' WHERE slot_id=?1 AND generation=?2 AND state='PUBLISHED'",
        params![fixture.slot_id, fixture.generation],
    )?;
    tx.commit()?;
    Ok(())
}

fn lease_state(conn: &Connection, fixture: &NativeFixture) -> rusqlite::Result<(i64, i64)> {
    conn.query_row(
        "SELECT COUNT(*),COALESCE(SUM(active),0) FROM t10_leases WHERE slot_id=?1 AND generation=?2",
        params![fixture.slot_id, fixture.generation],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

fn guarded_gc(fixture: &NativeFixture) -> T10Result<(bool, String)> {
    let mut conn = open_connection(&fixture.authority_path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let authority: (String, i64) = tx.query_row(
        "SELECT slot_id,generation FROM t10_authority WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if authority == (fixture.slot_id.clone(), fixture.generation) {
        tx.commit()?;
        return Ok((false, "PUBLISHED_AUTHORITY".into()));
    }
    let active: i64 = tx.query_row(
        "SELECT COUNT(*) FROM t10_leases WHERE slot_id=?1 AND generation=?2 AND active=1",
        params![fixture.slot_id, fixture.generation],
        |row| row.get(0),
    )?;
    if active > 0 {
        tx.commit()?;
        return Ok((false, "LIVE_PIN".into()));
    }
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM t10_slots WHERE slot_id=?1 AND generation=?2",
            params![fixture.slot_id, fixture.generation],
            |row| row.get(0),
        )
        .ok();
    match state.as_deref() {
        Some("RETIRING") => {
            tx.execute(
                "UPDATE t10_slots SET state='GC_ELIGIBLE' WHERE slot_id=?1 AND generation=?2 AND state='RETIRING'",
                params![fixture.slot_id, fixture.generation],
            )?;
        }
        Some("GC_ELIGIBLE") => {}
        Some("REMOVED") => {
            tx.commit()?;
            return Ok((false, "ALREADY_REMOVED".into()));
        }
        _ => {
            tx.commit()?;
            return Ok((false, "NOT_RETIRING".into()));
        }
    }
    tx.commit()?;
    let file_present = fixture.slot_path.exists();
    let manifest_present = fixture.manifest_path.exists();
    if file_present {
        fs::remove_file(&fixture.slot_path)?;
    }
    if manifest_present {
        fs::remove_file(&fixture.manifest_path)?;
    }
    conn.execute(
        "UPDATE t10_slots SET state='REMOVED' WHERE slot_id=?1 AND generation=?2 AND state='GC_ELIGIBLE'",
        params![fixture.slot_id, fixture.generation],
    )?;
    Ok((file_present && manifest_present, "REMOVED_AFTER_GUARD".into()))
}

fn release_after_observed_exit(
    fixture: &NativeFixture,
    lease_id: &str,
    pid: u32,
    nonce: &str,
    expected_generation: i64,
    process_exit_observed: bool,
) -> T10Result<bool> {
    if !process_exit_observed {
        return Ok(false);
    }
    let mut conn = open_connection(&fixture.authority_path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let exact: i64 = tx.query_row(
        "SELECT COUNT(*) FROM t10_leases WHERE lease_id=?1 AND slot_id=?2 AND generation=?3 AND process_id=?4 AND nonce=?5 AND active=1",
        params![lease_id, fixture.slot_id, expected_generation, i64::from(pid), nonce],
        |row| row.get(0),
    )?;
    if exact != 1 || expected_generation != fixture.generation {
        tx.commit()?;
        return Ok(false);
    }
    let changed = tx.execute(
        "UPDATE t10_leases SET active=0 WHERE lease_id=?1 AND slot_id=?2 AND generation=?3 AND process_id=?4 AND nonce=?5 AND active=1",
        params![lease_id, fixture.slot_id, expected_generation, i64::from(pid), nonce],
    )?;
    tx.commit()?;
    Ok(changed == 1)
}

fn new_lease_identity(label: &str) -> (String, String) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    (
        format!("{label}-{serial}"),
        format!("{}-{nonce}-{serial}", std::process::id()),
    )
}

struct ChildGuard {
    child: Child,
    protocol_lines: Receiver<String>,
    protocol_reader: Option<JoinHandle<()>>,
}

impl ChildGuard {
    fn spawn(args: &[String]) -> T10Result<Self> {
        let exe = std::env::current_exe()?;
        let mut child = Command::new(exe)
            .arg("--t10-native-child")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("child stdout unavailable")?;
        let (line_sender, protocol_lines) = mpsc::channel();
        let protocol_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if line_sender.send(line.trim().to_owned()).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = line_sender.send("T10_PROTOCOL_READ_ERROR".into());
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            protocol_lines,
            protocol_reader: Some(protocol_reader),
        })
    }

    fn id(&self) -> u32 {
        self.child.id()
    }

    fn read_line(&self) -> T10Result<String> {
        self.protocol_lines
            .recv_timeout(Duration::from_secs(15))
            .map_err(|_| "child protocol did not reach the next bounded barrier".into())
    }

    fn send_command(&mut self, command: &str) -> T10Result<()> {
        let stdin = self.child.stdin.as_mut().ok_or("child stdin unavailable")?;
        stdin.write_all(command.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;
        Ok(())
    }

    fn join_protocol_reader(&mut self) -> T10Result<()> {
        if let Some(reader) = self.protocol_reader.take() {
            reader
                .join()
                .map_err(|_| "child protocol reader thread panicked")?;
        }
        Ok(())
    }

    fn clean_exit(&mut self) -> T10Result<ExitStatus> {
        self.send_command("EXIT")?;
        self.child.stdin.take();
        let status = wait_bounded(&mut self.child, Duration::from_secs(15))?;
        self.join_protocol_reader()?;
        Ok(status)
    }

    fn kill_and_wait(&mut self) -> T10Result<ExitStatus> {
        if self.child.try_wait()?.is_none() {
            self.child.kill()?;
        }
        let status = self.child.wait()?;
        self.join_protocol_reader()?;
        Ok(status)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(reader) = self.protocol_reader.take() {
            let _ = reader.join();
        }
    }
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> T10Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("child process exceeded bounded exit timeout".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn child_args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn child_hold_lease(args: &[String]) -> T10Result<()> {
    if args.len() != 6 {
        return Err("hold-lease child argument count".into());
    }
    let authority_path = Path::new(&args[0]);
    let slot_path = Path::new(&args[1]);
    let slot_id = &args[2];
    let generation = args[3].parse::<i64>()?;
    let lease_id = &args[4];
    let nonce = &args[5];
    let pid = std::process::id();
    let mut authority = Connection::open(authority_path)?;
    authority.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = authority.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: (String, i64) = tx.query_row(
        "SELECT slot_id,generation FROM t10_authority WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM t10_slots WHERE slot_id=?1 AND generation=?2",
            params![slot_id, generation],
            |row| row.get(0),
        )
        .ok();
    if current != (slot_id.clone(), generation) || state.as_deref() != Some("PUBLISHED") {
        tx.commit()?;
        return Err("lease registration precondition rejected".into());
    }
    tx.execute(
        "INSERT INTO t10_leases VALUES(?1,?2,?3,?4,?5,1)",
        params![lease_id, slot_id, generation, i64::from(pid), nonce],
    )?;
    tx.commit()?;
    println!("T10_LEASE_REGISTERED {}", pid);
    io::stdout().flush()?;
    let mut command = String::new();
    io::stdin().read_line(&mut command)?;
    if command.trim() != "OPEN" {
        return Err("lease child expected OPEN barrier command".into());
    }
    let readonly = read_only_slot_tx(slot_path)?;
    let count: i64 = readonly.query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get(0))?;
    println!("T10_LEASE_OPENED {} {count}", pid);
    io::stdout().flush()?;
    command.clear();
    io::stdin().read_line(&mut command)?;
    if command.trim() != "EXIT" {
        return Err("lease child received unknown protocol command".into());
    }
    drop(readonly);
    println!("T10_LEASE_EXITED {}", pid);
    io::stdout().flush()?;
    Ok(())
}

static PIN_BUSY_HANDLER_ANNOUNCED: AtomicBool = AtomicBool::new(false);

fn pin_wait_busy_handler(count: i32) -> bool {
    if !PIN_BUSY_HANDLER_ANNOUNCED.swap(true, Ordering::SeqCst) {
        println!("T10_PIN_LOCK_WAIT");
        let _ = io::stdout().flush();
    }
    if count < 500 {
        thread::sleep(Duration::from_millis(10));
        true
    } else {
        false
    }
}

fn child_late_pin(args: &[String]) -> T10Result<()> {
    if args.len() != 5 {
        return Err("late-pin child argument count".into());
    }
    let authority_path = Path::new(&args[0]);
    let slot_id = &args[1];
    let generation = args[2].parse::<i64>()?;
    let lease_id = &args[3];
    let nonce = &args[4];
    let pid = std::process::id();
    let mut conn = Connection::open(authority_path)?;
    PIN_BUSY_HANDLER_ANNOUNCED.store(false, Ordering::SeqCst);
    conn.busy_handler(Some(pin_wait_busy_handler))?;
    println!("T10_PIN_ATTEMPTING {}", pid);
    io::stdout().flush()?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let current: (String, i64) = tx.query_row(
        "SELECT slot_id,generation FROM t10_authority WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM t10_slots WHERE slot_id=?1 AND generation=?2",
            params![slot_id, generation],
            |row| row.get(0),
        )
        .ok();
    let accepted = current == (slot_id.clone(), generation)
        && state.as_deref() == Some("PUBLISHED");
    if accepted {
        tx.execute(
            "INSERT INTO t10_leases VALUES(?1,?2,?3,?4,?5,1)",
            params![lease_id, slot_id, generation, i64::from(pid), nonce],
        )?;
    }
    tx.commit()?;
    println!(
        "T10_PIN_RESULT {}",
        json!({"accepted":accepted,"observed_state":state,"observed_authority":current})
    );
    io::stdout().flush()?;
    Ok(())
}

fn child_file_ops(args: &[String]) -> T10Result<()> {
    if args.len() != 3 {
        return Err("file-ops child argument count".into());
    }
    let source = PathBuf::from(&args[0]);
    let renamed = PathBuf::from(&args[1]);
    let replacement = PathBuf::from(&args[2]);
    let rename = fs::rename(&source, &renamed);
    let active = if rename.is_ok() { &renamed } else { &source };
    let replace = fs::rename(&replacement, active);
    let delete = fs::remove_file(active);
    let result = json!({
        "actor_pid":std::process::id(),
        "rename":io_observation("rename_while_readonly_handle_open",rename),
        "replace":io_observation("replace_while_readonly_handle_open",replace),
        "delete":io_observation("delete_while_readonly_handle_open",delete),
        "resulting_original_path_exists":source.exists(),
        "resulting_renamed_path_exists":renamed.exists()
    });
    println!("T10_FILE_OPS {result}");
    io::stdout().flush()?;
    Ok(())
}

fn child_pointer_barrier(args: &[String]) -> T10Result<()> {
    if args.len() != 2 {
        return Err("pointer-barrier child argument count".into());
    }
    let path = Path::new(&args[0]);
    let phase = args[1].as_str();
    if !matches!(phase, "before-commit" | "after-commit") {
        return Err("pointer-barrier phase".into());
    }
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE t10_pointer SET slot_id='E1',generation=2 WHERE singleton=1",
        [],
    )?;
    if phase == "after-commit" {
        tx.commit()?;
        println!("T10_POINTER_COMMITTED");
    } else {
        println!("T10_POINTER_BEFORE_COMMIT");
    }
    io::stdout().flush()?;
    let mut command = String::new();
    io::stdin().read_line(&mut command)?;
    if command.trim() != "EXIT" {
        return Err("pointer child received unknown protocol command".into());
    }
    Ok(())
}

pub fn run_child(args: &[String]) -> i32 {
    let result = match args.first().map(String::as_str) {
        Some("hold-lease") => child_hold_lease(&args[1..]),
        Some("late-pin") => child_late_pin(&args[1..]),
        Some("file-ops") => child_file_ops(&args[1..]),
        Some("pointer-barrier") => child_pointer_barrier(&args[1..]),
        _ => Err("unknown T10 child mode".into()),
    };
    if let Err(error) = result {
        eprintln!("T10 child failed: {}", error_details(&*error));
        return 2;
    }
    0
}

fn t10_01() -> T10Result<CaseReport> {
    let filesystem = std::env::var("T10_FILESYSTEM").unwrap_or_else(|_| "unreported".into());
    let rustc_version = std::env::var("T10_RUSTC_VERSION").ok();
    let cargo_version = std::env::var("T10_CARGO_VERSION").ok();
    let rustc_host = std::env::var("T10_RUSTC_HOST").ok();
    let sqlite = source_identity()?;
    let parity = lock_parity()?;
    let os = std::env::consts::OS;
    let filesystem_expected = match os {
        "windows" => filesystem.eq_ignore_ascii_case("NTFS"),
        "macos" => filesystem.to_ascii_lowercase().contains("apfs"),
        _ => false,
    };
    let tools_recorded = rustc_version.is_some() && cargo_version.is_some() && rustc_host.is_some();
    let lock_matches = parity["every_harness_registry_version_and_checksum_in_product_lock"] == true;
    let source_ok = sqlite["sqlite_fts5_compile_option"] == 1
        && sqlite["trigram_fts_smoke_matches"] == 1
        && sqlite["sqlite_system_override_variables"] == json!([])
        && sqlite["sqlite_source_id"].as_str().is_some_and(|value| !value.is_empty());
    let details = json!({
        "os":os,
        "source_sha":std::env::var("T10_SOURCE_SHA").unwrap_or_else(|_| "unreported".into()),
        "source_tree":std::env::var("T10_SOURCE_TREE").unwrap_or_else(|_| "unreported".into()),
        "workflow_run_id":std::env::var("T10_RUN_ID").unwrap_or_else(|_| "unreported".into()),
        "workflow_run_attempt":std::env::var("T10_RUN_ATTEMPT").unwrap_or_else(|_| "unreported".into()),
        "os_version":os_version(),
        "architecture":std::env::consts::ARCH,
        "filesystem_type":filesystem,
        "filesystem_type_matches_hosted_target":filesystem_expected,
        "rustc_version":rustc_version,
        "cargo_version":cargo_version,
        "rustc_host_target":rustc_host,
        "toolchain_recorded":tools_recorded,
        "sqlite":sqlite,
        "product_lock_parity":parity,
        "fixture_seed":"documents(128), deterministic harness generator",
        "temp_root_is_runner_temp_when_hosted":std::env::var_os("T10_SANDBOX_ROOT").is_some(),
        "source_paths_or_user_filenames_logged":false
    });
    if !lock_matches {
        return Ok(report("FAIL", details));
    }
    if !filesystem_expected || !tools_recorded || !source_ok || os_version().is_none() {
        return Ok(report("ENVIRONMENT_UNAVAILABLE", details));
    }
    Ok(report("PASS", details))
}

fn sidecar_observation(path: &Path) -> Value {
    match fs::metadata(path) {
        Ok(metadata) => json!({"exists":true,"bytes":metadata.len()}),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            json!({"exists":false,"bytes":0})
        }
        Err(error) => json!({
            "exists":true,
            "bytes":null,
            "error_kind":format!("{:?}",error.kind()),
            "raw_os_error":error.raw_os_error()
        }),
    }
}

fn directory_sync_value(attempts: &[DirectorySyncAttempt]) -> Value {
    serde_json::to_value(attempts).unwrap_or_else(|_| json!([]))
}

fn t10_02() -> T10Result<CaseReport> {
    let temp = TempRoot::new("seal-wal")?;
    let docs = documents(128);
    let slot = create_native_test_slot(&temp.path("sealed.sqlite"), "T10-SEALED", &docs)?;
    let verified = verify_seal(&slot)?;
    let reader = read_only_slot_tx(&slot.path)?;
    let rows = query_rows(&reader, "common", None, None)?;
    let documents: i64 = reader.query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get(0))?;
    let file_sha = file_sha256(&slot.path)?;
    let manifest_bytes = fs::read(&slot.manifest_path)?;
    let manifest_sha = sha256_bytes(&manifest_bytes);
    let wal = sidecar_observation(&suffix(&slot.path, "-wal"));
    let shm = sidecar_observation(&suffix(&slot.path, "-shm"));
    let directory_sync_ok = slot
        .directory_sync_attempts
        .iter()
        .all(|attempt| attempt.outcome == "SYNCED");
    let details = json!({
        "sealed_file_sha256":file_sha.0,
        "sealed_file_bytes":file_sha.1,
        "manifest_sha256":manifest_sha,
        "manifest_bytes_match_seal":manifest_sha == slot.manifest_sha256,
        "wal_checkpoint_truncate_completed":true,
        "wal_sidecar_after_close":wal,
        "shm_sidecar_after_close":shm,
        "sqlite_integrity_check":verified["integrity_check"],
        "fts5_integrity_check_before_checkpoint_completed":true,
        "read_only_reopen":verified["reopened_readonly"],
        "read_only_write_rejected":verified["readonly_write_rejected"],
        "document_count":documents,
        "frozen_global_effective_fts_query_rows":rows.len(),
        "file_sync_all_succeeded":true,
        "file_sync_ms":slot.file_sync_ms,
        "directory_sync_attempts":directory_sync_value(&slot.directory_sync_attempts),
        "directory_sync_complete":directory_sync_ok,
        "no_authority_published":true
    });
    drop(reader);
    let cleanup_ok = temp.cleanup().is_ok();
    let wal_cleared = wal["exists"] == false || wal["bytes"] == 0;
    let shm_cleared = shm["exists"] == false || shm["bytes"] == 0;
    let verified_ok = verified["integrity_check"] == "ok"
        && verified["reopened_readonly"] == true
        && verified["readonly_write_rejected"] == true
        && rows.len() > 0
        && documents == slot.manifest.document_count as i64
        && manifest_sha == slot.manifest_sha256
        && file_sha.0 == slot.manifest.sqlite_file_sha256
        && wal_cleared
        && shm_cleared;
    if !cleanup_ok || !verified_ok {
        return Ok(report("FAIL", json!({"observations":details,"cleanup_succeeded":cleanup_ok})));
    }
    if !directory_sync_ok {
        return Ok(report("ENVIRONMENT_UNAVAILABLE", json!({
            "observations":details,
            "durability_classification":"SQLite/file seal verified; directory-entry durability unavailable or failed and is not counted as native persistence PASS"
        })));
    }
    Ok(report("PASS", json!({"observations":details,"cleanup_succeeded":cleanup_ok})))
}

fn path_arg(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

fn spawn_hold_lease(
    fixture: &NativeFixture,
    label: &str,
) -> T10Result<(ChildGuard, String, String)> {
    let (lease_id, nonce) = new_lease_identity(label);
    let args = child_args(&[
        "hold-lease",
        &path_arg(&fixture.authority_path),
        &path_arg(&fixture.slot_path),
        &fixture.slot_id,
        &fixture.generation.to_string(),
        &lease_id,
        &nonce,
    ]);
    let child = ChildGuard::spawn(&args)?;
    let pid = child.id();
    let registered = child.read_line()?;
    let parts = registered.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 2 || parts[0] != "T10_LEASE_REGISTERED" {
        return Err("lease child did not reach registration barrier".into());
    }
    let child_pid = parts[1].parse::<u32>()?;
    if child_pid != pid {
        return Err("lease child registration PID mismatch".into());
    }
    Ok((child, lease_id, nonce))
}

fn open_lease_handle(child: &mut ChildGuard) -> T10Result<i64> {
    child.send_command("OPEN")?;
    let opened = child.read_line()?;
    let parts = opened.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "T10_LEASE_OPENED" {
        return Err("lease child did not reach read-only-open barrier".into());
    }
    let pid = parts[1].parse::<u32>()?;
    if pid != child.id() {
        return Err("lease child read-only-open PID mismatch".into());
    }
    let count = parts[2].parse::<i64>()?;
    if count <= 0 {
        return Err("lease child read-only transaction has no fixture rows".into());
    }
    Ok(count)
}

fn parse_protocol_json(line: &str, prefix: &str) -> T10Result<Value> {
    let payload = line
        .strip_prefix(prefix)
        .ok_or("child protocol message prefix mismatch")?
        .trim();
    Ok(serde_json::from_str(payload)?)
}

fn t10_03() -> T10Result<CaseReport> {
    let temp = TempRoot::new("readonly-pin-gc")?;
    let docs = documents(128);
    let fixture = fixture(&temp, "handle", &docs)?;
    let (mut child, lease_id, nonce) = spawn_hold_lease(&fixture, "pin-first")?;
    let pid = child.id();
    retire_fixture(&fixture)?;
    let guarded_attempt = guarded_gc(&fixture)?;
    let child_opened_count = open_lease_handle(&mut child)?;
    let lease_reader = read_only_slot_tx(&fixture.slot_path)?;
    let held_count: i64 = lease_reader.query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get(0))?;

    let probe_path = temp.path("handle-probe.sqlite");
    fs::copy(&fixture.slot_path, &probe_path)?;
    let probe_reader = read_only_slot_tx(&probe_path)?;
    let renamed_path = temp.path("handle-probe-renamed.sqlite");
    let replacement_path = temp.path("handle-probe-replacement.sqlite");
    fs::write(&replacement_path, b"disposable replacement bytes")?;
    let file_ops_args = child_args(&[
        "file-ops",
        &path_arg(&probe_path),
        &path_arg(&renamed_path),
        &path_arg(&replacement_path),
    ]);
    let mut file_ops_child = ChildGuard::spawn(&file_ops_args)?;
    let actor_pid = file_ops_child.id();
    let operation_line = file_ops_child.read_line()?;
    let operations = parse_protocol_json(&operation_line, "T10_FILE_OPS ")?;
    let operations_status = wait_bounded(&mut file_ops_child.child, Duration::from_secs(15))?;
    let handle_after_operations = probe_reader
        .query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get::<_, i64>(0))
        .ok();
    let held_handle_still_queries = handle_after_operations == Some(held_count);
    let fresh_open_count = open_readonly(&fixture.slot_path)
        .and_then(|conn| conn.query_row("SELECT COUNT(*) FROM slot_docs", [], |row| row.get::<_, i64>(0)))
        .ok();
    let fresh_open_still_works = fresh_open_count == Some(held_count);
    let original_slot_still_present = fixture.slot_path.exists();

    let child_exit = child.clean_exit()?;
    let released = release_after_observed_exit(
        &fixture,
        &lease_id,
        pid,
        &nonce,
        fixture.generation,
        child_exit.success(),
    )?;
    drop(probe_reader);
    drop(lease_reader);
    let gc_after_release = guarded_gc(&fixture)?;
    let second_gc = guarded_gc(&fixture)?;
    let slot_absent_after_gc = !fixture.slot_path.exists() && !fixture.manifest_path.exists();
    drop(file_ops_child);
    drop(child);
    let cleanup_ok = temp.cleanup().is_ok();

    let operation_rows = [&operations["rename"], &operations["replace"], &operations["delete"]];
    let every_operation_observed = operation_rows.iter().all(|entry| {
        entry["operation"].as_str().is_some()
            && (entry["outcome"] == "ALLOWED"
                || entry["outcome"] == "DENIED" && entry["raw_os_error"].as_i64().is_some())
    });
    let passed = held_count > 0
        && child_opened_count == held_count
        && guarded_attempt == (false, "LIVE_PIN".into())
        && operations_status.success()
        && operations["actor_pid"] == actor_pid
        && every_operation_observed
        && held_handle_still_queries
        && fresh_open_still_works
        && original_slot_still_present
        && child_exit.success()
        && released
        && gc_after_release.0
        && gc_after_release.1 == "REMOVED_AFTER_GUARD"
        && !second_gc.0
        && second_gc.1 == "ALREADY_REMOVED"
        && slot_absent_after_gc
        && cleanup_ok;
    let details = json!({
        "actual_readonly_sqlite_transaction_held_in_child":true,
        "lease_registered_before_reader_open":true,
        "gc_between_lease_registration_and_reader_open":{
            "removed":guarded_attempt.0,
            "decision":guarded_attempt.1,
            "child_opened_rows_after_gc_denial":child_opened_count
        },
        "lease_process_pid":pid,
        "lease_generation":fixture.generation,
        "sqlite_rows_seen_by_child":held_count,
        "filesystem_actor_pid":actor_pid,
        "native_rename_replace_delete_outcomes":operations,
        "filesystem_actor_exit_code":operations_status.code(),
        "existing_readonly_handle_query_after_file_operations":held_handle_still_queries,
        "fresh_reader_can_open_published_fixture_while_live_pin":fresh_open_still_works,
        "slot_path_present_while_pin_active":original_slot_still_present,
        "guarded_gc_while_live_pin":{
            "removed":guarded_attempt.0,
            "decision":guarded_attempt.1,
            "file_and_manifest_remain_accessible":original_slot_still_present
        },
        "child_clean_exit_observed":child_exit.success(),
        "lease_released_only_after_observed_exit":released,
        "gc_after_release":gc_after_release,
        "idempotent_second_gc":second_gc,
        "slot_and_manifest_absent_after_gc":slot_absent_after_gc,
        "cleanup_succeeded":cleanup_ok,
        "raw_file_operations_target_disposable_copy":true
    });
    if passed {
        Ok(report("PASS", details))
    } else {
        Ok(report("FAIL", details))
    }
}

fn t10_04() -> T10Result<CaseReport> {
    let temp = TempRoot::new("gc-first-late-pin")?;
    let docs = documents(128);
    let fixture = fixture(&temp, "gc-first", &docs)?;
    let mut conn = open_connection(&fixture.authority_path)?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let next_slot = format!("E1-gc-first");
    tx.execute(
        "UPDATE t10_authority SET slot_id=?1,generation=1,manifest_sha256=(SELECT manifest_sha256 FROM t10_slots WHERE slot_id=?1 AND generation=1) WHERE singleton=1",
        [&next_slot],
    )?;
    tx.execute(
        "UPDATE t10_slots SET state='PUBLISHED' WHERE slot_id=?1 AND generation=1",
        [&next_slot],
    )?;
    tx.execute(
        "UPDATE t10_slots SET state='GC_ELIGIBLE' WHERE slot_id=?1 AND generation=?2 AND state='PUBLISHED'",
        params![fixture.slot_id, fixture.generation],
    )?;
    let (lease_id, nonce) = new_lease_identity("gc-first-late-pin");
    let args = child_args(&[
        "late-pin",
        &path_arg(&fixture.authority_path),
        &fixture.slot_id,
        &fixture.generation.to_string(),
        &lease_id,
        &nonce,
    ]);
    let mut child = ChildGuard::spawn(&args)?;
    let attempting = child.read_line()?;
    let busy_wait = child.read_line()?;
    tx.commit()?;
    let pin_result_line = child.read_line()?;
    let pin_result = parse_protocol_json(&pin_result_line, "T10_PIN_RESULT ")?;
    let child_status = wait_bounded(&mut child.child, Duration::from_secs(15))?;
    let reopened = open_connection(&fixture.authority_path)?;
    let lease_rows: i64 = reopened.query_row(
        "SELECT COUNT(*) FROM t10_leases WHERE lease_id=?1",
        [&lease_id],
        |row| row.get(0),
    )?;
    let late_pin_denied = pin_result["accepted"] == false
        && pin_result["observed_state"] == "GC_ELIGIBLE"
        && pin_result["observed_authority"] == next_slot
        && lease_rows == 0;
    drop(reopened);
    let gc = guarded_gc(&fixture)?;
    let cleanup_file_absent = !fixture.slot_path.exists() && !fixture.manifest_path.exists();
    drop(child);
    drop(conn);
    let cleanup_ok = temp.cleanup().is_ok();
    let passed = attempting.starts_with("T10_PIN_ATTEMPTING ")
        && busy_wait == "T10_PIN_LOCK_WAIT"
        && pin_result["accepted"] == false
        && child_status.success()
        && late_pin_denied
        && gc.0
        && gc.1 == "REMOVED_AFTER_GUARD"
        && cleanup_file_absent
        && cleanup_ok;
    Ok(report(
        if passed { "EXPECTED_REJECTION" } else { "FAIL" },
        json!({
            "gc_retirement_transaction_held_before_pin_attempt":true,
            "late_pin_process_observed_sqlite_busy_barrier":busy_wait == "T10_PIN_LOCK_WAIT",
            "late_pin_attempt_marker":attempting,
            "late_pin_result":pin_result,
            "late_lease_rows":lease_rows,
            "late_pin_was_denied_before_file_reclamation":late_pin_denied,
            "child_exit_code":child_status.code(),
            "gc_after_late_pin_rejection":gc,
            "slot_and_manifest_removed_after_guard":cleanup_file_absent,
            "cleanup_succeeded":cleanup_ok,
            "race_scheduling":"cross-process SQLite busy callback barrier; no sleep used to order actors"
        }),
    ))
}

fn t10_05() -> T10Result<CaseReport> {
    let temp = TempRoot::new("lease-exit-recovery")?;
    let docs = documents(128);
    let fixture = fixture(&temp, "lease-recovery", &docs)?;
    let (mut child, lease_id, nonce) = spawn_hold_lease(&fixture, "exit-recovery")?;
    let pid = child.id();
    let child_opened_count = open_lease_handle(&mut child)?;
    let before_unproven = release_after_observed_exit(
        &fixture,
        &lease_id,
        pid,
        &nonce,
        fixture.generation,
        false,
    )?;
    retire_fixture(&fixture)?;
    let active_before = {
        let conn = open_connection(&fixture.authority_path)?;
        lease_state(&conn, &fixture)?.1
    };
    let killed = child.kill_and_wait()?;
    let exit_observed = child.child.try_wait()?.is_some();
    let wrong_generation_rejected = !release_after_observed_exit(
        &fixture,
        &lease_id,
        pid,
        &nonce,
        fixture.generation + 1,
        exit_observed,
    )?;
    let wrong_nonce_rejected = !release_after_observed_exit(
        &fixture,
        &lease_id,
        pid,
        "stale-pid-reuse-token",
        fixture.generation,
        exit_observed,
    )?;
    let active_after_negative_controls = {
        let conn = open_connection(&fixture.authority_path)?;
        lease_state(&conn, &fixture)?.1
    };
    let released = release_after_observed_exit(
        &fixture,
        &lease_id,
        pid,
        &nonce,
        fixture.generation,
        exit_observed,
    )?;
    let active_after = {
        let conn = open_connection(&fixture.authority_path)?;
        lease_state(&conn, &fixture)?.1
    };
    let gc = guarded_gc(&fixture)?;
    let gc_twice = guarded_gc(&fixture)?;
    let slot_absent = !fixture.slot_path.exists() && !fixture.manifest_path.exists();
    drop(child);
    let cleanup_ok = temp.cleanup().is_ok();
    let passed = !before_unproven
        && exit_observed
        && active_before == 1
        && wrong_generation_rejected
        && wrong_nonce_rejected
        && active_after_negative_controls == 1
        && released
        && active_after == 0
        && gc.0
        && !gc_twice.0
        && gc_twice.1 == "ALREADY_REMOVED"
        && slot_absent
        && cleanup_ok;
    Ok(report(
        if passed { "PASS" } else { "FAIL" },
        json!({
            "child_pid":pid,
            "child_readonly_rows_before_termination":child_opened_count,
            "process_termination":"Child::kill followed by Child::wait; native process termination only",
            "child_exit_status_code":killed.code(),
            "child_exit_observed_before_lease_recovery":exit_observed,
            "recovery_without_proved_exit_accepted":before_unproven,
            "active_lease_rows_before_exit":active_before,
            "wrong_generation_reclaim_rejected":wrong_generation_rejected,
            "wrong_nonce_pid_reuse_reclaim_rejected":wrong_nonce_rejected,
            "active_lease_rows_after_negative_controls":active_after_negative_controls,
            "exact_generation_nonce_pid_reclaim_after_exit":released,
            "active_lease_rows_after_recovery":active_after,
            "guarded_gc":gc,
            "idempotent_second_gc":gc_twice,
            "slot_and_manifest_absent":slot_absent,
            "power_loss_durability_proven":false,
            "cleanup_succeeded":cleanup_ok
        }),
    ))
}

fn expectation<'a>(
    manifest: &'a crate::db::SlotManifest,
    manifest_sha256: &'a str,
) -> ManifestExpectation<'a> {
    ManifestExpectation {
        format_version: manifest.format_version,
        slot_id: &manifest.slot_id,
        document_count: manifest.document_count,
        corpus_sha256: &manifest.corpus_sha256,
        sqlite_file_sha256: &manifest.sqlite_file_sha256,
        sqlite_file_bytes: manifest.sqlite_file_bytes,
        source_id: &manifest.source_id,
        root_revision: manifest.root_revision,
        source_revision: manifest.source_revision,
        sealed_stage: &manifest.sealed_stage,
        manifest_sha256,
    }
}

fn user_facts(conn: &Connection) -> rusqlite::Result<Vec<(String, String, String)>> {
    let mut statement = conn.prepare(
        "SELECT legacy_file_id,fact_key,fact_value FROM user_facts ORDER BY legacy_file_id,fact_key",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect()
}

fn pointer_barrier_case(
    temp: &TempRoot,
    fixture: &NativeFixture,
    phase: &str,
    expected_slot: &str,
) -> T10Result<Value> {
    let pointer_path = temp.path(&format!("pointer-{phase}.sqlite"));
    let mut conn = open_connection(&pointer_path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.busy_timeout(Duration::from_millis(PRODUCT_BUSY_TIMEOUT_MS))?;
    conn.execute_batch(
        "CREATE TABLE t10_pointer(singleton INTEGER PRIMARY KEY CHECK(singleton=1),slot_id TEXT NOT NULL,generation INTEGER NOT NULL);\
         CREATE TABLE t10_pointer_slots(slot_id TEXT NOT NULL,generation INTEGER NOT NULL,manifest_sha256 TEXT NOT NULL,PRIMARY KEY(slot_id,generation));\
         CREATE TABLE user_facts(legacy_file_id TEXT NOT NULL,fact_key TEXT NOT NULL,fact_value TEXT NOT NULL,PRIMARY KEY(legacy_file_id,fact_key));\
         INSERT INTO t10_pointer VALUES(1,'E0',1);\
         INSERT INTO user_facts VALUES('pointer-fixture','tag','stable');",
    )?;
    conn.execute(
        "INSERT INTO t10_pointer_slots VALUES('E0',1,?1)",
        [&fixture.manifest_sha256],
    )?;
    conn.execute(
        "INSERT INTO t10_pointer_slots VALUES('E1',2,?1)",
        [&fixture.candidate_sha256],
    )?;
    drop(conn);

    let args = child_args(&["pointer-barrier", &path_arg(&pointer_path), phase]);
    let mut child = ChildGuard::spawn(&args)?;
    let marker = child.read_line()?;
    let expected_marker = if phase == "after-commit" {
        "T10_POINTER_COMMITTED"
    } else {
        "T10_POINTER_BEFORE_COMMIT"
    };
    let killed = child.kill_and_wait()?;
    let child_reaped = child.child.try_wait()?.is_some();
    drop(child);

    let reopened = open_connection(&pointer_path)?;
    let pointer: (String, i64) = reopened.query_row(
        "SELECT slot_id,generation FROM t10_pointer WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let pointer_hash: String = reopened.query_row(
        "SELECT manifest_sha256 FROM t10_pointer_slots WHERE slot_id=?1 AND generation=?2",
        params![pointer.0, pointer.1],
        |row| row.get(0),
    )?;
    let facts = user_facts(&reopened)?;
    let (target_path, target_manifest, target_manifest_digest, target_metadata) =
        if pointer.0 == "E0" {
            (
                &fixture.slot_path,
                &fixture.manifest_path,
                fixture.manifest_sha256.as_str(),
                &fixture.manifest,
            )
        } else {
            (
                &fixture.candidate_path,
                &fixture.candidate_manifest_path,
                fixture.candidate_sha256.as_str(),
                &fixture.candidate_manifest,
            )
        };
    let file_digest = file_sha256(target_path)?;
    let manifest_digest = sha256_bytes(&fs::read(target_manifest)?);
    let seal = verify_manifest_file(
        target_path,
        target_manifest,
        &expectation(target_metadata, target_manifest_digest),
    )
    .is_ok();
    let expected_tuple = if expected_slot == "E0" {
        ("E0".to_owned(), 1)
    } else {
        ("E1".to_owned(), 2)
    };
    let facts_unchanged = facts == vec![("pointer-fixture".into(), "tag".into(), "stable".into())];
    let pointer_and_slot_match = pointer == expected_tuple
        && pointer_hash == target_manifest_digest
        && file_digest.0 == target_metadata.sqlite_file_sha256
        && manifest_digest == target_manifest_digest
        && seal
        && facts_unchanged;
    Ok(json!({
        "phase":phase,
        "child_barrier_marker":marker,
        "expected_barrier_marker":expected_marker,
        "child_termination":"Child::kill + Child::wait; process boundary only",
        "child_exit_status_code":killed.code(),
        "child_reaped":child_reaped,
        "reopened_pointer":pointer,
        "expected_pointer":expected_tuple,
        "pointer_matches_sealed_slot":pointer_and_slot_match,
        "pointer_manifest_sha256":pointer_hash,
        "target_sqlite_sha256":file_digest.0,
        "target_manifest_sha256":manifest_digest,
        "target_seal_valid":seal,
        "user_facts_after_restart":facts,
        "user_facts_unchanged":facts_unchanged,
        "valid":marker == expected_marker && child_reaped && pointer_and_slot_match
    }))
}

fn t10_06() -> T10Result<CaseReport> {
    let temp = TempRoot::new("manifest-tamper-reconcile")?;
    let docs = documents(128);
    let fixture = fixture(&temp, "manifest", &docs)?;
    let authority_before = {
        let conn = open_connection(&fixture.authority_path)?;
        let pointer: (String, i64) = conn.query_row(
            "SELECT slot_id,generation FROM t10_authority WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        (pointer, user_facts(&conn)?)
    };
    let original_manifest = fs::read(&fixture.manifest_path)?;
    let original_slot_digest = file_sha256(&fixture.slot_path)?.0;
    let valid_before = verify_manifest_file(
        &fixture.slot_path,
        &fixture.manifest_path,
        &expectation(&fixture.manifest, &fixture.manifest_sha256),
    )
    .is_ok();

    let mut forged: Value = serde_json::from_slice(&original_manifest)?;
    forged["slot_id"] = json!("T10-FORGED-SLOT");
    fs::write(&fixture.manifest_path, serde_json::to_vec(&forged)?)?;
    fs::File::open(&fixture.manifest_path)?.sync_all()?;
    let manifest_tamper_rejected = verify_manifest_file(
        &fixture.slot_path,
        &fixture.manifest_path,
        &expectation(&fixture.manifest, &fixture.manifest_sha256),
    )
    .is_err();
    fs::write(&fixture.manifest_path, &original_manifest)?;
    fs::File::open(&fixture.manifest_path)?.sync_all()?;
    let restored_manifest_valid = verify_manifest_file(
        &fixture.slot_path,
        &fixture.manifest_path,
        &expectation(&fixture.manifest, &fixture.manifest_sha256),
    )
    .is_ok();

    let wrong_root_expectation = ManifestExpectation {
        root_revision: fixture.manifest.root_revision + 1,
        ..expectation(&fixture.manifest, &fixture.manifest_sha256)
    };
    let wrong_root_rejected = verify_manifest_file(
        &fixture.slot_path,
        &fixture.manifest_path,
        &wrong_root_expectation,
    )
    .is_err();
    let wrong_source_expectation = ManifestExpectation {
        source_revision: fixture.manifest.source_revision + 1,
        ..expectation(&fixture.manifest, &fixture.manifest_sha256)
    };
    let wrong_source_rejected = verify_manifest_file(
        &fixture.slot_path,
        &fixture.manifest_path,
        &wrong_source_expectation,
    )
    .is_err();

    let missing_slot = temp.path("intentionally-missing-slot.sqlite");
    let missing_slot_rejected = verify_manifest_file(
        &missing_slot,
        &fixture.manifest_path,
        &expectation(&fixture.manifest, &fixture.manifest_sha256),
    )
    .is_err();
    let corrupt_slot = temp.path("intentionally-corrupt-slot.sqlite");
    let mut corrupt_bytes = fs::read(&fixture.slot_path)?;
    if let Some(first) = corrupt_bytes.first_mut() {
        *first ^= 0x80;
    }
    fs::write(&corrupt_slot, corrupt_bytes)?;
    fs::File::open(&corrupt_slot)?.sync_all()?;
    let corrupt_slot_rejected = verify_manifest_file(
        &corrupt_slot,
        &fixture.manifest_path,
        &expectation(&fixture.manifest, &fixture.manifest_sha256),
    )
    .is_err();

    let pre_commit = pointer_barrier_case(&temp, &fixture, "before-commit", "E0")?;
    let post_commit = pointer_barrier_case(&temp, &fixture, "after-commit", "E1")?;
    let authority_after = {
        let conn = open_connection(&fixture.authority_path)?;
        let pointer: (String, i64) = conn.query_row(
            "SELECT slot_id,generation FROM t10_authority WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        (pointer, user_facts(&conn)?)
    };
    let final_slot_digest = file_sha256(&fixture.slot_path)?.0;
    let cleanup_ok = temp.cleanup().is_ok();
    let barrier_ok = pre_commit["valid"] == true && post_commit["valid"] == true;
    let unchanged = authority_before == authority_after && original_slot_digest == final_slot_digest;
    let passed = valid_before
        && manifest_tamper_rejected
        && restored_manifest_valid
        && wrong_root_rejected
        && wrong_source_rejected
        && missing_slot_rejected
        && corrupt_slot_rejected
        && barrier_ok
        && unchanged
        && cleanup_ok;
    Ok(report(
        if passed { "PASS" } else { "FAIL" },
        json!({
            "valid_seal_before_tamper":valid_before,
            "actual_manifest_file_slot_id_tamper_rejected":manifest_tamper_rejected,
            "original_manifest_bytes_restored_and_valid":restored_manifest_valid,
            "root_revision_mismatch_rejected":wrong_root_rejected,
            "source_revision_mismatch_rejected":wrong_source_rejected,
            "missing_slot_rejected_read_only":missing_slot_rejected,
            "corrupt_slot_rejected_read_only":corrupt_slot_rejected,
            "pointer_before_commit_process_exit":pre_commit,
            "pointer_after_commit_process_exit":post_commit,
            "process_exit_is_not_power_loss_proof":true,
            "authority_and_synthetic_user_facts_before":authority_before,
            "authority_and_synthetic_user_facts_after":authority_after,
            "slot_digest_unchanged":original_slot_digest == final_slot_digest,
            "mutable_files_fts_fallback_used":false,
            "cleanup_succeeded":cleanup_ok
        }),
    ))
}

fn readonly_permission_probe(path: &Path) -> T10Result<Value> {
    fs::write(path, b"disposable permission probe")?;
    fs::File::open(path)?.sync_all()?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)?;
    let attempt = OpenOptions::new().append(true).open(path).and_then(|mut file| {
        file.write_all(b"x")?;
        file.sync_all()
    });
    let observed = io_observation("write_to_readonly_fixture", attempt);
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    Ok(observed)
}

fn t10_07() -> T10Result<CaseReport> {
    let temp = TempRoot::new("filesystem-constraints")?;
    let permission = readonly_permission_probe(&temp.path("readonly-probe.bin"))?;
    let docs = documents(64);
    let slot = create_native_test_slot(&temp.path("directory-sync.sqlite"), "T10-DIRSYNC", &docs)?;
    let dir_sync_ok = slot
        .directory_sync_attempts
        .iter()
        .all(|attempt| attempt.outcome == "SYNCED");
    let permission_denied = permission["outcome"] == "DENIED"
        && matches!(permission["error_kind"].as_str(), Some("PermissionDenied"));
    let cleanup_ok = temp.cleanup().is_ok();
    let details = json!({
        "readonly_permission_api_observation":permission,
        "permission_denial_observed":permission_denied,
        "file_sync_all_succeeded_for_disposable_slot":true,
        "directory_sync_attempts":directory_sync_value(&slot.directory_sync_attempts),
        "directory_sync_complete":dir_sync_ok,
        "disk_full":"ENVIRONMENT_UNAVAILABLE",
        "disk_full_reason":"No disposable quota or bounded test volume is available; the hosted runner's disk was not filled",
        "power_loss_durability_proven":false,
        "cleanup_succeeded":cleanup_ok
    });
    if !cleanup_ok || !permission_denied {
        return Ok(report("FAIL", details));
    }
    Ok(report("ENVIRONMENT_UNAVAILABLE", details))
}

fn t10_08() -> T10Result<CaseReport> {
    let temp = TempRoot::new("pool-busy-teardown")?;
    let path = temp.path("pool.sqlite");
    drop(open_connection(&path)?);
    let pool = open_product_pool(&path)?;
    let pool_size = pool.max_size();
    let mut connections = Vec::new();
    for _ in 0..PRODUCT_POOL_SIZE {
        connections.push(pool_get(&pool)?);
    }
    let settings = connections
        .iter()
        .map(|conn| query_pool_settings(conn).map_err(|error| Box::new(error) as Box<dyn Error>))
        .collect::<Result<Vec<_>, _>>()?;
    let busy_values = settings.iter().map(|entry| entry.2).collect::<Vec<_>>();
    let busy_unchanged = busy_values.len() == PRODUCT_POOL_SIZE as usize
        && busy_values
            .iter()
            .all(|value| *value == PRODUCT_BUSY_TIMEOUT_MS as i64);
    let pool_unchanged = pool_size == PRODUCT_POOL_SIZE;
    drop(connections);
    drop(pool);
    let cleanup_ok = temp.cleanup().is_ok();
    let passed = pool_unchanged && busy_unchanged && cleanup_ok;
    Ok(report(
        if passed { "PASS" } else { "FAIL" },
        json!({
            "pool_max_size":pool_size,
            "expected_pool_max_size":PRODUCT_POOL_SIZE,
            "opened_connection_count":settings.len(),
            "busy_timeout_values_ms":busy_values,
            "expected_busy_timeout_ms":PRODUCT_BUSY_TIMEOUT_MS,
            "pool_size_unchanged":pool_unchanged,
            "busy_timeout_unchanged":busy_unchanged,
            "connections_dropped_before_temp_cleanup":true,
            "cleanup_succeeded":cleanup_ok
        }),
    ))
}

pub fn run() -> i32 {
    let source_sha = std::env::var("T10_SOURCE_SHA").unwrap_or_else(|_| "unreported".into());
    let source_tree = std::env::var("T10_SOURCE_TREE").unwrap_or_else(|_| "unreported".into());
    let run_id = std::env::var("T10_RUN_ID").unwrap_or_else(|_| "unreported".into());
    let run_attempt = std::env::var("T10_RUN_ATTEMPT").unwrap_or_else(|_| "unreported".into());
    let event_sha = std::env::var("GITHUB_SHA").unwrap_or_else(|_| "unreported".into());
    println!(
        "{}",
        json!({
            "record_type":"t10_run_start",
            "mode":"--t10-native",
            "run_id":run_id,
            "run_attempt":run_attempt,
            "source_sha":source_sha,
            "source_tree":source_tree,
            "event_sha":event_sha,
            "source_sha_matches_push_event":source_sha != "unreported" && source_sha == event_sha,
            "os":std::env::consts::OS,
            "architecture":std::env::consts::ARCH,
            "filesystem_type":std::env::var("T10_FILESYSTEM").unwrap_or_else(|_| "unreported".into()),
            "hosted_native_target":hosted_native_target(),
            "power_loss_test_performed":false,
            "temp_root_isolated":std::env::var_os("T10_SANDBOX_ROOT").is_some()
        })
    );

    let mut failed = false;
    let mut unavailable = false;
    for (case_id, result) in [
        ("T10-01", run_case("T10-01", t10_01)),
        ("T10-02", run_case("T10-02", t10_02)),
        ("T10-03", run_case("T10-03", t10_03)),
        ("T10-04", run_case("T10-04", t10_04)),
        ("T10-05", run_case("T10-05", t10_05)),
        ("T10-06", run_case("T10-06", t10_06)),
        ("T10-07", run_case("T10-07", t10_07)),
        ("T10-08", run_case("T10-08", t10_08)),
    ] {
        let _ = case_id;
        failed |= result.0;
        unavailable |= result.1;
    }
    let overall = if failed {
        "FAIL"
    } else if unavailable {
        "T10_NATIVE_GATE_BLOCKED"
    } else {
        "T10_NATIVE_TESTS_EXECUTED_PASS"
    };
    println!(
        "{}",
        json!({
            "record_type":"t10_run_summary",
            "status":overall,
            "failed_cases_present":failed,
            "environment_unavailable_cases_present":unavailable,
            "power_loss_durability_proven":false,
            "exit_code":if failed { 1 } else { 0 }
        })
    );
    if failed { 1 } else { 0 }
}
