/** Browser-only deterministic UI filesystem. Never native mutation evidence. */
import type { BrowseEntry, EntryRef } from "../types/fileWorkspace";
import type { TransferPreview, TransferTicket, SavedPlace } from "./fileManagementApi";
import type { OperationLog, OperationPreview } from "../types/domain";
const files = new Map<string, { id: string; version: number; directory?: boolean }>([
  ["C:/U1/Source/one.txt", { id: "one", version: 1 }],
  ["C:/U1/Source/two.txt", { id: "two", version: 1 }],
  ["C:/U1/Other/three.txt", { id: "three", version: 1 }],
  ["C:/U1/Target/one.txt", { id: "conflict", version: 1 }]
]);
const tickets = new Map<string, TransferTicket>();
const previews = new Map<string, TransferPreview>();
const places: SavedPlace[] = [];
let serial = 0;
export const managementHistory: OperationLog[] = [];
const parent = (path: string) => path.slice(0, path.lastIndexOf("/"));
export function managementFixturePage(sessionId: string, directory: string, enumerationId: string): BrowseEntry[] | null {
  if (!directory.replace(/\\/g, "/").startsWith("C:/U1/")) return null;
  return [...files].filter(([path]) => parent(path) === directory).map(([path, file]) => ({
    ref: { kind: "ephemeral", browseSessionId: sessionId, entryId: `${enumerationId}:${file.id}:${file.version}` },
    name: path.slice(path.lastIndexOf("/") + 1), displayPath: path, kind: "file", size: 10, materialization: "unknown"
  }));
}
export function isManagementCommand(command: string) { return ["file_workspace_transfer_capture", "file_workspace_transfer_preview", "file_workspace_transfer_execute", "file_workspace_places"].includes(command); }
export function managementMock(command: string, request: any, resolve: (ref: EntryRef) => { name: string; path: string; isDirectory: boolean }, resolveLocation: (sessionId: string) => string) {
  if (command.endsWith("_capture")) {
    for (const id of request.releaseIds ?? []) tickets.delete(id);
    const refs: EntryRef[] = request.selection?.kind === "explicit" ? request.selection.fileIds.map((fileId: string) => ({ kind: "managed", fileId })) : request.refs ?? [];
    return refs.map((ref) => { const entry = resolve(ref); const old = [...tickets.values()].find((ticket) => ticket.path === entry.path); if (old) return old; const ticket = { id: `ticket-${++serial}`, ...entry }; tickets.set(ticket.id, ticket); return ticket; });
  }
  if (command.endsWith("_preview")) {
    const reserved = new Set<string>();
    const rows = request.ticketIds.map((ticketId: string) => {
      const ticket = tickets.get(ticketId); const skipped = request.skip?.includes(ticketId) ?? false;
      if (!ticket) return { ticketId, skipped, preview: null, error: "transfer_reference_expired" };
      if (ticket.path.startsWith("C:/U1/") && !files.has(ticket.path)) return { ticketId, skipped, preview: null, error: "transfer_source_changed" };
      let name = ticket.name;
      let target = `${request.destinationDirectory}/${name}`;
      if (request.keepBoth?.includes(ticketId) && (files.has(target) || reserved.has(target))) {
        const dot = name.lastIndexOf("."); const stem = dot > 0 ? name.slice(0, dot) : name; const ext = dot > 0 ? name.slice(dot) : "";
        let n = 2; do { name = `${stem} (${n++})${ext}`; target = `${request.destinationDirectory}/${name}`; } while (files.has(target) || reserved.has(target));
      }
      const conflict = files.has(target) || reserved.has(target); reserved.add(target);
      const preview = { id: `op-transfer-mock-${++serial}`, fileId: ticketId, operation_type: request.action, source_path: ticket.path, target_path: target, old_name: ticket.name, new_name: name, reason: "Browser fixture manual transfer", status: "pending", risk_level: "Normal", confidence: 1, requires_confirmation: true, suggested_action: request.action, is_executable: !conflict, operationFingerprint: `mock-${serial}` } as OperationPreview;
      return { ticketId, skipped, preview: conflict ? null : preview, error: conflict ? "transfer_target_exists" : null };
    });
    const preview = { id: `preview-${++serial}`, destinationDirectory: request.destinationDirectory, rows };
    previews.set(preview.id, preview); return preview;
  }
  if (command.endsWith("_execute")) {
    const preview = previews.get(request.previewId); previews.delete(request.previewId);
    if (!request.confirmed || !preview || preview.rows.some((row) => !row.skipped && row.error)) throw Error("operation_preview_stale");
    const batchId = `batch-${++serial}`;
    const result = { batch_id: batchId, logs: preview.rows.filter((row) => !row.skipped).map((row) => {
      const op = row.preview!; const source = files.get(op.source_path) ?? { id: `copied-${++serial}`, version: 1 };
      files.set(op.target_path, { ...source, id: `published-${++serial}` }); if (op.operation_type === "move") files.delete(op.source_path);
      return { id: `log-${++serial}`, batch_id: batchId, operation_type: op.operation_type, source_path: op.source_path, target_path: op.target_path, path_before: op.source_path, path_after: op.target_path, name_before: op.old_name, name_after: op.new_name, old_name: op.old_name, new_name: op.new_name, error_message: null, restored_at: null, restore_error: null, status: "success", can_restore: false, can_undo: false, restore_status: "not_restored", operation_phase: "completed", created_at: new Date().toISOString() } as OperationLog;
    }) };
    managementHistory.unshift(...result.logs);
    return result;
  }
  if (request.sessionId) {
    const path = resolveLocation(request.sessionId);
    let place = places.find((p) => p.routingHint === path);
    if (!place) { place = { id: `place-${++serial}`, routingHint: path, name: path.split("/").at(-1)!, favorite: false, visitedAt: Date.now(), unavailable: false }; places.unshift(place); }
    if (request.favorite !== undefined) place.favorite = request.favorite;
    for (const p of places) (p as SavedPlace & {current?:boolean}).current = p.id === place.id;
  }
  if (request.removeId) places.splice(0, places.length, ...places.filter((place) => place.id !== request.removeId));
  return places;
}
