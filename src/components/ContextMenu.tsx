import { Fragment, useEffect, useRef, type CSSProperties } from "react";
import { Check } from "lucide-react";
import { useUiStore, type MenuItemSpec } from "../stores/ui";

/** Renders a list of menu items (shared by context menus and title-bar menus). */
export function MenuItems({ items, onClose }: { items: MenuItemSpec[]; onClose: () => void }) {
  return (
    <>
      {items.map((it, i) =>
        it.separator ? (
          <div key={`sep-${i}`} className="menu-sep" />
        ) : (
          <Fragment key={`${it.label}-${i}`}>
            <button
              role="menuitem"
              disabled={it.disabled}
              title={it.title}
              className={`menu-item ${it.danger ? "hover:!text-critical focus:!text-critical" : ""}`}
              onClick={() => {
                onClose();
                it.onSelect?.();
              }}
            >
              <span className="flex w-4 shrink-0 items-center justify-center opacity-80">{it.checked ? <Check size={14} /> : it.icon}</span>
              <span className="flex-1">{it.label}</span>
              {it.shortcut && <span className="ml-6 text-[11px] text-content-muted">{it.shortcut}</span>}
            </button>
          </Fragment>
        ),
      )}
    </>
  );
}

/** Fixed-position menu at a point; closes on outside click, Esc, scroll or resize. */
export default function ContextMenu() {
  const menu = useUiStore((s) => s.contextMenu);
  const close = useUiStore((s) => s.closeContextMenu);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("resize", close);
    };
  }, [menu, close]);

  if (!menu) return null;
  const height = menu.items.length * 32 + 12;
  const width = 220;
  const style: CSSProperties = {
    left: Math.min(menu.x, window.innerWidth - width - 8),
    top: menu.y + height > window.innerHeight ? Math.max(8, menu.y - height) : menu.y,
  };
  return (
    <div ref={ref} role="menu" className="menu" style={style}>
      <MenuItems items={menu.items} onClose={close} />
    </div>
  );
}
