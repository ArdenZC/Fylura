# Fylura v3.40 prototype restoration QA

Reference: `docs/design/prototypes/Fylura-proto-v3.40.zip` (`redesign-v2.html`, `MOTION.md`). The prototype is treated as the visual and interaction reference; its embedded sample files, counts, and user name are not copied into product state.

## Visual review

Paired captures use a 1440×900 CSS viewport, DPR 1, Chinese locale, light theme, and the Windows titlebar variant. The Files capture shows an open folder, one selected file, and the Inspector. Cleanup shows a completed browser-mock analysis for Documents. Overview, Organize, History, and Settings show their default browser-mock states.

- [Overview comparison](docs/design/v340-evidence/brand-motion-qa/overview-comparison.png)
- [Files comparison](docs/design/v340-evidence/brand-motion-qa/files-comparison.png)
- [Organize comparison](docs/design/v340-evidence/brand-motion-qa/organize-comparison.png)
- [Cleanup comparison](docs/design/v340-evidence/brand-motion-qa/cleanup-comparison.png)
- [History comparison](docs/design/v340-evidence/brand-motion-qa/history-comparison.png)
- [Settings comparison](docs/design/v340-evidence/brand-motion-qa/settings-comparison.png)
- [Logo comparison](docs/design/v340-evidence/brand-motion-qa/brand-comparison.png)
- Capture metadata and motion measurements: [capture.json](docs/design/v340-evidence/brand-motion-qa/capture.json)

The sidebar now uses the repository's `build/icon.png` asset at the prototype's 30px expanded size and 28px rail size. The titlebar no longer repeats the mark; its sidebar, theme, language, and Windows controls follow the reference. Overview, Files, Organize, Cleanup, History, and Settings were visually compared. Cleanup now follows the prototype's order: page heading, four metrics, candidate heading/filter controls, selectable list, and selection footer. Finding details and secondary actions stay available from each row's details control.

## Motion review

Browser interaction measurements on the implementation:

- Sidebar: 216px expanded → 64px rail in a 200ms `cubic-bezier(0.32, 0.72, 0, 1)` transition; hide reaches 0px and restore returns to the prior rail width.
- Page entry: 220ms fade and 4px rise with the same smooth curve.
- Reduced motion: page-entry animation count is zero; shell transition resolves to the browser's 1µs reduced-motion duration.
- Prototype and implementation rail/hidden states are included in the paired evidence. See `MOTION.md` for the reference's press, overlay, toast, tree, and file-row rules; the implementation uses the shared motion tokens and helpers.

## State and acceptance boundary

The implementation continues to render current application state and existing operation/preview/restore flows. Prototype-only example names and counts remain reference data, not product records. Browser-mock screenshots verify layout and interactions; they are not native Windows/macOS acceptance evidence. Frontend typecheck and production build are recorded in the task handoff; the frontend test suite was not run.

final result: passed
