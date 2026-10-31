import type {
  TransferAction,
  TransferPreview,
  TransferTicket,
} from "../../../api/fileManagementApi";
import type { BrowseEntryRef } from "../../../types/fileWorkspace";

export interface TransferOrchestrationRequest {
  tickets: TransferTicket[];
  action: TransferAction;
  destinationDirectory: string;
  destinationEntry?: BrowseEntryRef;
  newName?: string;
  skip: string[];
  keepBoth: string[];
}

export type TransferOrchestrationOutcome<TResult> =
  | { kind: "stale" }
  | { kind: "conflicts"; preview: TransferPreview; ticketIds: string[] }
  | { kind: "failed"; preview: TransferPreview; errors: string[] }
  | { kind: "all-skipped"; preview: TransferPreview }
  | { kind: "executed"; preview: TransferPreview; result: TResult };

export interface TransferOrchestrationPorts<TResult> {
  preview: (request: {
    ticketIds: string[];
    action: TransferAction;
    destinationDirectory: string;
    destinationEntry?: BrowseEntryRef;
    newName?: string;
    skip: string[];
    keepBoth: string[];
  }) => Promise<TransferPreview>;
  isCurrent: () => boolean;
  beforeExecute: () => Promise<void>;
  execute: (preview: TransferPreview) => Promise<TResult>;
}

const TARGET_EXISTS = "transfer_target_exists";

function isTargetConflict(error: string | null) {
  return error?.includes(TARGET_EXISTS) ?? false;
}

/**
 * The single UI-side transfer path: ask the authoritative backend for a fresh
 * plan, pause only for real target conflicts, and execute that exact plan only
 * while its source/destination context is still current.
 */
export async function orchestrateTransfer<TResult>(
  request: TransferOrchestrationRequest,
  ports: TransferOrchestrationPorts<TResult>,
): Promise<TransferOrchestrationOutcome<TResult>> {
  const createDirectory = request.action === "create_directory";
  if ((!request.tickets.length && !createDirectory) || !ports.isCurrent()) return { kind: "stale" };

  const preview = await ports.preview({
    ticketIds: request.tickets.map((ticket) => ticket.id),
    action: request.action,
    destinationDirectory: request.destinationDirectory,
    destinationEntry: request.destinationEntry,
    ...(request.newName === undefined ? {} : { newName: request.newName }),
    skip: request.skip,
    keepBoth: request.keepBoth,
  });
  if (!ports.isCurrent()) return { kind: "stale" };

  const expected = new Set(createDirectory ? ["create-directory"] : request.tickets.map((ticket) => ticket.id));
  const skip = new Set(request.skip);
  const keepBoth = new Set(request.keepBoth);
  const seen = new Set<string>();
  const errors: string[] = [];
  const conflicts: string[] = [];

  if (preview.rows.length !== expected.size) errors.push("transfer_preview_incomplete");
  for (const row of preview.rows) {
    if (!expected.has(row.ticketId) || seen.has(row.ticketId)) {
      errors.push("transfer_preview_incomplete");
      continue;
    }
    seen.add(row.ticketId);

    if (skip.has(row.ticketId)) {
      if (!row.skipped || row.error) errors.push("transfer_skip_not_applied");
      continue;
    }
    if (row.skipped) {
      errors.push("transfer_preview_unexpected_skip");
      continue;
    }
    if (row.error) {
      if (isTargetConflict(row.error) && !skip.has(row.ticketId)) {
        if (keepBoth.has(row.ticketId)) errors.push("transfer_keep_both_conflict_persisted");
        else conflicts.push(row.ticketId);
      } else {
        errors.push(row.error);
      }
      continue;
    }
    if (!row.preview) errors.push("transfer_preview_incomplete");
  }
  for (const id of expected) {
    if (!seen.has(id)) errors.push("transfer_preview_incomplete");
  }

  if (errors.length) return { kind: "failed", preview, errors: [...new Set(errors)] };
  if (conflicts.length) return { kind: "conflicts", preview, ticketIds: conflicts };
  if (preview.rows.every((row) => row.skipped)) return { kind: "all-skipped", preview };

  if (!ports.isCurrent()) return { kind: "stale" };
  await ports.beforeExecute();
  if (!ports.isCurrent()) return { kind: "stale" };
  const result = await ports.execute(preview);
  return { kind: "executed", preview, result };
}
