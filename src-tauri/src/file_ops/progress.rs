use super::{
    OperationProgressEmitter, OperationProgressPayload, OPERATION_PROGRESS_BATCH_SIZE,
    OPERATION_PROGRESS_EMIT_INTERVAL,
};
use std::time::Instant;

pub(super) struct OperationProgressBuffer {
    kind: &'static str,
    batch_id: String,
    total: u64,
    last_emit_at: Instant,
    processed_since_emit: u64,
}

impl OperationProgressBuffer {
    pub(super) fn new(kind: &'static str, batch_id: String, total: u64) -> Self {
        Self {
            kind,
            batch_id,
            total,
            last_emit_at: Instant::now(),
            processed_since_emit: 0,
        }
    }

    pub(super) fn record(
        &mut self,
        emitter: &impl OperationProgressEmitter,
        processed: u64,
        current_path: String,
    ) {
        self.processed_since_emit += 1;
        let now = Instant::now();
        if processed == self.total
            || processed.is_multiple_of(OPERATION_PROGRESS_BATCH_SIZE)
            || self.processed_since_emit >= OPERATION_PROGRESS_BATCH_SIZE
            || now.duration_since(self.last_emit_at) >= OPERATION_PROGRESS_EMIT_INTERVAL
        {
            emitter.emit_progress(OperationProgressPayload {
                kind: self.kind.to_string(),
                batch_id: self.batch_id.clone(),
                processed,
                total: self.total,
                current_path,
                phase: None,
            });
            self.last_emit_at = now;
            self.processed_since_emit = 0;
        }
    }
}
