import type { ReactNode } from "react";

/** HUD glass panel: title bar (status dot + label + right slot) + scrollable body. */
export default function HudPanel({
  title,
  ok,
  right,
  children,
  className,
  bodyClassName,
}: {
  title: string;
  ok?: boolean;
  right?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  return (
    <section className={`hud-panel ${className ?? ""}`}>
      <header className="hud-panel-title">
        {ok !== undefined && <span className={`hud-dot ${ok ? "hud-dot-ok" : "hud-dot-bad"}`} />}
        <span className="flex-1 truncate">{title}</span>
        {right}
      </header>
      <div className={`hud-panel-body ${bodyClassName ?? ""}`}>{children}</div>
    </section>
  );
}
