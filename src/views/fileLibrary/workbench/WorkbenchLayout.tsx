import { useRef, type ReactNode } from "react";
import type { FileWorkbenchModel, WorkbenchTab } from "./workbenchModel";
/** Layout preferences only; pane controllers and transfer ownership stay outside. */
export function WorkbenchLayout({ tab, model, wide, children, label, disabled }: {
    tab: WorkbenchTab;
    model: FileWorkbenchModel;
    wide: boolean;
    children: ReactNode[];
    label: string;
    disabled: boolean;
}) {
    const root = useRef<HTMLDivElement>(null);
    const columns = wide ? (tab.layout === "grid" && tab.paneCount === 4 ? 2 : tab.paneCount) : 1;
    const rows = wide && tab.layout === "grid" && tab.paneCount === 4 ? 2 : 1;
    function separator(axis: "columnSizes" | "rowSizes", index: number, count: number) {
        const sizes = tab[axis];
        const total = sizes.slice(0, count).reduce((a, b) => a + b, 0);
        const before = sizes.slice(0, index).reduce((a, b) => a + b, 0);
        const position = (before + sizes[index]) / total * 100;
        const vertical = axis === "columnSizes";
        const change = (delta: number) => {
            const current = model.getSnapshot().tabs.find((item) => item.id === tab.id);
            if (!current)
                return;
            const next = [...current[axis]];
            const pair = next[index] + next[index + 1];
            next[index] = Math.max(pair * 0.2, Math.min(pair * 0.8, next[index] + delta));
            next[index + 1] = pair - next[index];
            model.setSizes(tab.id, axis, next);
        };
        return <div key={`${axis}-${index}`} role="separator" tabIndex={disabled ? -1 : 0} aria-label={`${label} ${index + 1}`} aria-orientation={vertical ? "vertical" : "horizontal"} aria-valuemin={20} aria-valuemax={80} aria-valuenow={Math.round(sizes[index] / (sizes[index] + sizes[index + 1]) * 100)} className={`wb-divider wb-divider-${vertical ? "column" : "row"}`} style={vertical ? { left: `${position}%` } : { top: `${position}%` }} onKeyDown={(event) => {
                if (disabled)
                    return;
                const direction = vertical ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
                if (!direction.includes(event.key))
                    return;
                event.preventDefault();
                change((event.key === direction[0] ? -1 : 1) * total * 0.025);
            }} onPointerDown={(event) => {
                if (disabled)
                    return;
                event.preventDefault();
                event.currentTarget.setPointerCapture(event.pointerId);
            }} onPointerMove={(event) => {
                if (disabled || !event.currentTarget.hasPointerCapture(event.pointerId))
                    return;
                const rect = root.current?.getBoundingClientRect();
                if (!rect)
                    return;
                const fraction = vertical ? (event.clientX - rect.left) / rect.width : (event.clientY - rect.top) / rect.height;
                const desired = fraction * total - before;
                change(desired - sizes[index]);
            }} onPointerUp={(event) => {
                if (event.currentTarget.hasPointerCapture(event.pointerId))
                    event.currentTarget.releasePointerCapture(event.pointerId);
            }}/>;
    }
    return <div ref={root} className="wb-layout" data-columns={columns} data-rows={rows} style={{ gridTemplateColumns: tab.columnSizes.slice(0, columns).map((n) => `minmax(0, ${n}fr)`).join(" "),
            gridTemplateRows: tab.rowSizes.slice(0, rows).map((n) => `minmax(0, ${n}fr)`).join(" ") }}>
    {children.map((child, index) => <div className="wb-pane-cell" key={tab.panes[index].id} hidden={index >= tab.paneCount || (!wide && tab.panes[index].id !== tab.activePaneId)}>{child}</div>)}
    {Array.from({ length: columns - 1 }, (_, i) => separator("columnSizes", i, columns))}
    {rows > 1 && separator("rowSizes", 0, rows)}
  </div>;
}
