import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode, type RefObject, type WheelEvent } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, Search, X } from "lucide-react";
import { cn, focusSurface, focusVisibleState, focusWithinSurface, selectedFocusSurface, selectedFocusVisibleState, selectedSurface } from "../../../utils/tw";
import { switchThumb, switchTrack } from "../../../components/ui/Switch";

export type SettingsSectionOption = {
  id: string;
  label: string;
};

export type SettingsSearchResult = {
  sectionId: string;
  sectionLabel: string;
  title: string;
  description?: string;
  targetId?: string;
};

type SectionChangeOptions = {
  focusContent?: boolean;
  revealContent?: boolean;
};

export function settingsSectionContentTop(container: HTMLElement) {
  const containerRect = container.getBoundingClientRect();
  const navShell = container.querySelector<HTMLElement>("[data-settings-section-nav-shell]");
  const navRect = navShell?.getBoundingClientRect();
  const horizontalNav = Boolean(navRect && navRect.width >= containerRect.width * 0.75);
  return containerRect.top + (horizontalNav && navRect ? navRect.height + 8 : 0);
}

export function scrollSettingsSectionIntoView(
  container: HTMLElement | null,
  sectionId: string,
  options: SectionChangeOptions = {}
) {
  if (!container) return null;
  const section = container.querySelector<HTMLElement>(`#${sectionId}`);
  if (!section) return null;
  if (options.revealContent) {
    const disclosure = section.querySelector<HTMLDetailsElement>("details[data-settings-progressive-disclosure]");
    if (disclosure) disclosure.open = true;
  }
  const firstSection = container.querySelector<HTMLElement>("[data-settings-section-content]");
  const targetTop = settingsSectionContentTop(container);
  container.scrollTop = section === firstSection
    ? 0
    : Math.max(0, container.scrollTop + section.getBoundingClientRect().top - targetTop);
  const heading = section.querySelector<HTMLElement>("[data-settings-section-heading]");
  if (options.focusContent !== false) (heading ?? section).focus({ preventScroll: true });
  return { section, heading };
}

export function centerSettingsNavItem(nav: HTMLElement | null, item: HTMLElement | null) {
  if (!nav || !item || nav.scrollWidth <= nav.clientWidth) return;
  const target = item.offsetLeft - (nav.clientWidth - item.offsetWidth) / 2;
  nav.scrollLeft = Math.max(0, Math.min(target, nav.scrollWidth - nav.clientWidth));
}

export function activeSettingsSectionId(container: HTMLElement, sectionIds: readonly string[]) {
  const sections = sectionIds
    .map((id) => container.querySelector<HTMLElement>(`#${id}`))
    .filter((section): section is HTMLElement => Boolean(section));
  if (!sections.length) return null;
  if (container.scrollHeight <= container.clientHeight) return sections[0].id;
  if (container.scrollTop + container.clientHeight >= container.scrollHeight - 1) return sections[sections.length - 1].id;
  const contentTop = settingsSectionContentTop(container);
  const activationLine = contentTop + Math.min(240, Math.max(72, container.clientHeight * 0.4));
  return [...sections].reverse().find((section) => section.getBoundingClientRect().top <= activationLine)?.id ?? sections[0].id;
}

const settingsControl = cn(
  "min-h-[var(--zc-control-height-current)] rounded-[var(--zc-radius-control)] border border-[var(--zc-control-border)] bg-[var(--zc-surface)] px-3 text-sm text-[var(--zc-text-primary)]",
  "transition-[background,border-color,box-shadow,color] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)]",
  "hover:border-[var(--zc-control-border-hover)] focus:border-[var(--zc-primary)] focus:bg-[var(--zc-surface)] focus:shadow-none",
  focusVisibleState,
  "disabled:cursor-not-allowed disabled:border-[var(--zc-control-border)] disabled:bg-[var(--zc-surface-subtle)] disabled:text-[var(--zc-text-disabled)] disabled:opacity-70"
);

export const settingsField = settingsControl;
export const settingsSelect = settingsControl;

function normalizedSearchText(value: string | null | undefined) {
  return value?.trim().toLocaleLowerCase() ?? "";
}

function settingsSearchElementText(element: HTMLElement) {
  const label = element.getAttribute("data-settings-search-label") ?? "";
  const description = element.getAttribute("data-settings-search-description") ?? "";
  return normalizedSearchText(`${label} ${description}`);
}

function settingsSearchTargetId(element: HTMLElement) {
  if (element.matches("button[id], input[id], select[id], textarea[id]") && element.id) return element.id;
  const nestedControl = element.querySelector<HTMLElement>("button[id], input[id], select[id], textarea[id]");
  return nestedControl?.id || element.id || undefined;
}

export function collectSettingsSearchResults(
  container: HTMLElement | null,
  sections: readonly SettingsSectionOption[],
  query: string
): SettingsSearchResult[] {
  const term = normalizedSearchText(query);
  if (!container || !term) return [];

  const results: SettingsSearchResult[] = [];
  for (const sectionOption of sections) {
    const section = container.querySelector<HTMLElement>(`#${sectionOption.id}`);
    if (!section) continue;
    const sectionTitle = section.getAttribute("data-settings-search-label") ?? sectionOption.label;
    const sectionDescription = section.getAttribute("data-settings-search-description") ?? undefined;
    const sectionMatches = normalizedSearchText(`${sectionTitle} ${sectionDescription ?? ""}`).includes(term);
    const candidates = [...section.querySelectorAll<HTMLElement>("[data-settings-search-label]")]
      .filter((element) => element !== section)
      .filter((element) => settingsSearchElementText(element).includes(term))
      .sort((left, right) => {
        const leftLabelMatch = normalizedSearchText(left.getAttribute("data-settings-search-label")).includes(term);
        const rightLabelMatch = normalizedSearchText(right.getAttribute("data-settings-search-label")).includes(term);
        return Number(rightLabelMatch) - Number(leftLabelMatch);
      });

    if (sectionMatches) {
      results.push({
        sectionId: sectionOption.id,
        sectionLabel: sectionOption.label,
        title: sectionTitle,
        description: sectionDescription,
        targetId: `${sectionOption.id}-heading`
      });
    }
    for (const candidate of candidates) {
      const title = candidate.getAttribute("data-settings-search-label") ?? sectionTitle;
      const description = candidate.getAttribute("data-settings-search-description") ?? undefined;
      results.push({
        sectionId: sectionOption.id,
        sectionLabel: sectionOption.label,
        title,
        description,
        targetId: settingsSearchTargetId(candidate) ?? `${sectionOption.id}-heading`
      });
    }
  }
  return results;
}

export function SettingsSearch({
  containerRef,
  sections,
  onSelect,
  label,
  placeholder,
  clearLabel,
  noResultsLabel,
  resultCountLabel,
  localeKey
}: {
  containerRef: RefObject<HTMLDivElement | null>;
  sections: readonly SettingsSectionOption[];
  onSelect: (result: SettingsSearchResult) => void;
  label: string;
  placeholder: string;
  clearLabel: string;
  noResultsLabel: string;
  resultCountLabel: (count: number, query: string) => string;
  localeKey?: string;
}) {
  const inputRef = useRef<HTMLInputElement | null>(null);
  const resultRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(-1);
  const results = useMemo(
    () => collectSettingsSearchResults(containerRef.current, sections, query),
    [containerRef, localeKey, query, sections]
  );

  useEffect(() => {
    setActiveIndex(-1);
  }, [query, localeKey]);

  const clear = useCallback(() => {
    setQuery("");
    setActiveIndex(-1);
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  function focusResult(index: number) {
    if (!results.length) return;
    const nextIndex = Math.max(0, Math.min(index, results.length - 1));
    setActiveIndex(nextIndex);
    window.requestAnimationFrame(() => resultRefs.current[nextIndex]?.focus({ preventScroll: true }));
  }

  function selectResult(result: SettingsSearchResult) {
    setQuery("");
    setActiveIndex(-1);
    onSelect(result);
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Escape") {
      if (query) {
        event.preventDefault();
        clear();
      }
      return;
    }
    if (!results.length) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusResult(activeIndex < 0 ? 0 : activeIndex + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      focusResult(activeIndex < 0 ? results.length - 1 : activeIndex - 1);
    } else if (event.key === "Home") {
      event.preventDefault();
      focusResult(0);
    } else if (event.key === "End") {
      event.preventDefault();
      focusResult(results.length - 1);
    } else if (event.key === "Enter") {
      // §8.5: Enter opens the first result (or the highlighted one).
      event.preventDefault();
      selectResult(results[activeIndex >= 0 ? activeIndex : 0]);
    }
  }

  function handleResultKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      focusResult(index + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      focusResult(index - 1);
    } else if (event.key === "Home") {
      event.preventDefault();
      focusResult(0);
    } else if (event.key === "End") {
      event.preventDefault();
      focusResult(results.length - 1);
    } else if (event.key === "Escape") {
      event.preventDefault();
      setActiveIndex(-1);
      inputRef.current?.focus({ preventScroll: true });
    } else if (event.key === "Enter" || event.key === " " || event.key === "Spacebar") {
      event.preventDefault();
      selectResult(results[index]);
    }
  }

  const hasQuery = Boolean(query.trim());
  return (
    <div data-settings-search className="relative w-full max-w-[360px]" role="search">
      <div className={cn(
        "flex min-h-[var(--zc-control-height-current)] items-center gap-2 rounded-[var(--zc-radius-control)] border border-transparent bg-[var(--zc-surface-subtle)] px-3",
        "transition-[background,border-color,box-shadow] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)]",
        "hover:bg-[var(--zc-surface-hover)]",
        focusWithinSurface
      )}>
        <Search size={15} aria-hidden="true" className="shrink-0 text-[var(--zc-text-tertiary)]" />
        <input
          ref={inputRef}
          type="search"
          role="searchbox"
          value={query}
          aria-label={label}
          aria-controls="settings-search-results"
          aria-expanded={hasQuery}
          placeholder={placeholder}
          data-settings-search-input
          className="min-w-0 flex-1 bg-transparent text-sm text-[var(--zc-text-primary)] outline-none placeholder:text-[var(--zc-text-tertiary)]"
          onChange={(event) => {
            setQuery(event.target.value);
            setActiveIndex(-1);
          }}
          onKeyDown={handleInputKeyDown}
        />
        {hasQuery ? (
          <button
            type="button"
            data-settings-search-clear
            aria-label={clearLabel}
            title={clearLabel}
            className={cn("grid h-7 w-7 shrink-0 place-items-center rounded-[var(--zc-radius-control)] text-[var(--zc-text-tertiary)] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState)}
            onClick={clear}
          >
            <X size={14} aria-hidden="true" />
          </button>
        ) : null}
      </div>
      {hasQuery ? (
        <div
          id="settings-search-results"
          data-settings-search-results
          className="absolute right-0 top-[calc(100%+0.5rem)] z-40 grid max-h-[min(24rem,calc(100vh-2rem))] w-full gap-1 overflow-y-auto overscroll-contain rounded-[var(--zc-radius-floating)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-1.5 shadow-[var(--zc-shadow-menu)]"
        >
          {results.length ? (
            <>
              <p className="px-2 py-1 text-xs text-[var(--zc-text-tertiary)]" aria-live="polite">{resultCountLabel(results.length, query)}</p>
              {results.map((result, index) => (
                <button
                  key={`${result.sectionId}-${result.title}-${index}`}
                  ref={(element) => { resultRefs.current[index] = element; }}
                  type="button"
                  data-settings-search-result={result.sectionId}
                  data-settings-search-target={result.targetId}
                  className={cn(
                    "grid min-w-0 gap-0.5 rounded-[var(--zc-radius-control)] px-3 py-2 text-left transition-[background,color] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)]",
                    "hover:bg-[var(--zc-surface-hover)]",
                    focusVisibleState,
                    activeIndex === index && "bg-[var(--zc-surface-hover)]"
                  )}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={() => selectResult(result)}
                  onKeyDown={(event) => handleResultKeyDown(event, index)}
                >
                  <span className="truncate text-sm font-medium text-[var(--zc-text-primary)]">{result.title}</span>
                  <span className="truncate text-xs text-[var(--zc-text-tertiary)]">{result.description || result.sectionLabel}</span>
                </button>
              ))}
            </>
          ) : <p data-settings-search-no-results className="px-3 py-3 text-sm text-[var(--zc-text-secondary)]">{noResultsLabel}</p>}
        </div>
      ) : null}
      {hasQuery && !results.length ? <span className="sr-only" aria-live="polite">{noResultsLabel}</span> : null}
    </div>
  );
}

export function SettingsLayout({
  sections,
  activeSectionId,
  onSectionChange,
  scrollRef,
  sectionLabel,
  header,
  children
}: {
  sections: SettingsSectionOption[];
  activeSectionId: string;
  onSectionChange: (sectionId: string, options?: SectionChangeOptions) => void;
  scrollRef?: RefObject<HTMLDivElement | null>;
  sectionLabel: string;
  header?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div data-settings-layout className="flex h-full min-h-0 min-w-0 flex-col">
      {header ? <div data-settings-page-header className="mx-auto flex w-full max-w-[1240px] shrink-0 justify-end px-1 pb-3">{header}</div> : null}
      <div ref={scrollRef} data-settings-scroll-container className="min-h-0 min-w-0 flex-1 overflow-auto overscroll-contain pr-1">
        <div
          data-settings-layout-grid
          className="mx-auto grid w-full max-w-[1240px] min-w-0 gap-5 px-1 pb-8 min-[841px]:grid-cols-[200px_minmax(0,1fr)] min-[841px]:items-start min-[841px]:gap-[clamp(2rem,3vw,2.75rem)]"
        >
          <SettingsSectionNav
            sections={sections}
            activeSectionId={activeSectionId}
            onSectionChange={onSectionChange}
            sectionLabel={sectionLabel}
          />
          <div data-settings-content className="grid min-w-0 max-w-[800px] gap-7">{children}</div>
        </div>
      </div>
    </div>
  );
}

export function SettingsSectionNav({
  sections,
  activeSectionId,
  onSectionChange,
  sectionLabel
}: {
  sections: SettingsSectionOption[];
  activeSectionId: string;
  onSectionChange: (sectionId: string, options?: SectionChangeOptions) => void;
  sectionLabel: string;
}) {
  // Every mounted section owns its own navigation item. Technical sections
  // remain quiet disclosures, but they must not be projected into another
  // section or disappear from the active navigation model.
  const visibleSections = sections;
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const navRef = useRef<HTMLElement | null>(null);
  const activeIndex = visibleSections.findIndex((section) => section.id === activeSectionId);

  useEffect(() => {
    if (activeIndex < 0) return;
    centerSettingsNavItem(navRef.current, buttonRefs.current[activeIndex]);
  }, [activeIndex]);

  function handleWheel(event: WheelEvent<HTMLElement>) {
    const nav = navRef.current;
    if (!nav || nav.scrollWidth <= nav.clientWidth) return;
    const delta = Math.abs(event.deltaX) >= Math.abs(event.deltaY) ? event.deltaX : event.deltaY;
    if (!delta) return;
    event.preventDefault();
    nav.scrollLeft += delta;
  }

  function moveFocus(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const isNext = event.key === "ArrowRight" || event.key === "ArrowDown";
    const isPrevious = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!isNext && !isPrevious && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? visibleSections.length - 1
        : (index + (isNext ? 1 : -1) + visibleSections.length) % visibleSections.length;
    const next = visibleSections[nextIndex];
    onSectionChange(next.id, { focusContent: false });
    window.requestAnimationFrame(() => buttonRefs.current[nextIndex]?.focus());
  }

  return (
    <aside
      data-settings-section-nav-shell
      className="sticky top-0 z-20 min-w-0 border-b border-[var(--zc-divider)] bg-[var(--zc-surface)] py-2 min-[841px]:top-4 min-[841px]:self-start min-[841px]:border-b-0 min-[841px]:bg-transparent min-[841px]:py-0"
    >
      <p className="mb-2 hidden px-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--zc-text-tertiary)] min-[841px]:block">
        {sectionLabel}
      </p>
      <div className="relative min-w-0">
        <nav
          ref={navRef}
          aria-label={sectionLabel}
          data-settings-section-nav
          className="flex max-w-full scroll-px-5 gap-1 overflow-x-auto overscroll-contain px-5 pb-1 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden min-[841px]:grid min-[841px]:overflow-visible min-[841px]:px-0 min-[841px]:pb-0"
          onWheel={handleWheel}
        >
          {visibleSections.map((section, index) => {
            const active = activeSectionId === section.id;
            return (
            <button
              key={section.id}
              ref={(element) => { buttonRefs.current[index] = element; }}
              type="button"
              data-settings-section={section.id}
              aria-current={active ? "location" : undefined}
              tabIndex={active ? 0 : -1}
              className={cn(
                "min-h-9 min-w-0 shrink-0 whitespace-nowrap rounded-[var(--zc-radius-control)] border border-transparent px-3 py-2 text-left text-sm font-medium leading-5 text-[var(--zc-text-secondary)]",
                "min-[841px]:w-full min-[841px]:whitespace-normal",
                "transition-[background,border-color,color] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)]",
                "hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]",
                active ? selectedFocusVisibleState : focusVisibleState,
                active && selectedSurface
              )}
              onClick={() => onSectionChange(section.id)}
              onKeyDown={(event) => moveFocus(event, index)}
            >
              {section.label}
            </button>
            );
          })}
        </nav>
        <span data-settings-nav-fade="start" aria-hidden="true" className="pointer-events-none absolute inset-y-0 left-0 w-5 bg-gradient-to-r from-[var(--zc-surface)] to-transparent min-[841px]:hidden" />
        <span data-settings-nav-fade="end" aria-hidden="true" className="pointer-events-none absolute inset-y-0 right-0 w-5 bg-gradient-to-l from-[var(--zc-surface)] to-transparent min-[841px]:hidden" />
      </div>
    </aside>
  );
}

export function SettingsSection({
  id,
  title,
  description,
  children,
  progressiveDisclosure = false,
  className
}: {
  id: string;
  title: string;
  description?: string;
  children: ReactNode;
  progressiveDisclosure?: boolean;
  className?: string;
}) {
  const sectionClass = cn("grid min-w-0 gap-[var(--zc-density-gap)] border-b border-[var(--zc-divider)] pb-7 outline-none last:border-b-0", className);
  if (progressiveDisclosure) {
    return (
      <section
        id={id}
        tabIndex={-1}
        aria-labelledby={`${id}-heading`}
        data-settings-section-content
        data-settings-progressive-section
        data-settings-search-label={title}
        data-settings-search-description={description}
        className={sectionClass}
      >
        <details data-settings-progressive-disclosure className="group grid min-w-0 gap-[var(--zc-density-gap)]">
          <summary className={cn("flex cursor-pointer list-none items-start justify-between gap-4 rounded-[var(--zc-radius-control)] py-1", focusVisibleState)}>
            <span className="grid min-w-0 gap-1">
              <h2 id={`${id}-heading`} data-settings-section-heading tabIndex={-1} className="text-base font-semibold tracking-[-0.01em] text-[var(--zc-text-primary)] outline-none">
                {title}
              </h2>
              {description ? <span className="max-w-2xl text-sm font-normal leading-6 text-[var(--zc-text-secondary)]">{description}</span> : null}
            </span>
            <span aria-hidden="true" className="mt-0.5 text-[var(--zc-text-tertiary)] transition-transform group-open:rotate-90">›</span>
          </summary>
          <div className="grid min-w-0 gap-0 pt-1">{children}</div>
        </details>
      </section>
    );
  }

  return (
    <section
      id={id}
      tabIndex={-1}
      aria-labelledby={`${id}-heading`}
      data-settings-section-content
      data-settings-search-label={title}
      data-settings-search-description={description}
      className={sectionClass}
    >
      <header className="grid gap-1">
        <h2 id={`${id}-heading`} data-settings-section-heading tabIndex={-1} className="text-lg font-semibold tracking-[-0.01em] text-[var(--zc-text-primary)] outline-none">
          {title}
        </h2>
        {description ? <p className="max-w-2xl text-sm leading-6 text-[var(--zc-text-secondary)]">{description}</p> : null}
      </header>
      <div className="grid min-w-0 gap-0">{children}</div>
    </section>
  );
}

export function SettingsControlGroup({
  title,
  description,
  children,
  className
}: {
  title?: string;
  description?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      data-settings-search-label={title}
      data-settings-search-description={description}
      className={cn("grid min-w-0 gap-[var(--zc-density-gap)] border-t border-[var(--zc-divider)] pt-5 first:border-t-0 first:pt-0", className)}
    >
      {title ? <h3 className="text-sm font-semibold text-[var(--zc-text-primary)]">{title}</h3> : null}
      {description ? <p className="max-w-2xl text-sm leading-6 text-[var(--zc-text-secondary)]">{description}</p> : null}
      <div className="grid min-w-0 gap-0">{children}</div>
    </div>
  );
}

export function SettingsRow({
  id,
  label,
  description,
  hint,
  children,
  className,
  controlWidth = "default"
}: {
  id?: string;
  label: string;
  description?: string;
  hint?: string;
  children: ReactNode;
  className?: string;
  controlWidth?: "default" | "wide";
}) {
  return (
    <div
      data-settings-row
      data-settings-search-label={label}
      data-settings-search-description={[description, hint].filter(Boolean).join(" ") || undefined}
      className={cn(
        "grid min-w-0 gap-[var(--zc-density-gap)] border-b border-[var(--zc-divider)] py-4 last:border-b-0 min-[841px]:items-start",
        controlWidth === "wide"
          ? "min-[841px]:grid-cols-[minmax(220px,1fr)_minmax(0,480px)]"
          : "min-[841px]:grid-cols-[minmax(0,1fr)_minmax(0,360px)]",
        className
      )}
    >
      <div className="min-w-0">
        {id ? (
          <label htmlFor={id} className="block text-sm font-medium text-[var(--zc-text-primary)]">{label}</label>
        ) : (
          <strong className="block text-sm font-medium text-[var(--zc-text-primary)]">{label}</strong>
        )}
        {description ? <span className="mt-1 block max-w-[600px] text-sm leading-6 text-[var(--zc-text-secondary)]">{description}</span> : null}
        {hint ? <span className="mt-1 block max-w-[600px] text-xs leading-5 text-[var(--zc-text-tertiary)]">{hint}</span> : null}
      </div>
      <div className={cn(
        "min-w-0 min-[841px]:w-full min-[841px]:justify-self-end",
        controlWidth === "wide" ? "min-[841px]:max-w-[480px]" : "min-[841px]:max-w-[360px]"
      )}>{children}</div>
    </div>
  );
}

export function SettingsSegmentedControl<T extends string>({
  value,
  options,
  ariaLabel,
  onChange,
  disabled = false,
  layout = "wrap"
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  ariaLabel: string;
  onChange: (value: T) => void;
  disabled?: boolean;
  layout?: "wrap" | "three-option-responsive";
}) {
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);

  function handleKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    if (disabled) return;
    const isNext = event.key === "ArrowRight" || event.key === "ArrowDown";
    const isPrevious = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!isNext && !isPrevious && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? options.length - 1
        : (index + (isNext ? 1 : -1) + options.length) % options.length;
    const next = options[nextIndex];
    onChange(next.value);
    window.requestAnimationFrame(() => buttonRefs.current[nextIndex]?.focus());
  }

  return (
    <div
      role="radiogroup"
      data-settings-segmented-control
      aria-label={ariaLabel}
      aria-disabled={disabled || undefined}
      className={cn(
        "max-w-full gap-1 rounded-[var(--zc-radius-control)] border border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] p-1",
        layout === "three-option-responsive" ? "grid grid-cols-1 min-[841px]:grid-cols-3" : "flex flex-wrap",
        disabled && "cursor-not-allowed opacity-60"
      )}
    >
      {options.map((option, index) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            ref={(element) => { buttonRefs.current[index] = element; }}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={disabled}
            tabIndex={disabled ? -1 : selected ? 0 : -1}
            className={cn(
              "min-h-[var(--zc-control-height-current)] min-w-0 shrink-0 rounded-[var(--zc-radius-control)] px-3 py-1.5 text-sm font-medium text-[var(--zc-text-secondary)]",
              layout === "three-option-responsive" ? "w-full whitespace-normal text-center leading-5" : "whitespace-nowrap",
              "transition-[background,color] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)]",
              "hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]",
              "disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-[var(--zc-text-secondary)]",
              selected ? selectedFocusVisibleState : focusVisibleState,
              selected && cn(selectedSurface, "font-semibold")
            )}
            onClick={() => { if (!disabled) onChange(option.value); }}
            onKeyDown={(event) => handleKeyDown(event, index)}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

export function SettingsSwitch({
  id,
  label,
  description,
  checked,
  onChange,
  disabled = false,
  className
}: {
  id: string;
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <SettingsRow id={id} label={label} description={description} className={className}>
      <SettingsSwitchControl id={id} label={label} checked={checked} onChange={onChange} disabled={disabled} />
    </SettingsRow>
  );
}

export function SettingsSwitchControl({
  id,
  label,
  checked,
  onChange,
  disabled = false
}: {
  id: string;
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label
      htmlFor={id}
      data-settings-switch-control
      className={cn(
        "relative flex min-h-[var(--zc-control-height-current)] w-fit items-center justify-end justify-self-end",
        disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer"
      )}
    >
      <input
        id={id}
        type="checkbox"
        role="switch"
        className="peer sr-only"
        checked={checked}
        disabled={disabled}
        aria-checked={checked}
        aria-label={label}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span
        aria-hidden="true"
        data-settings-switch-track
        className={cn(
          switchTrack,
          "peer-checked:border-[var(--zc-primary)] peer-checked:bg-[var(--zc-primary-soft)]",
          "peer-focus-visible:bg-[var(--zc-focus-soft)] peer-focus-visible:border-[var(--zc-focus)]",
          "peer-disabled:cursor-not-allowed peer-disabled:!border-[var(--zc-control-border)] peer-disabled:!bg-[var(--zc-surface-subtle)]"
        )}
      />
      <span
        aria-hidden="true"
        data-settings-switch-thumb
        className={cn(switchThumb, "peer-checked:translate-x-4 peer-checked:bg-[var(--zc-primary)] peer-disabled:!bg-[var(--zc-control-border)]")}
      />
    </label>
  );
}

export function SettingsSelect<T extends string>({
  id,
  label,
  description,
  value,
  options,
  onChange,
  disabled = false
}: {
  id: string;
  label: string;
  description?: string;
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  const controlRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({});
  const selectedIndex = Math.max(0, options.findIndex((option) => option.value === value));
  const selectedOption = options.find((option) => option.value === value) ?? options[0];
  const menuId = `${id}-listbox`;

  useEffect(() => {
    if (!open) return undefined;
    setActiveIndex(selectedIndex);

    function updateMenuPosition() {
      const trigger = triggerRef.current;
      if (!trigger) return;
      const rect = trigger.getBoundingClientRect();
      const viewportWidth = Math.max(window.innerWidth || 0, 320);
      const viewportHeight = Math.max(window.innerHeight || 0, 240);
      const width = Math.min(Math.max(rect.width, 176), viewportWidth - 16);
      const availableBelow = Math.max(96, viewportHeight - rect.bottom - 12);
      const availableAbove = Math.max(96, rect.top - 12);
      const maxHeight = Math.min(320, Math.max(availableBelow, availableAbove));
      const openUp = availableBelow < 180 && availableAbove > availableBelow;
      const top = openUp
        ? Math.max(8, rect.top - Math.min(maxHeight, availableAbove) - 4)
        : Math.min(viewportHeight - 8 - Math.min(maxHeight, availableBelow), rect.bottom + 4);
      const left = Math.max(8, Math.min(rect.left, viewportWidth - width - 8));
      setMenuStyle({ left, top, width, maxHeight, position: "fixed" });
    }

    function handleOutsidePointerDown(event: PointerEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (controlRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    }

    function handleOutsideFocus(event: FocusEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (controlRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    }

    updateMenuPosition();
    document.addEventListener("pointerdown", handleOutsidePointerDown);
    document.addEventListener("focusin", handleOutsideFocus);
    window.addEventListener("resize", updateMenuPosition);
    document.addEventListener("scroll", updateMenuPosition, true);
    return () => {
      document.removeEventListener("pointerdown", handleOutsidePointerDown);
      document.removeEventListener("focusin", handleOutsideFocus);
      window.removeEventListener("resize", updateMenuPosition);
      document.removeEventListener("scroll", updateMenuPosition, true);
    };
  }, [open, selectedIndex]);

  useEffect(() => {
    if (!open) return;
    setActiveIndex((current) => Math.min(current, Math.max(0, options.length - 1)));
  }, [open, options.length]);

  function closeAndRestoreFocus() {
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  }

  function chooseOption(index: number) {
    const option = options[index];
    if (!option || disabled) return;
    onChange(option.value);
    closeAndRestoreFocus();
  }

  function handleTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (disabled || !options.length) return;
    const isOpen = open;
    if (event.key === "Escape" && isOpen) {
      event.preventDefault();
      closeAndRestoreFocus();
      return;
    }
    if ((event.key === "Enter" || event.key === " " || event.key === "Spacebar") && !isOpen) {
      event.preventDefault();
      setActiveIndex(selectedIndex);
      setOpen(true);
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!isOpen) {
        setActiveIndex(selectedIndex);
        setOpen(true);
        return;
      }
      const delta = event.key === "ArrowDown" ? 1 : -1;
      setActiveIndex((current) => (current + delta + options.length) % options.length);
      return;
    }
    if (!isOpen) return;
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      setActiveIndex(event.key === "Home" ? 0 : options.length - 1);
    } else if (event.key === "Enter" || event.key === " " || event.key === "Spacebar") {
      event.preventDefault();
      chooseOption(activeIndex);
    }
  }

  function handleTriggerClick() {
    if (disabled || !options.length) return;
    if (open) {
      closeAndRestoreFocus();
    } else {
      setActiveIndex(selectedIndex);
      setOpen(true);
    }
  }

  return (
    <SettingsRow label={label} description={description}>
      <div ref={controlRef} data-settings-select-control className={cn("relative min-w-0", open && "z-30")}>
        <select
          id={id}
          tabIndex={-1}
          aria-hidden="true"
          data-settings-select-native
          className="pointer-events-none absolute h-px w-px overflow-hidden opacity-0"
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value as T)}
        >
          {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
        </select>
        <button
          ref={triggerRef}
          id={`${id}-trigger`}
          type="button"
          role="combobox"
          data-settings-select-trigger
          aria-label={label}
          aria-haspopup="listbox"
          aria-autocomplete="none"
          aria-expanded={open}
          aria-controls={menuId}
          aria-activedescendant={open && options[activeIndex] ? `${id}-option-${activeIndex}` : undefined}
          disabled={disabled || options.length === 0}
          className={cn(settingsSelect, "flex w-full items-center justify-between gap-3 text-left", open && "border-[var(--zc-control-border-hover)] bg-[var(--zc-surface)]")}
          onClick={handleTriggerClick}
          onKeyDown={handleTriggerKeyDown}
        >
          <span data-settings-select-value className="min-w-0 truncate">{selectedOption?.label ?? "—"}</span>
          <ChevronDown size={15} aria-hidden="true" className={cn("shrink-0 text-[var(--zc-text-tertiary)] transition-transform duration-[var(--zc-duration-fast)]", open && "rotate-180")} />
        </button>
        {open && typeof document !== "undefined" ? createPortal(
          <div
            ref={menuRef}
            id={menuId}
            role="listbox"
            aria-label={label}
            data-settings-select-menu
            className="fixed z-[120] grid gap-1 overflow-y-auto overscroll-contain rounded-[var(--zc-radius-floating)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-1.5 shadow-[var(--zc-shadow-menu)]"
            style={menuStyle}
          >
            {options.map((option, index) => {
              const selected = option.value === value;
              const active = index === activeIndex;
              return (
                <button
                  key={option.value}
                  id={`${id}-option-${index}`}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  tabIndex={-1}
                  data-settings-select-option
                  data-active={active || undefined}
                  className={cn(
                    "flex h-[29px] min-h-[29px] w-full items-center justify-between gap-3 rounded-[var(--zc-radius-control)] px-2 py-0 text-left text-[11.75px] text-[var(--zc-text-primary)]",
                    "transition-[background,color,outline] duration-[var(--zc-duration-fast)] ease-[var(--zc-ease-standard)]",
                    "hover:bg-[var(--zc-surface-hover)]",
                    active && (selected ? selectedFocusSurface : focusSurface),
                    selected && !active && selectedSurface,
                    selected && "font-semibold"
                  )}
                  onMouseEnter={() => setActiveIndex(index)}
                  onClick={() => chooseOption(index)}
                >
                  <span className="min-w-0 truncate">{option.label}</span>
                  {selected ? <Check size={14} aria-hidden="true" className="shrink-0 text-[var(--zc-primary-text)]" /> : <span aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />}
                </button>
              );
            })}
          </div>,
          document.body
        ) : null}
      </div>
    </SettingsRow>
  );
}

export function SettingsTextField({
  id,
  label,
  description,
  value,
  onChange,
  type = "text",
  placeholder,
  list,
  min,
  max,
  maxLength,
  disabled = false
}: {
  id?: string;
  label: string;
  description?: string;
  value: string;
  onChange: (value: string) => void;
  type?: string;
  placeholder?: string;
  list?: string;
  min?: number;
  max?: number;
  maxLength?: number;
  disabled?: boolean;
}) {
  return (
    <label data-settings-search-label={label} data-settings-search-description={description} className="grid min-w-0 gap-1.5">
      <span className="text-sm font-medium text-[var(--zc-text-primary)]">{label}</span>
      {description ? <span className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{description}</span> : null}
      <input id={id} className={cn(settingsField, "w-full")} type={type} value={value} placeholder={placeholder} list={list} min={min} max={max} maxLength={maxLength} disabled={disabled} onChange={(event) => onChange(event.target.value)} />
    </label>
  );
}

export function SettingsDisclosure({
  title,
  description,
  children,
  defaultOpen = false,
  open,
  onOpenChange
}: {
  title: string;
  description?: string;
  children: ReactNode;
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  return (
    <details
      open={open ?? (defaultOpen || undefined)}
      onToggle={(event) => onOpenChange?.(event.currentTarget.open)}
      data-settings-search-label={title}
      data-settings-search-description={description}
      className="group grid min-w-0 gap-3 border-t border-[var(--zc-divider)] pt-4"
    >
      <summary className={cn("flex cursor-pointer list-none items-start justify-between gap-3 text-sm font-semibold text-[var(--zc-text-primary)]", focusVisibleState)}>
        <span className="min-w-0">
          <span className="block">{title}</span>
          {description ? <span className="mt-1 block text-xs font-normal leading-5 text-[var(--zc-text-tertiary)]">{description}</span> : null}
        </span>
        <span aria-hidden="true" className="mt-0.5 text-[var(--zc-text-tertiary)] transition-transform group-open:rotate-90">›</span>
      </summary>
      <div className="grid min-w-0 gap-4">{children}</div>
    </details>
  );
}

export function SettingsEmptyState({
  title,
  description,
  action
}: {
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="grid min-h-28 place-items-center gap-3 rounded-[var(--zc-radius-field)] border border-dashed border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] px-5 py-6 text-center">
      <div className="grid max-w-xl gap-1">
        <strong className="text-sm text-[var(--zc-text-primary)]">{title}</strong>
        {description ? <span className="text-sm leading-6 text-[var(--zc-text-secondary)]">{description}</span> : null}
      </div>
      {action ? <div>{action}</div> : null}
    </div>
  );
}

export function SettingsInlineMessage({
  tone = "info",
  children,
  role,
  status
}: {
  tone?: "info" | "success" | "warning" | "danger";
  children: ReactNode;
  role?: "status" | "alert";
  status?: string;
}) {
  const toneClass = tone === "danger"
    ? "border-[var(--zc-danger-border)] bg-[var(--zc-danger-soft)] text-[var(--zc-danger-text)]"
    : tone === "warning"
      ? "border-[var(--zc-warning-border)] bg-[var(--zc-warning-soft)] text-[var(--zc-warning-text)]"
      : tone === "success"
        ? "border-[var(--zc-success-border)] bg-[var(--zc-success-soft)] text-[var(--zc-success-text)]"
        : "border-[var(--zc-info-border)] bg-[var(--zc-info-soft)] text-[var(--zc-info-text)]";
  return <div className={cn("rounded-[var(--zc-radius-field)] border px-3 py-2 text-sm leading-6", toneClass)} role={role} data-settings-status={status}>{children}</div>;
}
