use super::*;

/// One post-persistence wake for the bounded operation's roots, including when
/// a later journal write fails after an earlier item has already committed.
/// The scheduler retains sole authority over leases, backoff and exhaustion.
pub(super) fn with_reconciliation_wake<R: Runtime, T>(
    app: &AppHandle<R>,
    db: &Database,
    paths: &[String],
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let root_ids = db
        .managed_root_ids_for_operation_paths(paths)
        .map_err(|error| error.to_string())?;
    let result = operation();
    if !root_ids.is_empty() {
        let jobs = app.state::<crate::scanner::ScanJobManager>();
        let dedupe_jobs = app.state::<crate::dedupe::DedupeJobManager>();
        if let Err(error) = crate::scanner::schedule_watcher_reconciliation_roots(
            app.clone(),
            db.clone(),
            jobs.inner().clone(),
            dedupe_jobs.inner().clone(),
            root_ids,
        ) {
            // Do not turn an already committed mutation into an apparent failure
            // that invites a second execution. Durable needs remains truthful.
            eprintln!("Unable to schedule reconciliation after user operation: {error}");
        }
    }
    result
}
