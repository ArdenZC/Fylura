import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

describe("v3.40 motion system", () => {
  it("keeps the approved duration and spring ladder wired as tokens", () => {
    const tokens = read("src/styles/tokens.css");
    expect(tokens).toContain("--zc-dur-press: 90ms");
    expect(tokens).toContain("--zc-dur-release: 260ms");
    expect(tokens).toContain("--zc-dur-pop: 240ms");
    expect(tokens).toContain("--zc-dur-pop-out: 130ms");
    expect(tokens).toContain("--zc-dur-panel: 200ms");
    expect(tokens).toContain("--zc-dur-page: 220ms");
    expect(tokens).toContain("--zc-spring-smooth: cubic-bezier(0.32, 0.72, 0, 1)");
    expect(tokens).toContain("--zc-spring-pop: cubic-bezier(0.34, 1.38, 0.64, 1)");
    expect(tokens).toContain("--zc-spring-bounce: cubic-bezier(0.2, 1.62, 0.36, 1)");
    expect(tokens).toContain("--zc-spring-out: cubic-bezier(0.4, 0, 1, 1)");
  });

  it("uses press, release and colour timings separately", () => {
    const motion = read("src/styles/v340-motion.css");
    expect(motion).toContain('html [data-zc-press="active"]:not(:disabled)');
    expect(motion).toContain("var(--zc-dur-press) var(--zc-spring-out)");
    expect(motion).toContain('html [data-zc-press="release"]');
    expect(motion).toContain("var(--zc-dur-release) var(--zc-spring-pop)");
    expect(motion).toContain("background-color var(--zc-duration-fast)");
  });

  it("animates real height for tree branches without display toggles", () => {
    const motion = read("src/styles/v340-motion.css");
    const tree = read("src/views/fileLibrary/navigation/DirectoryTree.tsx");
    const navigation = read("src/views/fileLibrary/navigation/FileLibraryNavigation.tsx");
    expect(motion).toContain("grid-template-rows: 0fr");
    expect(motion).toContain("grid-template-rows: 1fr");
    expect(tree).toContain('className="file-library-tree-branch"');
    expect(tree).toContain('data-open={node.expanded ? "true" : "false"}');
    expect(tree).toContain("inert={!node.expanded ? true : undefined}");
    expect(navigation).toContain('className="file-library-navigation-group-body"');
  });

  it("keeps sidebar content mounted so width and labels interpolate together", () => {
    const shell = read("src/styles/v340-shell.css");
    const appShell = read("src/components/AppShell.tsx");
    expect(shell).toContain("max-width: 220px");
    expect(shell).toContain("max-width: 0");
    expect(shell).toContain("contain: layout");
    expect(shell).toContain("grid-template-columns: 0 minmax(0, 1fr)");
    expect(shell).not.toContain('.zc-shell-body[data-sidebar-mode="rail"] .zc-nav-text,\n.zc-shell-body[data-sidebar-mode="rail"] .zc-nav-count');
    expect(appShell).toContain("<PressMotion />");
    expect(appShell).toContain("playPageEnter(viewStageRef.current)");
    expect(appShell).toContain('inert={mode === "hidden" ? true : undefined}');
  });

  it("keeps file selection instant and only animates the check mark", () => {
    const motion = read("src/styles/v340-motion.css");
    expect(motion).toContain(".shared-file-list-row.is-selected");
    expect(motion).toContain("transition: none !important");
    expect(motion).toContain("zc-file-check-in");
    expect(motion).toContain("var(--zc-dur-release) var(--zc-spring-bounce)");
  });

  it("skips WAAPI under reduced motion and cancels duplicate runs", () => {
    const helper = read("src/utils/motion.ts");
    const press = read("src/components/motion/PressMotion.tsx");
    expect(helper).toContain('window.matchMedia("(prefers-reduced-motion: reduce)")');
    expect(helper).toContain("if (!element || prefersReducedMotion()) return null");
    expect(helper).toContain("if (animation.id === id) animation.cancel()");
    expect(press).toContain('document.addEventListener("pointercancel", release, true)');
    expect(press).toContain('document.addEventListener("pointerout", onPointerOut, true)');
    expect(press).toContain('document.addEventListener("keydown", onKeyDown, true)');
    expect(press).toContain("window.setTimeout");
  });
});
