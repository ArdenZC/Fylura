//! Narrow diagnostics for the disposable hosted Global Index qualification.
//! This is inert unless the binary was explicitly built with `native-qa` and
//! the runner supplies a task-owned trace path.

#[cfg(feature = "native-qa")]
pub(crate) fn record(event: &str) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let Some(path) = std::env::var_os("ZC_GLOBAL_INDEX_QA_TRACE") else {
        return;
    };
    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("native_qa coordinator_trace_open_error: {error}");
            return;
        }
    };
    if let Err(error) = writeln!(file, "{event}") {
        eprintln!("native_qa coordinator_trace_write_error: {error}");
    }
}

#[cfg(feature = "native-qa")]
pub(crate) fn record_error(scope: &str, error: &str) {
    let detail = error
        .chars()
        .map(|character| match character {
            '\n' | '\r' | '\t' => ' ',
            character => character,
        })
        .collect::<String>();
    record(&format!("{scope}_error\t{detail}"));
}

#[cfg(not(feature = "native-qa"))]
pub(crate) fn record(_event: &str) {}
