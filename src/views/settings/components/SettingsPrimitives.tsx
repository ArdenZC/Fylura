import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  type WheelEvent
} from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, ChevronRight, Search, X } from "lucide-react";
import { cn, focusSurface, focusVisibleState, selectedFocusSurface, selectedFocusVisibleState, selectedSurface } from "../../../utils/tw";
import { playOverlayEnter, playOverlayExit, prefersReducedMotion } from "../../../utils/motion";
import { switchThumb, switchTrack } from "../../../components/ui/Switch";
import "../settingsV340.css";

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

export function scrollSettingsSectionIntoView(
  container: HTMLElement | null,
  sectionId: string,
  options: SectionChangeOptions = {}
) {
  if (!container) return null;
  const target = container.querySelector<HTMLElement>(`[id="${sectionId}"]`);
  if (!target) return null;
  const section = target.closest<HTMLElement>("[data-settings-section-content]") ?? target;
  const disclosure = target.querySelector<HTMLDetailsElement>("details")
    ?? target.closest<HTMLDetailsElement>("details");
  if (disclosure && (options.revealContent || !disclosure.open)) disclosure.open = true;
  const heading = section.querySelector<HTMLElement>("[data-settings-section-heading]");
  const sectionTarget = target === section;
  const visibleTarget = sectionTarget ? section : target;
  if (options.focusContent !== false) (sectionTarget ? heading ?? target : target).focus({ preventScroll: true });
  const containerRect = container.getBoundingClientRect();
  const targetRect = visibleTarget.getBoundingClientRect();
  if (!sectionTarget && (targetRect.top < containerRect.top + 60 || targetRect.bottom > containerRect.bottom - 24)) {
    container.scrollTop += targetRect.top - containerRect.top - 80;
  }
  return { section, heading };
}

export function centerSettingsNavItem(nav: HTMLElement | null, item: HTMLElement | null) {
  if (!nav || !item || nav.scrollWidth <= nav.clientWidth) return;
  const target = item.offsetLeft - (nav.clientWidth - item.offsetWidth) / 2;
  nav.scrollLeft = Math.max(0, Math.min(target, nav.scrollWidth - nav.clientWidth));
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

export type SettingsSearchCopy = {
  label: string;
  placeholder: string;
  clearLabel: string;
  escHint: string;
  title: string;
  summary: (sections: number, items: number) => string;
  resultLead: (count: number, sections: number) => string;
  noMatch: (query: string) => string;
  suggestionsLabel: string;
  suggestions: string[];
  itemCount: (count: number) => string;
};

type SettingsPageContextValue = {
  activeSectionId: string;
  counts: Record<string, number>;
  itemCount: (count: number) => string;
};

const SettingsPageContext = createContext<SettingsPageContextValue | null>(null);

function settingsSectionCount(container: HTMLElement | null, sectionId: string) {
  const section = container?.querySelector<HTMLElement>(`[id="${sectionId}"]`);
  if (!section) return 0;
  return section.querySelectorAll<HTMLElement>("[data-settings-row], [data-settings-item]").length;
}

function sameCounts(left: Record<string, number>, right: Record<string, number>) {
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) return false;
  return leftKeys.every((key) => left[key] === right[key]);
}

function settingsHighlightedText(text: string, query: string) {
  const term = query.trim();
  if (!term) return text;
  const index = text.toLocaleLowerCase().indexOf(term.toLocaleLowerCase());
  if (index < 0) return text;
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + term.length)}</mark>
      {text.slice(index + term.length)}
    </>
  );
}

export function SettingsSearchField({
  value,
  onChange,
  onEnter,
  onArrowDown,
  onEscape,
  onClear,
  label,
  placeholder,
  clearLabel,
  inputRef
}: {
  value: string;
  onChange: (value: string) => void;
  onEnter?: () => void;
  onArrowDown?: () => void;
  onEscape?: () => void;
  onClear: () => void;
  label: string;
  placeholder: string;
  clearLabel: string;
  inputRef?: RefObject<HTMLInputElement | null>;
}) {
  const hasQuery = Boolean(value.trim());
  return (
    <div data-settings-search className="settings-v340-search" role="search">
      <Search size={14} aria-hidden="true" className="settings-v340-search-icon" />
      <input
        ref={inputRef}
        type="search"
        role="searchbox"
        value={value}
        aria-label={label}
        aria-controls={hasQuery ? "settings-search-results-page" : undefined}
        aria-expanded={hasQuery}
        placeholder={placeholder}
        data-settings-search-input
        className="settings-v340-search-input"
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onEscape?.();
            return;
          }
          if (event.key === "Enter" && hasQuery) {
            event.preventDefault();
            onEnter?.();
            return;
          }
          if (event.key === "ArrowDown" && hasQuery) {
            event.preventDefault();
            onArrowDown?.();
          }
        }}
      />
      {hasQuery ? (
        <button
          type="button"
          data-settings-search-clear
          aria-label={clearLabel}
          title={clearLabel}
          className="settings-v340-search-clear"
          onClick={onClear}
        >
          <X size={11} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}

function SettingsSearchResults({
  query,
  results,
  copy,
  onSelect,
  onSuggestion,
  onEscape,
  resultRef
}: {
  query: string;
  results: SettingsSearchResult[];
  copy: SettingsSearchCopy;
  onSelect: (result: SettingsSearchResult) => void;
  onSuggestion: (suggestion: string) => void;
  onEscape: () => void;
  resultRef: RefObject<HTMLDivElement | null>;
}) {
  const sectionCount = new Set(results.map((result) => result.sectionId)).size;
  return (
    <section
      id="settings-search-results-page"
      data-settings-search-results
      aria-labelledby="settings-search-results-heading"
      className="settings-v340-results"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          onEscape();
        }
      }}
    >
      <header className="settings-v340-section-head">
        <div className="settings-v340-section-head-text">
          <h2 id="settings-search-results-heading" tabIndex={-1} data-settings-search-heading>
            {copy.title}
            {results.length ? <span className="settings-v340-count">{copy.itemCount(results.length)}</span> : null}
          </h2>
          <p className="settings-v340-lead" aria-live="polite">
            {results.length ? copy.resultLead(results.length, sectionCount) : copy.noMatch(query.trim())}
          </p>
        </div>
      </header>
      {results.length ? (
        <div ref={resultRef} data-settings-search-result-list className="settings-v340-result-list">
          {results.map((result, index) => (
            <button
              key={`${result.sectionId}-${result.title}-${index}`}
              type="button"
              data-settings-search-result={result.sectionId}
              data-settings-search-target={result.targetId}
              className="settings-v340-result"
              onClick={() => onSelect(result)}
            >
              <span className="settings-v340-result-module">{result.sectionLabel}</span>
              <span className="settings-v340-result-main">
                <strong>{settingsHighlightedText(result.title, query)}</strong>
                {result.description ? <span>{settingsHighlightedText(result.description, query)}</span> : null}
              </span>
              <ChevronRight size={14} aria-hidden="true" className="settings-v340-result-arrow" />
            </button>
          ))}
        </div>
      ) : (
        <div data-settings-search-no-results className="settings-v340-no-results">
          <span>{copy.suggestionsLabel}</span>
          <div className="settings-v340-suggestions">
            {copy.suggestions.map((suggestion) => (
              <button key={suggestion} type="button" data-settings-search-suggestion={suggestion} onClick={() => onSuggestion(suggestion)}>
                {suggestion}
              </button>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}

export function SettingsLayout({
  sections,
  activeSectionId,
  onSectionChange,
  scrollRef,
  sectionLabel,
  search,
  children
}: {
  sections: SettingsSectionOption[];
  activeSectionId: string;
  onSectionChange: (sectionId: string, options?: SectionChangeOptions) => void;
  scrollRef?: RefObject<HTMLDivElement | null>;
  sectionLabel: string;
  search?: SettingsSearchCopy;
  children: ReactNode;
}) {
  const localScrollRef = useRef<HTMLDivElement | null>(null);
  const ownerRef = scrollRef ?? localScrollRef;
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const resultsRef = useRef<HTMLDivElement | null>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SettingsSearchResult[]>([]);
  const [pendingFocus, setPendingFocus] = useState<{ targetId: string; nonce: number } | null>(null);
  const focusNonceRef = useRef(0);
  const [counts, setCounts] = useState<Record<string, number>>({});
  const [scrolled, setScrolled] = useState(false);
  const [moreBelow, setMoreBelow] = useState(false);
  const navButtonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const sectionKey = sections.map((section) => section.id).join("|");
  const searching = Boolean(query.trim());
  const totalItems = Object.values(counts).reduce((total, count) => total + count, 0);

  useLayoutEffect(() => {
    const container = ownerRef.current;
    if (!container) return undefined;
    const measure = () => {
      const next: Record<string, number> = {};
      for (const section of sections) next[section.id] = settingsSectionCount(container, section.id);
      setCounts((current) => sameCounts(current, next) ? current : next);
    };
    measure();
    const observer = new MutationObserver(measure);
    observer.observe(container, { childList: true, subtree: true });
    return () => observer.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sectionKey]);

  useLayoutEffect(() => {
    const container = ownerRef.current;
    if (!container || !searching) {
      setResults([]);
      return;
    }
    setResults(collectSettingsSearchResults(container, sections, query));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query, searching, sectionKey, totalItems, search?.label]);

  useLayoutEffect(() => {
    const container = ownerRef.current;
    if (!container) return undefined;
    const update = () => {
      setScrolled(container.scrollTop > 1);
      setMoreBelow(container.scrollTop + container.clientHeight < container.scrollHeight - 2);
    };
    update();
    container.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(container);
    return () => {
      container.removeEventListener("scroll", update);
      observer.disconnect();
    };
  }, [ownerRef, searching]);

  useLayoutEffect(() => {
    const container = ownerRef.current;
    if (!container) return;
    container.scrollTop = 0;
  }, [activeSectionId, searching, ownerRef]);

  useEffect(() => {
    if (!pendingFocus) return undefined;
    const { targetId } = pendingFocus;
    const frame = window.requestAnimationFrame(() => {
      const container = ownerRef.current;
      if (!container) return;
      const target = container.querySelector<HTMLElement>(`[id="${targetId}"]`);
      if (!target) return;
      const disclosure = target.closest<HTMLDetailsElement>("details");
      if (disclosure && !disclosure.open) disclosure.open = true;
      const row = target.closest<HTMLElement>("[data-settings-row]") ?? target;
      row.setAttribute("data-settings-flash", "true");
      window.setTimeout(() => row.removeAttribute("data-settings-flash"), 1800);
      const control = target.matches("button, input, select, textarea, [tabindex]")
        ? target
        : target.querySelector<HTMLElement>("button, input, select, textarea, [tabindex]");
      const focusTarget = control?.matches("[data-settings-select-native]")
        ? control.parentElement?.querySelector<HTMLElement>("[data-settings-select-trigger]")
        : control;
      const finalTarget = focusTarget && !focusTarget.matches(":disabled")
        ? focusTarget
        : target.closest<HTMLElement>("[data-settings-section-content]")?.querySelector<HTMLElement>("[data-settings-section-heading]");
      finalTarget?.focus({ preventScroll: true });
      const targetRect = target.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      if (targetRect.top < containerRect.top + 72 || targetRect.bottom > containerRect.bottom - 24) {
        container.scrollTop += targetRect.top - containerRect.top - 96;
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [pendingFocus, ownerRef]);

  function changeSection(sectionId: string, options?: SectionChangeOptions) {
    if (query) setQuery("");
    onSectionChange(sectionId, options);
  }

  function selectSearchResult(result: SettingsSearchResult) {
    setQuery("");
    focusNonceRef.current += 1;
    setPendingFocus({ targetId: result.targetId ?? `${result.sectionId}-heading`, nonce: focusNonceRef.current });
    onSectionChange(result.sectionId, { focusContent: false });
  }

  function clearSearch() {
    setQuery("");
    searchInputRef.current?.focus({ preventScroll: true });
  }

  const contextValue = useMemo<SettingsPageContextValue>(() => ({
    activeSectionId,
    counts,
    itemCount: search?.itemCount ?? ((count: number) => String(count))
  }), [activeSectionId, counts, search?.itemCount]);

  return (
    <div data-settings-v340 data-settings-layout>
      <aside data-settings-section-nav-shell>
        <nav
          aria-label={sectionLabel}
          data-settings-section-nav
          className="settings-v340-nav"
        >
          {sections.map((section, index) => {
            const active = activeSectionId === section.id;
            return (
              <button
                key={section.id}
                ref={(element) => { navButtonRefs.current[index] = element; }}
                type="button"
                data-settings-section={section.id}
                aria-current={!searching && active ? "location" : undefined}
                tabIndex={active ? 0 : -1}
                className={cn("settings-v340-nav-item", !searching && active && "is-active")}
                onClick={() => changeSection(section.id)}
                onKeyDown={(event) => moveNavFocus(event, index)}
              >
                {section.label}
              </button>
            );
          })}
        </nav>
      </aside>
      <div data-settings-pane className="settings-v340-pane">
        {search ? (
          <div data-settings-head className="settings-v340-head">
            <SettingsSearchField
              value={query}
              onChange={setQuery}
              onClear={clearSearch}
              onEscape={clearSearch}
              onEnter={() => { if (results[0]) selectSearchResult(results[0]); }}
              onArrowDown={() => resultsRef.current?.querySelector<HTMLButtonElement>("[data-settings-search-result]")?.focus({ preventScroll: true })}
              label={search.label}
              placeholder={search.placeholder}
              clearLabel={search.clearLabel}
              inputRef={searchInputRef}
            />
            <span data-settings-head-hint className="settings-v340-hint" aria-live="polite">
              {searching ? search.escHint : search.summary(sections.length, totalItems)}
            </span>
          </div>
        ) : null}
        <div
          ref={ownerRef}
          data-settings-scroll-container
          data-scrolled={scrolled ? "true" : undefined}
          data-more-below={moreBelow ? "true" : undefined}
          className="settings-v340-scroll"
        >
          <div data-settings-sections hidden={searching || undefined} className="settings-v340-sections">
            <SettingsPageContext.Provider value={contextValue}>
              {children}
            </SettingsPageContext.Provider>
          </div>
          {searching && search ? (
            <SettingsSearchResults
              query={query}
              results={results}
              copy={search}
              onSelect={selectSearchResult}
              onSuggestion={(suggestion) => {
                setQuery(suggestion);
                searchInputRef.current?.focus({ preventScroll: true });
              }}
              onEscape={clearSearch}
              resultRef={resultsRef}
            />
          ) : null}
        </div>
      </div>
    </div>
  );

  function moveNavFocus(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const isNext = event.key === "ArrowRight" || event.key === "ArrowDown";
    const isPrevious = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!isNext && !isPrevious && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? sections.length - 1
        : (index + (isNext ? 1 : -1) + sections.length) % sections.length;
    const next = sections[nextIndex];
    changeSection(next.id, { focusContent: false });
    window.requestAnimationFrame(() => navButtonRefs.current[nextIndex]?.focus());
  }
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
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const navRef = useRef<HTMLElement | null>(null);
  const activeIndex = sections.findIndex((section) => section.id === activeSectionId);

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
        ? sections.length - 1
        : (index + (isNext ? 1 : -1) + sections.length) % sections.length;
    onSectionChange(sections[nextIndex].id, { focusContent: false });
    window.requestAnimationFrame(() => buttonRefs.current[nextIndex]?.focus());
  }

  return (
    <aside data-settings-section-nav-shell className="settings-v340-nav-shell">
      <nav
        ref={navRef}
        aria-label={sectionLabel}
        data-settings-section-nav
        className="settings-v340-nav"
        onWheel={handleWheel}
      >
        {sections.map((section, index) => {
          const active = activeSectionId === section.id;
          return (
            <button
              key={section.id}
              ref={(element) => { buttonRefs.current[index] = element; }}
              type="button"
              data-settings-section={section.id}
              aria-current={active ? "location" : undefined}
              tabIndex={active ? 0 : -1}
              className={cn("settings-v340-nav-item", active && "is-active")}
              onClick={() => onSectionChange(section.id)}
              onKeyDown={(event) => moveFocus(event, index)}
            >
              {section.label}
            </button>
          );
        })}
      </nav>
      <span data-settings-nav-fade="start" aria-hidden="true" className="settings-v340-nav-fade is-start" />
      <span data-settings-nav-fade="end" aria-hidden="true" className="settings-v340-nav-fade is-end" />
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
  const page = useContext(SettingsPageContext);
  const count = page?.counts[id];
  const countBadge = typeof count === "number" && count > 0
    ? <span data-settings-section-count className="settings-v340-count">{page?.itemCount(count)}</span>
    : null;
  const sectionClass = cn("settings-v340-section", className);
  const sectionData = {
    tabIndex: -1,
    "data-settings-section-content": true,
    "data-settings-search-label": title,
    "data-settings-search-description": description,
    "data-settings-section-current": page ? String(page.activeSectionId === id) : "true"
  } as const;

  if (progressiveDisclosure) {
    return (
      <section id={id} aria-labelledby={`${id}-heading`} {...sectionData} className={sectionClass}>
        <details data-settings-progressive-disclosure className="settings-v340-disclosure">
          <summary>
            <span className="settings-v340-section-head-text">
              <h2 id={`${id}-heading`} data-settings-section-heading tabIndex={-1} className="settings-v340-section-title">
                {title}
                {countBadge}
              </h2>
              {description ? <span className="settings-v340-lead">{description}</span> : null}
            </span>
            <span aria-hidden="true" className="settings-v340-disclosure-caret">›</span>
          </summary>
          <div data-settings-section-body className="settings-v340-section-body">{children}</div>
        </details>
      </section>
    );
  }

  return (
    <section id={id} aria-labelledby={`${id}-heading`} {...sectionData} className={sectionClass}>
      <header className="settings-v340-section-head">
        <div className="settings-v340-section-head-text">
          <h2 id={`${id}-heading`} data-settings-section-heading tabIndex={-1} className="settings-v340-section-title">
            {title}
            {countBadge}
          </h2>
          {description ? <p className="settings-v340-lead">{description}</p> : null}
        </div>
      </header>
      <div data-settings-section-body className="settings-v340-section-body">{children}</div>
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
      data-settings-group
      className={cn("settings-v340-group", className)}
    >
      {title ? <h3 data-settings-group-title className="settings-v340-group-title">{title}</h3> : null}
      {description ? <p className="settings-v340-group-desc">{description}</p> : null}
      <div data-settings-group-rows className="settings-v340-group-rows">{children}</div>
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
      data-control-width={controlWidth}
      data-settings-search-label={label}
      data-settings-search-description={[description, hint].filter(Boolean).join(" ") || undefined}
      className={cn("settings-v340-row", className)}
    >
      <div className="settings-v340-row-label">
        {id ? (
          <label htmlFor={id} className="settings-v340-row-title">{label}</label>
        ) : (
          <strong className="settings-v340-row-title">{label}</strong>
        )}
        {description ? <span className="settings-v340-row-desc">{description}</span> : null}
        {hint ? <span className="settings-v340-row-hint">{hint}</span> : null}
      </div>
      <div className="settings-v340-row-control">{children}</div>
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
        "settings-v340-segmented",
        layout === "three-option-responsive" && "is-responsive",
        disabled && "is-disabled"
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
              "settings-v340-segment",
              layout === "three-option-responsive" && "is-responsive",
              selected && "is-selected",
              selected ? selectedFocusVisibleState : focusVisibleState
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
  const [closing, setClosing] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const [menuAbove, setMenuAbove] = useState(false);
  const [menuStyle, setMenuStyle] = useState<CSSProperties>({});
  const closeTimerRef = useRef<number | null>(null);
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
      setMenuAbove(openUp);
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
      dismissMenu(false);
    }

    function handleOutsideFocus(event: FocusEvent) {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (controlRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      dismissMenu(false);
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

  useEffect(() => {
    if (!open) return;
    playOverlayEnter(menuRef.current, "menu", {
      above: menuAbove,
      origin: menuAbove ? "bottom left" : "top left"
    });
  }, [open, menuAbove]);

  useEffect(() => {
    if (!closing) return;
    void playOverlayExit(menuRef.current, "menu");
  }, [closing]);

  useEffect(() => () => {
    if (closeTimerRef.current !== null) window.clearTimeout(closeTimerRef.current);
  }, []);

  function cancelPendingClose() {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
    setClosing(false);
  }

  function openMenu() {
    cancelPendingClose();
    setActiveIndex(selectedIndex);
    setOpen(true);
  }

  function closeNow() {
    cancelPendingClose();
    setOpen(false);
    triggerRef.current?.focus({ preventScroll: true });
  }

  function dismissMenu(restoreFocus = true) {
    if (!open || closing || closeTimerRef.current !== null) return;
    if (prefersReducedMotion()) {
      closeNow();
      return;
    }
    setClosing(true);
    closeTimerRef.current = window.setTimeout(() => {
      closeTimerRef.current = null;
      setClosing(false);
      setOpen(false);
      if (restoreFocus) triggerRef.current?.focus({ preventScroll: true });
    }, 130);
  }

  function chooseOption(index: number) {
    const option = options[index];
    if (!option || disabled) return;
    onChange(option.value);
    closeNow();
  }

  function handleTriggerKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (disabled || !options.length) return;
    const isOpen = open;
    if (event.key === "Escape" && isOpen) {
      event.preventDefault();
      dismissMenu();
      return;
    }
    if ((event.key === "Enter" || event.key === " " || event.key === "Spacebar") && !isOpen) {
      event.preventDefault();
      openMenu();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!isOpen) {
        openMenu();
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
    if (open && (closing || closeTimerRef.current !== null)) {
      // A second click during the exit reverses it instead of starting a
      // second, conflicting dismissal. The menu is still mounted, so replay
      // the entrance explicitly (the open-state effect will not fire again).
      cancelPendingClose();
      playOverlayEnter(menuRef.current, "menu", {
        above: menuAbove,
        origin: menuAbove ? "bottom left" : "top left"
      });
      return;
    }
    if (open) {
      dismissMenu();
    } else {
      openMenu();
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
          className={cn(settingsSelect, "settings-v340-select", open && "is-open")}
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
            data-closing={closing ? "true" : undefined}
            className="settings-v340-select-menu fixed z-[120] grid gap-1 overflow-y-auto overscroll-contain rounded-[var(--zc-radius-floating)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-1.5 shadow-[var(--zc-shadow-menu)]"
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
    <label data-settings-item data-settings-search-label={label} data-settings-search-description={description} className="settings-v340-field">
      <span className="settings-v340-field-label">{label}</span>
      {description ? <span className="settings-v340-field-desc">{description}</span> : null}
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
      data-settings-disclosure
      className="settings-v340-disclosure"
    >
      <summary className={cn("settings-v340-disclosure-summary", focusVisibleState)}>
        <span className="settings-v340-section-head-text">
          <span className="settings-v340-disclosure-title">{title}</span>
          {description ? <span className="settings-v340-disclosure-desc">{description}</span> : null}
        </span>
        <span aria-hidden="true" className="settings-v340-disclosure-caret">›</span>
      </summary>
      <div className="settings-v340-disclosure-body">{children}</div>
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
    <div data-settings-empty className="settings-v340-empty">
      <div>
        <strong>{title}</strong>
        {description ? <span>{description}</span> : null}
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
