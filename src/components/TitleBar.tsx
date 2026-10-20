import MenuBar from "./MenuBar";
import WindowControls, { isWindows } from "./WindowControls";

/** Custom title bar for the frameless Windows build: icon, menus, title, window controls. On
 *  other platforms the native decorations stay and only the menu strip renders. */
export default function TitleBar() {
  return (
    <div data-tauri-drag-region className="flex h-8 shrink-0 items-stretch border-b border-border bg-surface">
      <div className="flex items-center gap-1.5 pl-2.5">
        <img src="/icon.png" alt="" className="pointer-events-none h-4 w-4" />
      </div>
      <MenuBar />
      <div data-tauri-drag-region className="flex flex-1 items-center justify-center">
        <span className="pointer-events-none text-xs font-semibold tracking-wide text-content-secondary">dbclient</span>
      </div>
      {isWindows && <WindowControls />}
    </div>
  );
}
