import { useRef } from "react";

/** A drag handle between two panes. `onDelta` gets pixel movement; `onEnd` fires on release. */
export default function Splitter({ direction, onDelta, onEnd }: { direction: "vertical" | "horizontal"; onDelta: (px: number) => void; onEnd?: () => void }) {
  const last = useRef(0);
  const start = (e: React.PointerEvent) => {
    e.preventDefault();
    last.current = direction === "vertical" ? e.clientX : e.clientY;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) => {
      const pos = direction === "vertical" ? ev.clientX : ev.clientY;
      onDelta(pos - last.current);
      last.current = pos;
    };
    const up = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", up);
      onEnd?.();
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", up);
  };
  return (
    <div
      onPointerDown={start}
      className={`shrink-0 bg-transparent transition-colors hover:bg-accent/40 ${direction === "vertical" ? "w-1 cursor-col-resize" : "h-1 cursor-row-resize"}`}
    />
  );
}
