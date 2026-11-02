import { invokeCommand } from "./core";
import type { EntryRef, BrowseEntryRef, BrowsePathRef } from "../types/fileWorkspace";
import type { LibrarySelectionV1, OperationPreview, ExecuteOperationResult } from "../types/domain";
export type TransferAction = "copy" | "move" | "rename" | "trash" | "create_directory";
export interface TransferTicket { id: string; name: string; path: string; isDirectory: boolean; relativePath?: string }
export interface TransferPreviewRow { ticketId: string; preview: OperationPreview | null; error: string | null; skipped: boolean }
export interface TransferPreview { id: string; destinationDirectory: string; rows: TransferPreviewRow[] }
export interface SavedPlace { id: string; name: string; routingHint: string; favorite: boolean; visitedAt: number; unavailable: boolean; current?: boolean }
export type KnownLocationKind = "folder" | "volume";
export interface KnownLocation { id: string; kind: KnownLocationKind; displayName: string; path: string; available: boolean }
export const fileManagementApi = {
  captureTransfer(request: { refs?: EntryRef[]; selection?: LibrarySelectionV1; releaseIds?: string[]; externalPaths?: string[] }): Promise<TransferTicket[]> {
    return invokeCommand("file_workspace_transfer_capture", { request });
  },
  previewTransfer(request: { ticketIds: string[]; action: TransferAction; destinationDirectory: string; destinationEntry?: BrowseEntryRef; newName?: string; keepBoth?: string[]; skip?: string[] }): Promise<TransferPreview> {
    return invokeCommand("file_workspace_transfer_preview", { request });
  },
  executeTransfer(request: { previewId: string; confirmed: boolean }): Promise<ExecuteOperationResult> {
    return invokeCommand("file_workspace_transfer_execute", { request });
  },
  fileLibraryPlaces(request: { sessionId?: string; pathRef?: BrowsePathRef; favorite?: boolean; removeId?: string } = {}): Promise<SavedPlace[]> {
    return invokeCommand("file_workspace_places", { request });
  },
  /**
   * Read-only projection of well-known folders and mounted volumes. It is not a
   * place, a scan root or an index authorization; opening one still uses the
   * existing user-intent Browse admission.
   */
  fileLibraryKnownLocations(): Promise<KnownLocation[]> {
    return invokeCommand("file_workspace_known_locations");
  }
};
