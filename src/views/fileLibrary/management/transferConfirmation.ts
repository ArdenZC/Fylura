import type { TransferPreview } from "../../../api/fileManagementApi";

export interface TransferConfirmationDiagnostics {
  busy: boolean;
  planId: string | null;
  planCurrent: boolean;
  rows: Array<{ skipped: boolean; preview: boolean; error: boolean }>;
  readyCount: number;
  ready: boolean;
}

export function getTransferConfirmationDiagnostics(input: {
  busy: boolean;
  plan: TransferPreview | null;
  planEpoch: number | null;
  currentEpoch: number;
}): TransferConfirmationDiagnostics {
  const { busy, plan } = input;
  const planCurrent = !!plan && input.planEpoch === input.currentEpoch;
  const rows = plan?.rows.map((row) => ({
    skipped: row.skipped,
    preview: !!row.preview,
    error: !!row.error,
  })) ?? [];
  const readyCount = plan?.rows.filter((row) => !row.skipped && row.preview && !row.error).length ?? 0;
  const ready = !busy && planCurrent && readyCount > 0 && plan!.rows.every(
    (row) => row.skipped || (!!row.preview && !row.error),
  );
  return { busy, planId: plan?.id ?? null, planCurrent, rows, readyCount, ready };
}
