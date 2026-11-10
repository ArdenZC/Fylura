import { fileManagementApi, type TransferTicket } from "../../../api/fileManagementApi";
import { transferDestinationContextKey, useFileManagementStore } from "./fileManagementStore";
import { releaseTransferTickets } from "./transferTicketOwnership";

// Serialize only OS clipboard handoff; file tasks stay on TransferOrchestrator.
let clipboardTail: Promise<void> = Promise.resolve();
function clipboardHandoff<T>(work: () => Promise<T>): Promise<T> {
  const current = clipboardTail.then(work);
  clipboardTail = current.then(() => undefined, () => undefined);
  return current;
}

export function writeSystemClipboardForCopy(capture: Promise<TransferTicket[]>, moveFiles: boolean) {
  // Capture begins at the user's Copy/Cut event, before waiting for another
  // pane's OS handoff. Observe failure immediately to avoid a delayed rejection.
  const admitted = capture.then((tickets) => ({ ok: true as const, tickets }), (error: unknown) => ({ ok: false as const, error }));
  return clipboardHandoff(async () => {
    const result = await admitted;
    if (!result.ok) throw result.error;
    const tickets = result.tickets;
    if (!tickets.length) return;
    if (useFileManagementStore.getState().transferActive) {
      await releaseTransferTickets(tickets);
      throw Error("transfer_busy");
    }
    try { await fileManagementApi.systemClipboard({ kind: "write", ticketIds: tickets.map((ticket) => ticket.id), moveFiles }); }
    catch (error) { await releaseTransferTickets(tickets); throw error; }
    const old = useFileManagementStore.getState().clipboard;
    useFileManagementStore.setState({ clipboard: { tickets, action: moveFiles ? "move" : "copy" } });
    if (old) await releaseTransferTickets(old.tickets);
  });
}

/** Admit a user Paste into the pane that requested it, even across OS latency. */
export async function readSystemClipboardForPaste(paneId: string) {
  const before = useFileManagementStore.getState();
  const surface = before.surfaces[paneId]?.read();
  const destination = transferDestinationContextKey(surface);
  if (!destination || surface?.visible === false || before.activePaneId !== paneId || before.transferActive || before.transferOpen)
    throw Error("transfer_selection_changed");
  let ticketIds: string[] = [];
  const captured = await clipboardHandoff(() => {
    ticketIds = useFileManagementStore.getState().clipboard?.tickets.map((ticket) => ticket.id) ?? [];
    return fileManagementApi.systemClipboard({ kind: "read", ticketIds });
  });
  const current = useFileManagementStore.getState();
  const target = current.surfaces[paneId]?.read();
  const sameClipboardOwner = !captured.owned || JSON.stringify(current.clipboard?.tickets.map((ticket) => ticket.id) ?? []) === JSON.stringify(ticketIds);
  if (current.activePaneId !== paneId || target?.visible === false || transferDestinationContextKey(target) !== destination || current.transferActive || current.transferOpen || !sameClipboardOwner) {
    if (!captured.owned) await releaseTransferTickets(captured.tickets);
    throw Error("transfer_selection_changed");
  }
  return captured;
}
