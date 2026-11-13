import { describe, expect, it } from "vitest";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const root = process.cwd();

function read(relativePath: string) {
  const path = join(root, relativePath);
  return existsSync(path) ? readFileSync(path, "utf8") : "";
}

function productionSource() {
  const files: string[] = [];
  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(path);
      } else if (/\.(css|ts|tsx)$/.test(entry.name)) {
        files.push(path);
      }
    }
  };

  visit(join(root, "src"));
  return files.map((path) => readFileSync(path, "utf8")).join("\n");
}

function tokenValue(source: string, name: string) {
  return source.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1].trim() ?? "";
}

function brandMarkVariant(source: string, variant: string, nextVariant?: string) {
  const normalizedSource = source.replace(/\r\n?/g, "\n");
  const boundary = nextVariant
    ? `\\n\\s*\\},\\n\\s*${nextVariant}:`
    : "\\n\\s*\\}";
  return normalizedSource.match(
    new RegExp(`${variant}:\\s*\\{([\\s\\S]*?)${boundary}`)
  )?.[1] ?? "";
}

function relativeLuminance(hex: string) {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!match) throw new Error(`Expected a six-digit hex color, received: ${hex}`);

  const channels = match.slice(1).map((channel) => Number.parseInt(channel, 16) / 255);
  const [red, green, blue] = channels.map((channel) =>
    channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
  );
  return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
}

function contrastRatio(foreground: string, background: string) {
  const foregroundLuminance = relativeLuminance(foreground);
  const backgroundLuminance = relativeLuminance(background);
  const lighter = Math.max(foregroundLuminance, backgroundLuminance);
  const darker = Math.min(foregroundLuminance, backgroundLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

describe("v3.40 design foundation (approved prototype baseline)", () => {
  const styles = read("src/styles.css");
  const tokens = read("src/styles/tokens.css");
  const shellV340 = read("src/styles/v340-shell.css");
  const filesV340 = read("src/views/fileLibrary/fileLibraryV340.css");
  const filesV26 = read("src/views/fileLibrary/fileLibraryV26.css");
  const tw = read("src/utils/tw.ts");
  const brandMark = read("src/components/ui/BrandMark.tsx");
  const shellChrome = read("src/components/ShellChrome.tsx");

  it("binds production semantic roles to the approved v3.40 light target", () => {
    expect(tokens).toContain("--zc-canvas: #eff3f9");
    expect(tokens).toContain("--zc-surface: #ffffff");
    expect(tokens).toContain("--zc-surface-base: #ffffff");
    expect(tokens).toContain("--zc-surface-raised: #fafcff");
    expect(tokens).toContain("--zc-surface-overlay: #ffffff");
    expect(tokens).toContain("--zc-surface-subtle: #edf3fb");
    expect(tokens).toContain("--zc-surface-hover: #e8f0fa");
    expect(tokens).toContain("--zc-surface-selected: #e2edfb");
    expect(tokens).toContain("--zc-primary: #1071dc");
    expect(tokens).toContain("--zc-primary-hover: #0a63c6");
    expect(tokens).toContain("--zc-primary-pressed: #0854a8");
    expect(tokens).toContain("--zc-primary-soft: #e6f0fd");
    expect(tokens).toContain("--zc-border-subtle: #e2e8f1");
    expect(tokens).toContain("--zc-border: #c3cedd");
    expect(tokens).toContain("--zc-border-strong: #a7b6c9");
    expect(tokens).toContain("--zc-focus-soft: #e6f0fd");
    expect(tokens).toContain("--zc-selected-focus: #d4e4f8");
    expect(tokens).toContain("--zc-selection-mark: #1071dc");
    expect(tokens).toContain("--zc-shadow-float:");
    expect(tokens).toContain("--zc-shadow-menu:");
    expect(tokens).toContain("--zc-focus-halo:");
    expect(tokens).toContain("--zc-glass-titlebar:");
    expect(tokens).toContain("--zc-glass-sidebar:");
    expect(tokens).toContain("--zc-glass-main:");
    expect(tokens).not.toContain("--zc-surface-floating:");
    expect(tokens).not.toContain("--zc-shadow-floating:");
    expect(styles).toContain('@import "./styles/tokens.css"');
    expect(styles).toContain('@import "./styles/v340-shell.css"');
    expect(styles).toContain('@import "./views/fileLibrary/fileLibraryV26.css"');
  });

  it("meets the retained light-mode contrast gates used by the Solid / Calm shell", () => {
    const primary = tokenValue(tokens, "zc-primary");
    const primaryHover = tokenValue(tokens, "zc-primary-hover");
    const primaryPressed = tokenValue(tokens, "zc-primary-pressed");
    const primaryContrast = tokenValue(tokens, "zc-primary-contrast");
    const secondary = tokenValue(tokens, "zc-text-secondary");
    const surface = tokenValue(tokens, "zc-surface");
    const canvas = tokenValue(tokens, "zc-canvas");
    const focus = tokenValue(tokens, "zc-focus");
    const controlBorder = tokenValue(tokens, "zc-control-border");

    for (const value of [primary, primaryHover, primaryPressed, primaryContrast, secondary, surface, canvas, focus, controlBorder]) {
      expect(value).toMatch(/^#[0-9a-f]{6}$/i);
    }

    expect(contrastRatio(primary, primaryContrast)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(primaryHover, primaryContrast)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(primaryPressed, primaryContrast)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(secondary, surface)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(secondary, canvas)).toBeGreaterThanOrEqual(4.5);
    expect(contrastRatio(focus, canvas)).toBeGreaterThanOrEqual(3);
    expect(contrastRatio(controlBorder, surface)).toBeGreaterThanOrEqual(3);
  });

  it("binds dark mode to the approved v3.40 semantic roles", () => {
    const darkTheme = tokens.match(/:root\.dark\s*\{([\s\S]*?)\}/)?.[1] ?? "";

    expect(darkTheme).toContain("--zc-canvas: #14181e");
    expect(darkTheme).toContain("--zc-surface: #1d232b");
    expect(darkTheme).toContain("--zc-surface-raised: #222932");
    expect(darkTheme).toContain("--zc-surface-overlay: #222932");
    expect(darkTheme).toContain("--zc-surface-subtle: #262e38");
    expect(darkTheme).toContain("--zc-surface-selected: #2f3f52");
    expect(darkTheme).toContain("--zc-primary: #66b0ff");
    expect(darkTheme).toContain("--zc-control-border: #66788c");
    expect(darkTheme).toContain("--zc-focus: #66b0ff");
    expect(darkTheme).toContain("--zc-focus-soft: #1e3247");
    expect(darkTheme).toContain("--zc-selected-focus: #37495f");
    expect(tokenValue(tokens, "zc-warning")).not.toBe(tokenValue(tokens, "zc-danger"));
    expect(tokenValue(darkTheme, "zc-warning")).not.toBe(tokenValue(darkTheme, "zc-danger"));
  });

  it("keeps semantic info, neutral, purple, and safety families available in both themes", () => {
    const darkTheme = tokens.match(/:root\.dark\s*\{([\s\S]*?)\}/)?.[1] ?? "";

    for (const family of ["info", "neutral", "purple", "success", "warning", "danger"]) {
      for (const suffix of ["", "-text", "-soft", "-border"]) {
        const tokenName = `zc-${family}${suffix}`;
        expect(tokenValue(tokens, tokenName), `${tokenName} light`).not.toBe("");
        expect(tokenValue(darkTheme, tokenName), `${tokenName} dark`).not.toBe("");
      }
    }
  });

  it("uses the frozen spacing, radius, density, motion, and focus-role ladders", () => {
    expect(tokens).toContain("--zc-space-micro: 2px");
    expect(tokens).toContain("--zc-space-1: 4px");
    expect(tokens).toContain("--zc-space-2: 8px");
    expect(tokens).toContain("--zc-space-3: 12px");
    expect(tokens).toContain("--zc-space-4: 16px");
    expect(tokens).toContain("--zc-space-6: 24px");
    expect(tokens).toContain("--zc-space-8: 32px");
    expect(tokens).toContain("--zc-radius-micro: 4px");
    expect(tokens).toContain("--zc-radius-control: 8px");
    expect(tokens).toContain("--zc-radius-row: 10px");
    expect(tokens).toContain("--zc-radius-panel: 14px");
    expect(tokens).toContain("--zc-control-height-compact: 28px");
    expect(tokens).toContain("--zc-control-height-default: 32px");
    expect(tokens).toContain("--zc-control-height-large: 36px");
    expect(tokens).toContain("--zc-row-height-default: 46px");
    expect(tokens).toContain("--zc-row-height-compact: 36px");
    expect(tokens).toContain("--zc-duration-fast: 120ms");
    expect(tokens).toContain("--zc-duration-standard: 180ms");
    expect(tokens).toContain("--zc-ease-standard: cubic-bezier(0.2, 0, 0, 1)");
    expect(tokens).toContain("--zc-dur-press: 90ms");
    expect(tokens).toContain("--zc-dur-panel: 200ms");
    expect(tokens).toContain("--zc-spring-smooth: cubic-bezier(0.32, 0.72, 0, 1)");
    expect(tokens).toContain("--zc-primary-focus");
  });

  it("uses platform-local typography without reintroducing a remote-first Inter dependency", () => {
    const familyLine = styles.match(/font-family:[^;]+;/)?.[0] ?? "";
    expect(familyLine).toContain('"Segoe UI"');
    expect(familyLine).toContain("system-ui");
    expect(familyLine).toContain('"PingFang SC"');
    expect(familyLine).toContain('"Microsoft YaHei UI"');
    expect(familyLine).not.toContain("Inter");
  });

  it("migrates shell search and navigation to quiet structural chrome without a selection rail", () => {
    // One global search entry in the titlebar; shortcut hints are plain text.
    expect(shellV340).toContain(".zc-titlebar-search");
    expect(shellV340).toContain(".zc-titlebar-search > kbd");
    expect(shellV340).toContain("border: 0;");
    expect(shellV340).toContain("background: transparent;");
    // Navigation marks the current route with a soft fill, never a side rail.
    expect(shellV340).toContain('.zc-nav-item[aria-current="page"]');
    expect(shellV340).toContain("background: var(--zc-primary-soft);");
    expect(shellV340).toContain("outline: 2px solid var(--zc-focus);");
    expect(shellV340).not.toContain("rounded-full");
    expect(shellV340.includes("border-left: 2px")).toBe(false);
  });

  it("migrates the Files command and object surfaces to the retained state grammar", () => {
    expect(filesV340).toContain('.file-library-workspace[data-layout="large"]');
    expect(filesV340).toContain('.file-library-workspace[data-layout="large"][data-tree-inline="true"] .file-library-workspace-body');
    expect(filesV340).toContain("var(--zc-tree-width)");
    expect(filesV340).toContain("var(--zc-inspector-width)");
    expect(filesV340).toContain(".file-library-command-button[aria-pressed=\"true\"]");
    expect(filesV340).toContain("background: var(--zc-primary-soft)");
    const filesV26 = read("src/views/fileLibrary/fileLibraryV26.css");
    expect(filesV26).toContain(".file-library-workspace .file-library-command-group");
    expect(filesV26).toContain(".file-library-workspace .file-library-view-switch");
    expect(filesV26).toContain("border: 0");
    expect(filesV26).toContain("background: transparent");
    expect(filesV26).toContain(".file-library-mode-switch");
    expect(filesV26).toContain("background: var(--zc-surface-subtle)");
    expect(filesV26).toContain("background: var(--zc-focus-soft)");
    expect(filesV26).toContain("background: var(--zc-selected-focus)");
    expect(filesV26).toContain("outline: 0");
    expect(filesV26).not.toContain("box-shadow: inset");
    expect(filesV26).toContain("width: 20px");
    expect(filesV26).toContain('content: "✓"');
    expect(filesV26).toContain("var(--zc-selection-mark)");
    expect(filesV26).toContain('.file-library-workspace[data-layout="medium"] .file-library-command-bar');
    expect(filesV26).toContain('.file-library-workspace[data-layout="compact"] .file-library-command-bar');
    expect(filesV26).toContain(".file-library-workspace .file-library-context-close:focus-visible");
    expect(filesV26).toContain("outline: 2px solid CanvasText");
  });

  it("keeps obsolete floating material owners out of all production source", () => {
    const production = productionSource();

    expect(production).not.toContain("--zc-surface-floating");
    expect(production).not.toContain("--zc-shadow-floating");
  });

  it("keeps material exports semantic and free of revoked Liquid Glass owners", () => {
    for (const exportName of [
      "canvasSurface",
      "contentSurface",
      "raisedSurface",
      "overlaySurface",
      "sidebarSurface",
      "titlebarSurface"
    ]) {
      expect(tw).toContain(`export const ${exportName}`);
    }

    const materials = tw.slice(tw.indexOf("export const canvasSurface"), tw.indexOf("export const appPanel"));
    expect(materials).toContain("var(--zc-");
    expect(materials).not.toContain("slate-");
    expect(materials).not.toContain("blue-");

    // Glass material is a CSS shell concern, not a Tailwind utility export.
    expect(tw).not.toContain("glass");
    expect(tw).not.toContain("backdrop-blur");
    for (const exportName of ["appPanel", "contentPanel", "elevatedPanel", "softPanel", "toolbarSurface", "scopeBarSurface"]) {
      expect(tw).toContain(`export const ${exportName}`);
    }
  });

  it("preserves the local Zen mark semantics while presentation roles migrate", () => {
    expect(brandMark).toContain('type BrandMarkSize = "micro" | "sidebar" | "app"');
    expect(brandMark).toContain("Zen Core");
    expect(brandMark).toContain("Canvas");
    expect(brandMark).toContain("var(--zc-brand-blue)");
    expect(brandMark).toContain("var(--zc-brand-blue-soft)");
    expect(brandMark).not.toMatch(/var\(--zc-primary(?:-[^)]+)?\)/);
    expect(shellChrome).toContain("export function ZenMark");
    expect(shellChrome).toContain("<BrandMark");

    const micro = brandMarkVariant(brandMark, "micro", "sidebar");
    const sidebar = brandMarkVariant(brandMark, "sidebar", "app");
    const app = brandMarkVariant(brandMark, "app");
    expect(micro).not.toContain("shadow-");
    expect(sidebar).not.toMatch(/backdrop-blur-(?:sm|md|lg|xl|2xl|3xl)/);
    expect(app).not.toContain("backdrop-blur-md");
  });

  it("preserves reduced motion and avoids scale-based foundation interactions", () => {
    expect(styles).toContain("@media (prefers-reduced-motion: reduce)");
    expect(brandMark).not.toMatch(/(?:hover|active):scale-/);
    expect(tw).not.toMatch(/(?:hover|active):scale-/);
    expect(shellV340).not.toMatch(/transform:\s*scale/);
    expect(read("src/views/fileLibrary/fileLibraryV26.css")).not.toMatch(/transform: scale/);
  });

  it("keeps status exports free of fixed Tailwind palette colors", () => {
    const statusExports = tw.slice(tw.indexOf("export const statusToast"));
    expect(statusExports).toContain("var(--zc-");
    expect(statusExports).not.toMatch(/(?:red|blue|green|emerald|amber|slate|purple)-\d/);
  });

  it("keeps Ambient Mesh structure local to ShellChrome and semantic tokens", () => {
    expect(shellChrome).toContain("function AmbientMesh");
    expect(shellChrome).toContain("var(--zc-ambient-primary)");
    expect(shellChrome).toContain("var(--zc-ambient-secondary)");
    expect(tokens).toContain("--zc-ambient-primary");
    expect(tokens).toContain("--zc-ambient-secondary");
    expect(styles).not.toContain("--zc-ambient-primary:");
  });
});
