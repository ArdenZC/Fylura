import { invokeCommand } from "./core";
import type { EntryRef, BrowseEntryRef, BrowsePathRef } from "../types/fileWorkspace";
import type { LibrarySelectionV1, OperationPreview, ExecuteOperationResult } from "../types/domain";
export type TransferAction = "copy" | "move" | "duplicate" | "rename" | "trash" | "system_trash" | "create_directory" | "create_file";
export interface TransferTicket { id: string; name: string; path: string; isDirectory: boolean; relativePath?: string }
export interface PersistentBasketEntry { id: string; name: string; path: string; isDirectory: boolean; status: "available" | "changed" | "unavailable" | "offline" }
export interface PersistentBasketPage { entries: PersistentBasketEntry[]; total: number }
export interface TransferPreviewRow { ticketId: string; preview: OperationPreview | null; error: string | null; skipped: boolean }
export interface TransferPreview { id: string; destinationDirectory: string; rows: TransferPreviewRow[] }
export interface SavedPlace { id: string; name: string; routingHint: string; favorite: boolean; visitedAt: number; unavailable: boolean; current?: boolean }
export type KnownLocationKind = "folder" | "volume" | "cloud_provider";
export type VolumeAvailability = "unknown" | "available" | "connected_unverified" | "permission_required" | "offline";
export interface VolumeState { identity: string | null; kind: "unknown" | "local" | "removable" | "network" | "cloud_provider"; readOnly: boolean | null; availableBytes: number | null; totalBytes: number | null; canEject: boolean; availability?: VolumeAvailability }
export interface KnownLocation { id: string; kind: KnownLocationKind; displayName: string; path: string; available: boolean; volume?: VolumeState | null }
export interface TransferBrowseDestination { sessionId: string; pathRef: BrowsePathRef }
export type SystemClipboardRequest = { kind: "read"; ticketIds?: string[] } | { kind: "write"; ticketIds: string[]; moveFiles: boolean };
export interface SystemClipboardTransfer { tickets: TransferTicket[]; action: "copy" | "move"; owned: boolean }
export const fileManagementApi = {
  basketPage(request: { offset: number; limit: number }): Promise<PersistentBasketPage> {
    return invokeCommand("file_workspace_basket_page", { request });
  },
  basketAdd(request: { ticketIds: string[] }): Promise<number> {
    return invokeCommand("file_workspace_basket_add", { request });
  },
  basketCapture(request: { entryIds: string[] }): Promise<TransferTicket[]> {
    return invokeCommand("file_workspace_basket_capture", { request });
  },
  basketRemove(request: { entryIds: string[] }): Promise<number> {
    return invokeCommand("file_workspace_basket_remove", { request });
  },
  basketClear(): Promise<number> {
    return invokeCommand("file_workspace_basket_clear");
  },
  basketReconcileMoves(request: { moves: { entryId: string; operationLogId: string }[] }): Promise<number> {
    return invokeCommand("file_workspace_basket_reconcile_moves", { request });
  },
  ejectVolume(request: { locationId: string; identity: string }): Promise<void> {
    return invokeCommand("file_workspace_eject_volume", { request });
  },
  systemClipboard(request: SystemClipboardRequest): Promise<SystemClipboardTransfer> {
    return invokeCommand("file_workspace_system_clipboard", { request });
  },
  nativeDragOut(request: { ticketIds: string[] }): Promise<void> {
    return invokeCommand("file_workspace_native_drag_out", { request });
  },
  captureTransfer(request: { refs?: EntryRef[]; selection?: LibrarySelectionV1; releaseIds?: string[]; externalPaths?: string[] }): Promise<TransferTicket[]> {
    return invokeCommand("file_workspace_transfer_capture", { request });
  },
  captureTransferSelectionPage(request: { sessionId: string; snapshotId: string; offset: number; excludedEntryIds?: string[]; advanceSnapshot?: boolean }): Promise<TransferTicket[]> {
    return invokeCommand("file_workspace_transfer_capture_selection_page", { request });
  },
  previewTransfer(request: { ticketIds: string[]; action: TransferAction; destinationDirectory: string; destinationEntry?: BrowseEntryRef; destinationBrowse?: TransferBrowseDestination; newName?: string; newNames?: Record<string, string>; keepBoth?: string[]; skip?: string[] }): Promise<TransferPreview> {
    return invokeCommand("file_workspace_transfer_preview", { request });
  },
  executeTransfer(request: { previewId: string; confirmed: boolean }): Promise<ExecuteOperationResult> {
    return invokeCommand("file_workspace_transfer_execute", { request });
  },
  createZip(request: { ticketIds: string[]; outputPath: string; compressionLevel?: number }): Promise<ExecuteOperationResult> {
    return invokeCommand("file_workspace_zip_create", { request });
  },
  extractZip(request: { ticketId: string; target: "here" | "same_name_folder" | "chosen_directory"; destinationDirectory?: string; conflictPolicy: "skip" | "keep_both" }): Promise<ExecuteOperationResult> {
    return invokeCommand("file_workspace_zip_extract", { request });
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
