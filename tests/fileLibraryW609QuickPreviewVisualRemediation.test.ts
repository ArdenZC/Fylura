import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(path), "utf8").replace(/\r\n?/gu, "\n");

describe("W6-09 Quick Preview visual remediation contracts", () => {
  it("keeps the canonical surface presentation quiet and truthful", () => {
    const appShell = read("src/components/AppShell.tsx");
    const surface = read("src/views/fileLibrary/preview/ZenQuickPreviewSurface.tsx");
    const content = read("src/views/fileLibrary/preview/PreviewContent.tsx");
    const floating = read("src/views/fileLibrary/preview/ZenFloatingQuickPreview.tsx");
    const styles = read("src/views/fileLibrary/preview/zenFloatingQuickPreview.css");
    const tokens = read("src/styles/tokens.css");

    expect(appShell).toContain("Settings,");
    expect(appShell).not.toContain("Settings2");
    expect(appShell).not.toContain("Tune");
    expect(surface).toContain('data-preview-details-facts="true"');
    expect(surface).toContain("previewMaterializationLabel");
    expect(surface).toContain('data-preview-reveal="true"');
    expect(surface).toContain("setDetailsOpen");
    expect(surface).not.toContain("QuickPreviewFooter");
    expect(floating).toContain("initialFocusRef={surfaceRef}");
    expect(surface).toContain("tabIndex={-1}");
    expect(surface).not.toContain('t("libraryPreviewClose")}</button>');
    expect(content).not.toContain('className="zc-quick-preview-facts"');
    expect(content).not.toContain('t("previewMarkdownContent")');
    expect(content).not.toContain('<span>{t("libraryPreviewImage")}</span>');
    expect(content).toContain("TriangleAlert");
    expect(styles).toContain("background: transparent;");
    expect(styles).not.toContain(".zc-quick-preview-footer-status");
    expect(styles).not.toContain(".zc-quick-preview-footer");
    expect(styles).not.toContain("content-visibility: auto");
    expect(styles).toContain('[data-density="compact"] .zc-quick-preview-card');
    expect(styles).toContain("background: var(--zc-surface-overlay);");
    expect(styles).toContain("box-shadow: var(--zc-shadow-float);");
    expect(styles).toContain("grid-template-columns: 84px minmax(0, 1fr) 132px;");
    expect(styles).toContain("grid-template-columns: minmax(0, 1fr) 240px;");
    expect(surface).toContain("data-preview-content-mode={state.phase === \"content\"");
    expect(styles).toContain("height: 100%;");
    expect(styles).toContain("background: var(--zc-preview-canvas);");
    expect(styles).toContain('.zc-quick-preview-card > [data-preview-state-announcement="true"]');
    expect(styles).toContain("position: absolute;");
    expect(tokens).toContain("--zc-preview-canvas: #eaf1f9;");
    expect(tokens).toContain("--zc-preview-canvas: #151a20;");
    const documentModeRule = styles.match(/\.zc-quick-preview-content\[data-preview-content-mode="safe_html"\]\s*\{([^}]*)\}/u)?.[1] ?? "";
    expect(documentModeRule).toContain("display: block;");
    expect(documentModeRule).toContain("padding: 34px 40px;");
    expect(documentModeRule).not.toContain("place-items: center;");
    expect(styles).toContain("width: min(720px, 100%);");
    expect(styles).toContain("padding: 44px 50px;");
    expect(styles).toContain("width: min(720px, 92%);");
    expect(styles).toContain("max-width: min(720px, 92%);");
    expect(styles).toContain("max-height: 100%;");
    expect(styles).toContain(".zc-quick-preview-content[data-preview-content-mode=\"state\"]");
    expect(styles).toContain(".zc-quick-preview-content > .zc-quick-preview-status,");
    expect(styles).toContain(".zc-preview-image > .zc-quick-preview-status");
    expect(styles).toMatch(/\.zc-preview-image-stage\s*\{[\s\S]*?background: transparent;/u);
    expect(content).toContain('data-preview-progress="true"');
    expect(content).toContain('data-preview-image-failed="true"');
    // v3.40 reserves glass for the shell tier (titlebar / sidebar / main /
    // floating overlays). The preview surface itself never stacks glass or a
    // second blur on top of a card.
    expect(styles).not.toContain("--zc-glass-");
    expect(styles).not.toContain("backdrop-filter");
    for (const shellGlass of ["--zc-glass-titlebar:", "--zc-glass-sidebar:", "--zc-glass-main:", "--zc-glass-float:"]) {
      expect(tokens).toContain(shellGlass);
    }
    expect(tokens).not.toContain("--zc-glass-card");
    expect(tokens).not.toContain("--zc-glass-row");
    expect(styles).toContain("place-items: center;");
    expect(styles).toContain("forced-colors");
  });

  it("marks PDF pages only after a settled near-viewport render", () => {
    const renderer = read("src/views/fileLibrary/preview/renderers/PdfPreviewRenderer.tsx");

    expect(renderer).toContain("useMemo(");
    expect(renderer).toContain('setRenderState("rendering")');
    expect(renderer).toContain('setRenderState("rendered")');
    expect(renderer).toContain('data-preview-pdf-page-state={pageState}');
    expect(renderer).toContain(": !nearViewport");
    expect(renderer).toContain("pageRequestRef");
    expect(renderer).toContain("Do not cancel this request when the page briefly leaves the nearby");
    expect(renderer).toContain("const shouldRenderCanvas = nearViewport");
    expect(renderer).not.toContain("}, [nearViewport, page, viewport]);");
    expect(renderer).not.toContain("!visible || !nearViewport");
  });

  it("keeps PDF loading to one concise progress label", () => {
    const renderer = read("src/views/fileLibrary/preview/renderers/PdfPreviewRenderer.tsx");

    expect(renderer).toContain('if (status === "loading")');
    expect(renderer).toContain("<LoaderCircle className=\"animate-spin\"");
    expect(renderer).toContain('data-preview-pdf-message={status} role="status"');
  });
});
