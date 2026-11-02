import type { Translator } from "../../types/ui";
import { Button } from "../shared/ui";

/** Task data-sharing consent, separate from group/file selection semantics. */
export function OneOffOrganizeConsent({ consent, busy, providerReady, onConsent, onAnalyze, t }: {
  consent: boolean;
  busy: boolean;
  providerReady: boolean;
  onConsent: (allowed: boolean) => void;
  onAnalyze: () => void;
  t: Translator;
}) {
  return <div className="flex flex-wrap items-center gap-3 text-sm">
    <label className="flex items-center gap-2"><input type="checkbox" checked={consent} disabled={busy} onChange={(event) => onConsent(event.target.checked)} />{t("organizeOneOffConsent")}</label>
    <Button variant="primary" disabled={busy || !consent || !providerReady} onClick={onAnalyze}>{t("organizePlanAnalyze")}</Button>
  </div>;
}
