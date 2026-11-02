import { useOperationQueueStore } from "../../store/useOperationQueueStore";

// Presentation and navigation only. Organization Plan remains the review and
// execution authority; this adapter never selects or executes operations.
export function useOrganizationOperationProgress() {
  return useOperationQueueStore((state) => state.operationProgress);
}
export function requestOrganizationHistoryBatch(batchId: string | null) {
  useOperationQueueStore.setState({ requestedHistoryBatchId: batchId });
}
