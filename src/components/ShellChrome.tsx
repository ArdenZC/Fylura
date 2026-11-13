import { useId, useRef, useState } from "react";
import { Eye, Languages, Monitor, Moon, Sun } from "lucide-react";
import type { Language } from "../i18n";
import type { ThemeMode, Translator } from "../types/ui";
import { useI18nContext, useThemeContext } from "../contexts/AppContexts";
import { cn, overlaySurface, focusVisibleState, buttonDefault, buttonPrimary } from "../utils/tw";
import { BrandMark } from "./ui/BrandMark";
import { ModalPortal } from "./modal/ModalPortal";

const titlebarToolButton =
  cn("grid h-8 w-8 place-items-center rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] text-[var(--zc-text-secondary)] shadow-none transition-[background,border-color,box-shadow,color] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)] hover:border-[var(--zc-border)] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState);
const titlebarPillButton =
  cn("inline-flex h-8 items-center gap-1.5 rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-3 text-xs font-medium text-[var(--zc-text-secondary)] shadow-none transition-[background,border-color,box-shadow,color] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)] hover:border-[var(--zc-border)] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState);

export function ZenMark({
  decorative = true,
  size = "sidebar",
  ariaLabel
}: {
  decorative?: boolean;
  size?: "micro" | "sidebar" | "app";
  ariaLabel?: string;
} = {}) {
  return <BrandMark size={size} decorative={decorative} aria-label={ariaLabel} />;
}

export function AmbientMesh() {
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden" aria-hidden="true">
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_12%_0%,var(--zc-ambient-primary),transparent_42%)]" />
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_86%_88%,var(--zc-ambient-secondary),transparent_38%)]" />
      <div className="absolute inset-0 bg-[linear-gradient(180deg,transparent_72%,var(--zc-ambient-vignette))]" />
    </div>
  );
}

export function TitlebarTools({
  language,
  theme,
  effectiveTheme,
  setLanguage,
  setTheme,
  t
}: {
  language: Language;
  theme: ThemeMode;
  effectiveTheme: Exclude<ThemeMode, "system">;
  setLanguage: (language: Language) => void;
  setTheme: (theme: ThemeMode) => void;
  t: Translator;
}) {
  const nextTheme = nextThemeMode(theme);
  const themeLabel = themeToggleLabel(language, theme, nextTheme, t);

  return (
    <div className="flex items-center gap-2 [-webkit-app-region:no-drag]">
      <button
        className={titlebarToolButton}
        onClick={() => setTheme(nextTheme)}
        aria-label={themeLabel}
        title={themeLabel}
      >
        {themeIcon(theme)}
      </button>
      <button
        className={titlebarPillButton}
        onClick={() => setLanguage(language === "zh" ? "en" : "zh")}
      >
        <Languages size={16} />
        <span>{language === "zh" ? "EN" : "中文"}</span>
      </button>
    </div>
  );
}

/**
 * Titlebar display controls. v3.40 keeps a single theme entry here; interface
 * density and language are configuration values and live in Settings, so the
 * titlebar does not grow a permanent control strip (§4.2, §9.14).
 */
export function WorkspaceTopActions() {
  const { theme, setTheme } = useThemeContext();
  const { t } = useI18nContext();
  const nextTheme = nextThemeMode(theme);
  const themeLabel = themeToggleLabel("zh", theme, nextTheme, t);

  return (
    <div className="flex items-center gap-1 [-webkit-app-region:no-drag]" aria-label={t("workspaceDisplayControls")}>
      <button
        type="button"
        className="zc-titlebar-tool"
        onClick={() => setTheme(nextTheme)}
        aria-label={themeLabel}
        title={themeLabel}
        aria-pressed={theme === "dark"}
      >
        <Eye size={16} aria-hidden="true" />
      </button>
    </div>
  );
}

function nextThemeMode(theme: ThemeMode): ThemeMode {
  if (theme === "system") return "light";
  if (theme === "light") return "dark";
  return "system";
}

function themeIcon(theme: ThemeMode) {
  if (theme === "system") return <Monitor size={17} />;
  if (theme === "light") return <Sun size={17} />;
  return <Moon size={17} />;
}

function themeToggleLabel(language: Language, current: ThemeMode, next: ThemeMode, t: Translator) {
  const labelFor = (mode: ThemeMode) => {
    if (mode === "system") return t("systemTheme");
    if (mode === "light") return t("lightTheme");
    return t("darkTheme");
  };

  if (language === "zh") return `当前：${labelFor(current)}，点击切换到${labelFor(next)}`;
  return `Current: ${labelFor(current)}. Click to switch to ${labelFor(next)}.`;
}

export function CloseChoiceDialog({
  t,
  onCancel,
  onChoose
}: {
  t: Translator;
  onCancel: () => void;
  onChoose: (action: "minimize" | "quit", remember: boolean) => Promise<void>;
}) {
  const [remember, setRemember] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState<"minimize" | "quit" | null>(null);
  const cancelRef = useRef<HTMLButtonElement | null>(null);
  const titleId = useId();
  const descriptionId = useId();
  const onCancelRef = useRef(onCancel);
  const isSubmittingRef = useRef(isSubmitting);
  onCancelRef.current = onCancel;
  isSubmittingRef.current = isSubmitting;

  async function choose(action: "minimize" | "quit") {
    setIsSubmitting(action);
    await onChoose(action, remember);
  }

  return (
    <ModalPortal initialFocusRef={cancelRef} onEscape={() => { if (isSubmittingRef.current === null) onCancelRef.current(); }}>
    <div className="fixed inset-0 z-50 grid place-items-center bg-[var(--zc-overlay)] p-6">
      <section className={cn(overlaySurface, "grid w-full max-w-md gap-5 p-6")} role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId}>
        <div className="mx-auto">
          <ZenMark decorative={false} ariaLabel={t("appName")} />
        </div>
        <div className="text-center">
          <h2 id={titleId} className="text-xl font-semibold">{t("closeChoiceTitle")}</h2>
          <p id={descriptionId} className="mt-2 text-sm text-[var(--muted)]">{t("closeChoiceDesc")}</p>
        </div>
        <label className="flex items-center justify-center gap-2 text-sm text-[var(--muted)]">
          <input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} />
          <span>{t("doNotAskAgain")}</span>
        </label>
        <div className="grid grid-cols-3 gap-2">
          <button ref={cancelRef} className={buttonDefault} onClick={onCancel} disabled={isSubmitting !== null}>
            {t("cancel")}
          </button>
          <button className={buttonDefault} onClick={() => void choose("quit")} disabled={isSubmitting !== null}>
            {t("quitApp")}
          </button>
          <button className={buttonPrimary} onClick={() => void choose("minimize")} disabled={isSubmitting !== null}>
            {t("minimizeToTray")}
          </button>
        </div>
      </section>
    </div>
    </ModalPortal>
  );
}
