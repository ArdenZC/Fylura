import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { tauriApi } from "../../api/tauriApi";
import { useOrganizationPlanStore } from "../../store/useOrganizationPlanStore";
import type { Translator } from "../../types/ui";
import { readableError } from "../../utils/viewHelpers";
import { withOneOffOrganizeSource } from "./oneOffOrganizeSource";

export function useOneOffOrganizeTask(t: Translator, platform: string) {
  function taskError(error: unknown) {
    const code = readableError(error);
    return t(code.includes("too_large") ? "organizeOneOffTooLarge" : /incomplete|folder_changed/.test(code) ? "organizeOneOffIncomplete" : /source_changed|source_identity/.test(code) ? "organizeOneOffSourceChanged" : "organizeOneOffProviderError");
  }
  const [folderPath, setFolderPath] = useState("");
  const [reading, setReading] = useState(false);
  const activePlanId = useOrganizationPlanStore((state) => state.activePlan?.id);
  const [consentPlanId, setConsentPlanId] = useState<string | null>(null);
  const consent = Boolean(activePlanId && activePlanId === consentPlanId);
  const setConsent = (allowed: boolean) => setConsentPlanId(allowed ? useOrganizationPlanStore.getState().activePlan?.id ?? null : null);
  const analysisRunning = useRef(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const stop = useRef(false);
  const sourceAbort = useRef<AbortController | null>(null);
  useEffect(() => () => { stop.current = true; sourceAbort.current?.abort(); }, []);

  async function chooseFolder() {
    try {
      const selection = await open({ directory: true, multiple: false, title: t("browseChooseFolderTitle") });
      const path = Array.isArray(selection) ? selection[0] : selection;
      if (path) setFolderPath(path);
    } catch (error) { setError(taskError(error)); }
  }

  async function create(title: string) {
    if (sourceAbort.current || !folderPath.trim()) return;
    setReading(true); setError(null); setConsent(false);
    const abort = new AbortController(); sourceAbort.current = abort;
    const folderName = folderPath.trim().replace(/\\/g, "/").split("/").filter(Boolean).at(-1) ?? folderPath.trim();
    try {
      await withOneOffOrganizeSource({ displayHint: folderName, platform: platform === "win32" ? "windows" : "macos", routingHint: folderPath.trim() },
        ({ folder, entries }) => useOrganizationPlanStore.getState().createPlan({
          kind: "one_off", version: 1, sessionId: folder.sessionId, pathRef: folder.rootPathRef, entries: entries.map((entry) => entry.ref)
        }, entries.length, title || folderName), { maxItems: 100, signal: abort.signal });
    } catch (error) {
      if (!abort.signal.aborted) setError(taskError(error));
    } finally { sourceAbort.current = null; setReading(false); }
  }

  async function analyze() {
    const plan = useOrganizationPlanStore.getState().activePlan;
    if (!plan || plan.sourceKind !== "one_off" || !consent || analysisRunning.current) return;
    analysisRunning.current = true;
    stop.current = false; setError(null); setProgress({ done: 0, total: 0 });
    try {
      const page = await tauriApi.queryOrganizationPlanItems({ planId: plan.id, pageSize: 100 });
      if (page.hasMore) throw new Error("one_off_folder_too_large");
      const candidates = page.items.filter((item) => item.validity === "needs_analysis" && !item.sourceIsDirSnapshot && item.decision !== "kept");
      setProgress({ done: 0, total: candidates.length });
      let done = 0;
      for (const item of candidates) {
        if (stop.current || useOrganizationPlanStore.getState().activePlan?.id !== plan.id) break;
        try { const result = await useOrganizationPlanStore.getState().analyzeMissing([item.id], true); if (result.applied && result.value === 0) setError(t("organizeOneOffProviderError")); }
        catch (error) { setError(taskError(error)); }
        setProgress({ done: ++done, total: candidates.length });
      }
      if (useOrganizationPlanStore.getState().activePlan?.id === plan.id) await useOrganizationPlanStore.getState().refreshPlan();
    } catch (error) { setError(taskError(error)); }
    finally { analysisRunning.current = false; setProgress(null); }
  }

  return { folderPath, setFolderPath, reading, consent, setConsent, progress, error, chooseFolder, create, analyze,
    cancel: () => { stop.current = true; sourceAbort.current?.abort(); } };
}
