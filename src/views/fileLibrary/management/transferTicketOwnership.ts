import { fileManagementApi } from "../../../api/fileManagementApi";
import type { TransferTicket } from "../../../api/fileManagementApi";
import { useFileManagementStore } from "./fileManagementStore";

/** Release only this capture's tickets after every live UI holder lets go. */
export async function releaseTransferTickets(tickets: TransferTicket[] | string[]) {
  const state = useFileManagementStore.getState();
  const retained = new Set([
    ...state.basket.map((ticket) => ticket.id),
    ...(state.clipboard?.tickets ?? []).map((ticket) => ticket.id),
    ...(state.pendingTransfer?.tickets ?? []).map((ticket) => ticket.id),
    ...state.transferTicketIds,
  ]);
  const releaseIds = [...new Set(tickets.map((ticket) => typeof ticket === "string" ? ticket : ticket.id))]
    .filter((id) => !retained.has(id));
  if (releaseIds.length) await fileManagementApi.captureTransfer({ releaseIds });
}
