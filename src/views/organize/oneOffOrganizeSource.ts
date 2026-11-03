import { FileWorkspaceController } from "../../fileWorkspace/fileWorkspaceController";
import type { FileWorkspaceApi } from "../../api/fileWorkspaceApi";
import type { BrowseEntry, BrowseOpenRequest, BrowseOpenResponse } from "../../types/fileWorkspace";

export interface OneOffOrganizeSource {
  folder: BrowseOpenResponse;
  /** Direct children, including directories for explicit review; never recursive. */
  entries: readonly BrowseEntry[];
}

/**
 * Read an explicitly chosen folder without Managed Library admission/indexing.
 * The consumer must capture backend-verified Plan snapshots while refs are live;
 * neither these refs nor display paths are durable Plan/execution authority.
 */
export async function withOneOffOrganizeSource<T>(
  request: BrowseOpenRequest,
  consume: (source: OneOffOrganizeSource) => Promise<T>,
  options: { maxItems: number; signal?: AbortSignal; api?: FileWorkspaceApi }
): Promise<T> {
  const controller = new FileWorkspaceController(options.api);
  const checkCancellation = () => options.signal?.throwIfAborted();
  try {
    checkCancellation();
    const folder = await controller.openBrowse(request, () => !options.signal?.aborted);
    checkCancellation();
    if (!folder) throw new Error("one_off_folder_unavailable");

    // Start a fresh, unfiltered enumeration rather than treating an initial
    // search/page sample as the folder's complete contents.
    let page = await controller.startEnumeration(folder.rootPathRef);
    const entries: BrowseEntry[] = [];
    while (page) {
      checkCancellation();
      if ((page.skippedEntries ?? 0) > 0) throw new Error("one_off_folder_incomplete");
      entries.push(...page.entries);
      if (entries.length > options.maxItems) throw new Error("one_off_folder_too_large");
      if (!page.nextCursor) {
        if (page.completion !== "complete") throw new Error("one_off_folder_incomplete");
        return await consume({ folder, entries });
      }
      page = await controller.nextPage();
    }
    throw new Error("one_off_folder_incomplete");
  } finally {
    await controller.dispose();
  }
}
