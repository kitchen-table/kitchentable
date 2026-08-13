import { Mark } from "../Mark";
import { DeviceIcon, MoonIcon, SearchIcon } from "../icons";

/**
 * The window's top strip: mark, search, pending-device bell, appearance, new app.
 *
 * The mockup draws macOS traffic lights here. We do not: the window uses the
 * native overlay title bar style, so the real buttons float over this strip and
 * `--traffic-inset` reserves the room they need. Drawing our own would give the
 * user two sets, one of them fake.
 */
export function TitleBar({
  query,
  onQuery,
  pending,
  onPending,
  dark,
  onToggleTheme,
  onNewApp,
}: {
  query: string;
  onQuery: (value: string) => void;
  /** Devices waiting for a decision. Drives the badge. */
  pending: number;
  onPending: () => void;
  dark: boolean;
  onToggleTheme: () => void;
  onNewApp: () => void;
}) {
  return (
    <div
      data-tauri-drag-region
      style={{
        height: 48,
        flex: "none",
        display: "flex",
        alignItems: "center",
        gap: 14,
        padding: "0 16px 0 var(--traffic-inset)",
        background: "var(--raised)",
        borderBottom: "1px solid var(--border)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 9 }}>
        <Mark size={18} />
        <span style={{ font: "700 13px var(--font-sans)" }}>Kitchen Table</span>
      </div>

      <label
        style={{
          flex: 1,
          maxWidth: 300,
          margin: "0 auto",
          height: 28,
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderRadius: 8,
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "0 11px",
        }}
      >
        <SearchIcon colour="var(--faint)" />
        <input
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          placeholder="Search apps &amp; devices"
          aria-label="Search apps and devices"
          style={{
            flex: 1,
            minWidth: 0,
            border: "none",
            outline: "none",
            background: "none",
            font: "400 12px var(--font-sans)",
            color: "var(--ink)",
          }}
        />
      </label>

      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <IconButton
          label={
            pending === 1
              ? "1 device waiting for approval"
              : `${pending} devices waiting for approval`
          }
          onClick={onPending}
          badge={pending}
        >
          <DeviceIcon size={16} colour="var(--ink2)" />
        </IconButton>

        <IconButton
          label={dark ? "Switch to light appearance" : "Switch to dark appearance"}
          onClick={onToggleTheme}
        >
          <MoonIcon size={16} colour="var(--ink2)" />
        </IconButton>

        <button
          type="button"
          onClick={onNewApp}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            border: "none",
            background: "var(--accent)",
            color: "#fff",
            padding: "7px 13px",
            borderRadius: 9,
            font: "600 12.5px var(--font-sans)",
            cursor: "pointer",
          }}
        >
          <span aria-hidden="true">＋</span> New app
        </button>
      </div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  badge,
  children,
}: {
  label: string;
  onClick: () => void;
  badge?: number;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      style={{
        position: "relative",
        width: 32,
        height: 32,
        borderRadius: 9,
        background: "var(--paper)",
        border: "1px solid var(--border)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        cursor: "pointer",
        padding: 0,
      }}
    >
      {children}
      {badge !== undefined && badge > 0 && (
        <span
          style={{
            position: "absolute",
            top: -4,
            right: -4,
            minWidth: 16,
            height: 16,
            padding: "0 3px",
            background: "#ec6a5e",
            color: "#fff",
            borderRadius: 8,
            font: "700 9px var(--font-mono)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            border: "2px solid var(--raised)",
          }}
        >
          {badge > 9 ? "9+" : badge}
        </span>
      )}
    </button>
  );
}
