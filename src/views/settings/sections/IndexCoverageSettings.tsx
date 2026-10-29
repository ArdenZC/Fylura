import { useEffect, useRef, useState } from "react";
import { tauriApi } from "../../../api/tauriApi";
import type { GlobalIndexSource } from "../../../types/domain";
import type { IndexCoverageMode, IndexCoveragePreview, IndexCoverageRule, IndexCoverageSnapshot } from "../../../types/indexCoverage";
import type { Translator } from "../../../types/ui";
import { focusVisibleState, buttonSecondary } from "../../../utils/tw";
import { quietText } from "../../shared/ui";

export function IndexCoverageSettings({ t, sources, disabled, statusText }: { t: Translator; sources: GlobalIndexSource[]; disabled: boolean; statusText?: (status: string) => string }) {
  const [snapshot, setSnapshot] = useState<IndexCoverageSnapshot | null>(null);
  const [mode, setMode] = useState<IndexCoverageMode>("smart_lean");
  const [rules, setRules] = useState<IndexCoverageRule[]>([]);
  const [sourceId, setSourceId] = useState("");
  const [path, setPath] = useState("");
  const [preview, setPreview] = useState<IndexCoveragePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const initialized = useRef(false);
  const pending = snapshot?.sources.some((s) => s.enabled && s.pending) ?? false;
  const sourceSignature = sources.map((s) => `${s.volume.id}:${s.volume.enabled}:${s.volume.indexStatus}`).join("|");
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function read() {
      try {
        const response = await tauriApi.globalIndexCoverage({ action: "read" });
        if (disposed) return;
        setSnapshot(response.snapshot);
        if (!initialized.current) {
          initialized.current = true;
          setMode(response.snapshot.policy.mode);
          setRules(response.snapshot.policy.rules.map(({ sourceId, path, include }) => ({ sourceId, path, include })));
        }
        if (response.snapshot.sources.some((s) => s.enabled && ["indexing", "syncing", "rebuild_required"].includes(s.status))) timer = setTimeout(() => void read(), 2500);
      } catch (e) { if (!disposed) setError(String(e)); }
    }
    void read();
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, [reload, sourceSignature]);
  const authorized = sources.filter((s) => s.volume.enabled && (s.volume.platform !== "windows" || s.volume.scopeAuthorizationState === "authorized"));
  const totals = snapshot?.sources.filter((s) => s.enabled).reduce((a, s) => ({ included: a.included + s.included, excluded: a.excluded + s.excluded, undetermined: a.undetermined + s.undetermined }), { included: 0, excluded: 0, undetermined: 0 });
  const editable = !disabled && !busy && snapshot !== null;
  const clearPreview = () => { setPreview(null); setError(null); };
  function addRule(include: boolean) {
    const id = sourceId || authorized[0]?.volume.id;
    if (!id || !path.trim()) return;
    setMode("custom");
    setRules((current) => [...current.filter((r) => r.sourceId !== id || r.path !== path.trim()), { sourceId: id, path: path.trim(), include }]);
    setPath(""); clearPreview();
  }
  async function submit(action: "preview" | "save") {
    if (!snapshot || !editable) return;
    setBusy(true); setError(null);
    try {
      const response = await tauriApi.globalIndexCoverage({ action, expectedRevision: snapshot.policy.revision, mode, rules });
      setSnapshot(response.snapshot); setPreview(response.preview);
      if (action === "save") setReload((v) => v + 1);
    } catch (e) { setError(String(e)); setPreview(null); }
    finally { setBusy(false); }
  }
  return <fieldset className="grid min-w-0 gap-3 rounded-lg border border-[var(--zc-control-border)] p-3" disabled={disabled || busy} aria-busy={busy}>
    <legend className="px-1 text-sm font-medium">{t("indexCoverageTitle")}</legend>
    <p className={quietText}>{t("indexCoverageAuthorization")}</p>
    <label className="grid gap-1 text-sm">{t("indexCoverageMode")}
      <select aria-label={t("indexCoverageMode")} value={mode} disabled={!editable} onChange={(e) => { setMode(e.target.value as IndexCoverageMode); clearPreview(); }} className={`${focusVisibleState} min-h-9 max-w-full rounded border border-[var(--zc-control-border)] bg-[var(--zc-surface)] p-2`}>
        <option value="smart_lean">{t("indexCoverageSmart")}</option><option value="broad_coverage">{t("indexCoverageBroad")}</option><option value="custom">{t("indexCoverageCustom")}</option>
      </select>
    </label>
    <p className={quietText}>{t("indexCoverageRulesHelp")}</p>
    {mode === "custom" ? <div className="grid min-w-0 gap-2">
      <label className="grid gap-1 text-sm">{t("indexCoverageSource")}<select aria-label={t("indexCoverageSource")} value={sourceId || authorized[0]?.volume.id || ""} onChange={(e) => { setSourceId(e.target.value); clearPreview(); }} className="zc-focus-visible focus-visible:outline-none min-h-9 min-w-0 rounded border border-[var(--zc-control-border)] bg-[var(--zc-surface)] p-2">
        {!authorized.length ? <option value="">{t("indexCoverageNoSource")}</option> : null}
        {authorized.map((s) => <option key={s.volume.id} value={s.volume.id}>{s.volume.displayName} · {s.volume.mountPath}</option>)}
      </select></label>
      <label className="grid gap-1 text-sm">{t("indexCoverageFolder")}<input value={path} onChange={(e) => { setPath(e.target.value); clearPreview(); }} className="zc-focus-visible focus-visible:outline-none min-h-9 min-w-0 rounded border border-[var(--zc-control-border)] bg-transparent p-2" /></label>
      <div className="flex flex-wrap gap-2"><button type="button" className={buttonSecondary} disabled={!editable || !path.trim() || !authorized.length || rules.length >= 128} onClick={() => addRule(true)}>{t("indexCoverageInclude")}</button><button type="button" className={buttonSecondary} disabled={!editable || !path.trim() || !authorized.length || rules.length >= 128} onClick={() => addRule(false)}>{t("indexCoverageExclude")}</button></div>
      <ul className="grid max-h-56 gap-2 overflow-auto text-sm">{rules.map((rule, i) => <li key={`${rule.sourceId}:${rule.path}`} className="flex min-w-0 items-center gap-2"><span className="min-w-0 flex-1 break-all">{rule.include ? t("indexCoverageInclude") : t("indexCoverageExclude")} · {rule.path}</span><button type="button" className={buttonSecondary} aria-label={`${t("indexCoverageRemove")} ${rule.path}`} onClick={() => { setRules((r) => r.filter((_, index) => index !== i)); clearPreview(); }}>{t("indexCoverageRemove")}</button></li>)}</ul>
    </div> : null}
    <p role="status" className={quietText}>{!snapshot ? t("loading") : pending ? snapshot.sources.some((s) => s.enabled && ["indexing", "syncing", "rebuild_required"].includes(s.status)) ? t("indexCoverageReconfiguring") : t("indexCoverageWaiting") : snapshot.sources.some((s) => s.enabled && s.hasUndetermined) ? t("indexCoveragePartial") : t("indexCoverageCurrent")}</p>
    <ul className="grid max-h-56 gap-1 overflow-auto text-sm">{snapshot?.sources.filter((s) => s.enabled).map((s) => <li key={s.sourceId}>{sources.find((source) => source.volume.id === s.sourceId)?.volume.displayName ?? s.sourceId} · {t("indexCoverageSourceHealth")}: {statusText?.(s.status) ?? s.status}</li>)}</ul>
    {totals ? <p className={quietText}>{t("indexCoverageObservations")} · {t("indexCoverageIncluded")}: {totals.included} · {t("indexCoverageExcluded")}: {totals.excluded} · {t("indexCoverageUndetermined")}: {totals.undetermined}</p> : null}
    {preview ? <div role="status" className="grid gap-1 text-sm"><p>{t("indexCoveragePreviewHelp")}</p><p>{preview.sampled} / {preview.sampleLimit} · {t("indexCoverageIncluded")}: {preview.included} · {t("indexCoverageExcluded")}: {preview.excluded} · {t("indexCoverageUndetermined")}: {preview.undetermined}</p><p>{t("indexCoverageApplyHelp")}</p></div> : null}
    {error ? <p role="alert" className="break-words text-sm">{t("indexCoverageFailed")} {error}</p> : null}
    <div className="flex flex-wrap gap-2"><button type="button" className={buttonSecondary} disabled={!editable} onClick={() => void submit("preview")}>{t("indexCoveragePreview")}</button><button type="button" className={buttonSecondary} disabled={!editable || !preview} onClick={() => void submit("save")}>{t("indexCoverageApply")}</button><button type="button" className={buttonSecondary} onClick={() => { initialized.current = false; clearPreview(); setReload((v) => v + 1); }}>{t("indexCoverageRefresh")}</button></div>
  </fieldset>;
}
