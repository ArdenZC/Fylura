import { describe, expect, it } from "vitest";
import { mockFileWorkspaceInvoke as invoke } from "../src/api/fileWorkspaceMockApi";
import type { BrowseOpenResponse, BrowsePage, BrowseSortSpec } from "../src/types/fileWorkspace";
describe("Browse sorted pagination contract", () => {
  it("passes whole-folder sorting through eight field/direction combinations over 100+ entries and preserves query filtering", async () => {
    const session = await invoke<BrowseOpenResponse>("file_workspace_browse_open", { request: { platform:"windows", routingHint:"C:/U3/Left" } });
    for (const field of ["name","type","size","modified"] as const) for (const direction of ["asc","desc"] as const) {
      const sort: BrowseSortSpec = {field,direction,foldersFirst:true};
      let page = await invoke<BrowsePage>("file_workspace_browse_start_enumeration", {request:{sessionId:session.sessionId,requestId:`${field}-${direction}`,pathRef:session.rootPathRef,pageSize:17,query:{text:null,entryKind:"all",sort}}});
      const entries = [...page.entries];
      while (page.nextCursor) {
        page = await invoke<BrowsePage>("file_workspace_browse_next_page", {request:{sessionId:session.sessionId,cursor:page.nextCursor,pageSize:17}});
        entries.push(...page.entries);
      }
      expect(entries).toHaveLength(138);
      expect(page.knownCount).toBe(138);
      expect(entries[0].kind).toBe("directory");
      const key = (e: typeof entries[number]) => field === "name" ? e.name : field === "type" ? e.extension : field === "size" ? e.size : e.modifiedAt;
      for (let i=2;i<entries.length;i++) {
        const a=key(entries[i-1]),b=key(entries[i]);
        const order=a===b ? (entries[i-1].name < entries[i].name ? -1:1) : a! < b! ? -1:1;
        expect(direction === "asc" ? order : -order).toBeLessThan(0);
      }
    }
    const filtered = await invoke<BrowsePage>("file_workspace_browse_start_enumeration", {request:{sessionId:session.sessionId,requestId:"filtered",pathRef:session.rootPathRef,pageSize:256,query:{text:"Document-13",entryKind:"file",sort:{field:"name",direction:"desc",foldersFirst:false}}}});
    expect(filtered.completion).toBe("complete");
    expect(filtered.entries.map((e)=>e.name)).toEqual(["Document-136.md","Document-135.txt","Document-134.md","Document-133.txt","Document-132.md","Document-131.txt","Document-130.md"]);
  });
});
