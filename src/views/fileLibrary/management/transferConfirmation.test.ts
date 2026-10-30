import { describe, expect, it } from "vitest";
import { getTransferConfirmationDiagnostics } from "./transferConfirmation";
import type { TransferPreview } from "../../../api/fileManagementApi";

function plan(rows: TransferPreview["rows"]): TransferPreview {
  return { id: "preview-1", destinationDirectory: "E:/fixture/target", rows };
}

const preview = {} as NonNullable<TransferPreview["rows"][number]["preview"]>;

describe("transfer confirmation readiness", () => {
  it("enables confirmation only for a current, complete, error-free plan", () => {
    const diagnostics = getTransferConfirmationDiagnostics({
      busy: false,
      plan: plan([{ ticketId: "ticket-1", preview, error: null, skipped: false }]),
      planEpoch: 7,
      currentEpoch: 7,
    });
    expect(diagnostics).toMatchObject({ planId: "preview-1", planCurrent: true, readyCount: 1, ready: true });
    expect(diagnostics.rows).toEqual([{ skipped: false, preview: true, error: false }]);
  });

  it("keeps confirmation disabled while busy, stale, or any included row is incomplete", () => {
    const valid = plan([
      { ticketId: "ticket-1", preview, error: null, skipped: false },
      { ticketId: "ticket-2", preview: null, error: null, skipped: false },
    ]);
    expect(getTransferConfirmationDiagnostics({ busy: true, plan: valid, planEpoch: 7, currentEpoch: 7 }).ready).toBe(false);
    expect(getTransferConfirmationDiagnostics({ busy: false, plan: valid, planEpoch: 6, currentEpoch: 7 }).ready).toBe(false);
    expect(getTransferConfirmationDiagnostics({ busy: false, plan: valid, planEpoch: 7, currentEpoch: 7 }).ready).toBe(false);
  });

  it("allows an explicitly skipped failed row while requiring at least one ready row", () => {
    const readyAndSkipped = plan([
      { ticketId: "ticket-1", preview, error: null, skipped: false },
      { ticketId: "ticket-2", preview: null, error: "source_changed", skipped: true },
    ]);
    expect(getTransferConfirmationDiagnostics({ busy: false, plan: readyAndSkipped, planEpoch: 2, currentEpoch: 2 }).ready).toBe(true);
    const onlySkipped = plan([{ ticketId: "ticket-2", preview: null, error: "source_changed", skipped: true }]);
    expect(getTransferConfirmationDiagnostics({ busy: false, plan: onlySkipped, planEpoch: 2, currentEpoch: 2 }).ready).toBe(false);
  });
});
