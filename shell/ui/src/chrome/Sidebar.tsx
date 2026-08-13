import { useState } from "react";
import type { Visibility } from "../generated";
import {
  ActivityIcon,
  ChevronUpIcon,
  InvitedIcon,
  LibraryIcon,
  LockIcon,
  NetworkIcon,
  PublicIcon,
  SettingsIcon,
  TeamIcon,
} from "../icons";
import type { Surface } from "../navigation";
import { navSection } from "../navigation";
import { VISIBILITY, VISIBILITY_ORDER } from "../visibility";

/** What the MCP card reports. Absent until the server is running. */
export interface McpStatus {
  running: boolean;
  tools: number;
  port?: number;
}

const FILTER_ICON: Record<Visibility, (colour: string) => React.ReactNode> = {
  private: (colour) => <LockIcon size={16} colour={colour} />,
  network: (colour) => <NetworkIcon size={16} colour={colour} />,
  invited: (colour) => <InvitedIcon size={16} colour={colour} />,
  public: (colour) => <PublicIcon size={16} colour={colour} />,
};

/**
 * The permanent left rail: where you are, what you can filter to, and the two
 * status cards the mockup keeps pinned to the bottom.
 */
export function Sidebar({
  surface,
  onNavigate,
  total,
  counts,
  mcp,
  onQuit,
}: {
  surface: Surface;
  onNavigate: (surface: Surface) => void;
  total: number;
  counts: Partial<Record<Visibility, number>>;
  mcp?: McpStatus;
  onQuit: () => void;
}) {
  const section = navSection(surface);
  const filter = surface.kind === "library" ? surface.filter : null;

  return (
    <nav
      style={{
        width: 228,
        flex: "none",
        background: "var(--raised)",
        borderRight: "1px solid var(--border)",
        display: "flex",
        flexDirection: "column",
        padding: "16px 12px",
      }}
    >
      <Heading>Workspace</Heading>
      <NavRow
        icon={<LibraryIcon />}
        label="Library"
        count={total}
        active={section === "library"}
        onClick={() => onNavigate({ kind: "library", filter: null })}
      />
      <NavRow
        icon={<ActivityIcon />}
        label="Activity"
        active={section === "activity"}
        onClick={() => onNavigate({ kind: "activity" })}
      />
      <NavRow
        icon={<SettingsIcon />}
        label="Settings"
        active={section === "settings"}
        onClick={() => onNavigate({ kind: "settings" })}
      />
      <NavRow
        icon={<TeamIcon />}
        label="Team"
        badge="Paid"
        active={section === "team"}
        onClick={() => onNavigate({ kind: "team" })}
      />

      <Heading style={{ paddingTop: 20 }}>Filters</Heading>
      {VISIBILITY_ORDER.map((level) => {
        const info = VISIBILITY[level];
        const selected = filter === level;
        return (
          <button
            key={level}
            type="button"
            aria-pressed={selected}
            onClick={() =>
              onNavigate({ kind: "library", filter: selected ? null : level })
            }
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              padding: "8px 11px",
              borderRadius: 9,
              border: "none",
              textAlign: "left",
              cursor: "pointer",
              font: "500 13px var(--font-sans)",
              background: selected ? info.tint : "transparent",
              color: selected ? info.colour : "var(--ink2)",
            }}
          >
            {FILTER_ICON[level](info.colour)}
            {info.label}
            <span
              style={{
                marginLeft: "auto",
                font: "500 11px var(--font-mono)",
                color: selected ? info.colour : "var(--faint)",
              }}
            >
              {counts[level] ?? 0}
            </span>
          </button>
        );
      })}

      <div
        style={{
          marginTop: "auto",
          display: "flex",
          flexDirection: "column",
          gap: 10,
          paddingTop: 16,
        }}
      >
        <McpCard mcp={mcp} />
        <RelayCard />
        <AccountRow onNavigate={onNavigate} onQuit={onQuit} />
      </div>
    </nav>
  );
}

function McpCard({ mcp }: { mcp?: McpStatus }) {
  const running = mcp?.running ?? false;

  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 11,
        padding: 12,
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 7,
          font: "600 12px var(--font-sans)",
          marginBottom: 4,
        }}
      >
        <span style={{ position: "relative", width: 8, height: 8, flex: "none" }}>
          <span
            style={{
              position: "absolute",
              inset: 0,
              background: running ? "var(--green)" : "var(--idle)",
              borderRadius: "50%",
            }}
          />
          {running && (
            <span
              style={{
                position: "absolute",
                inset: 0,
                background: "var(--green)",
                borderRadius: "50%",
                animation: "kt-ping 1.8s ease-out infinite",
              }}
            />
          )}
        </span>
        MCP server
      </div>
      <div style={{ font: "400 10.5px var(--font-mono)", color: "var(--muted)" }}>
        {running
          ? `running · ${mcp?.tools ?? 0} tools${mcp?.port ? ` · :${mcp.port}` : ""}`
          : "not running"}
      </div>
    </div>
  );
}

/**
 * The relay upsell.
 *
 * Deliberately inert. The relay is a cloud feature and CLAUDE.md rule 10 keeps
 * its implementation out of this repo, so this card states what it is and stops
 * there rather than linking somewhere that cannot work yet.
 */
function RelayCard() {
  return (
    <div
      style={{
        background:
          "linear-gradient(135deg, var(--accent-tint), var(--gold-tint))",
        border: "1px solid var(--accent-bd)",
        borderRadius: 11,
        padding: 12,
      }}
    >
      <div style={{ font: "600 12px var(--font-sans)", marginBottom: 3 }}>
        Works away from home
      </div>
      <div
        style={{
          font: "400 11px/1.4 var(--font-sans)",
          color: "var(--ink3)",
          marginBottom: 8,
        }}
      >
        Keep links alive when your computer sleeps.
      </div>
      <div style={{ font: "600 11px var(--font-sans)", color: "var(--faint)" }}>
        Not available yet
      </div>
    </div>
  );
}

function AccountRow({
  onNavigate,
  onQuit,
}: {
  onNavigate: (surface: Surface) => void;
  onQuit: () => void;
}) {
  const [open, setOpen] = useState(false);

  return (
    <div style={{ position: "relative" }}>
      {open && (
        <>
          <button
            type="button"
            aria-label="Close menu"
            onClick={() => setOpen(false)}
            style={{
              position: "fixed",
              inset: 0,
              zIndex: 90,
              border: "none",
              background: "transparent",
              cursor: "default",
            }}
          />
          <div
            role="menu"
            style={{
              position: "absolute",
              bottom: "calc(100% + 8px)",
              left: 0,
              right: 0,
              zIndex: 95,
              background: "var(--paper)",
              border: "1px solid var(--border2)",
              borderRadius: 12,
              boxShadow: "0 22px 44px -14px rgba(0,0,0,.4)",
              overflow: "hidden",
            }}
          >
            <div
              style={{
                padding: 13,
                display: "flex",
                alignItems: "center",
                gap: 10,
                borderBottom: "1px solid var(--divider)",
              }}
            >
              <Avatar size={34} />
              <div style={{ minWidth: 0 }}>
                <div style={{ font: "700 13px var(--font-sans)" }}>This machine</div>
                <div style={{ font: "400 10px var(--font-mono)", color: "var(--faint)" }}>
                  root identity
                </div>
              </div>
            </div>
            <div style={{ padding: 6 }}>
              <MenuItem
                onClick={() => {
                  setOpen(false);
                  onNavigate({ kind: "settings" });
                }}
              >
                Settings
              </MenuItem>
              <MenuItem trailing="Local only">Relay</MenuItem>
            </div>
            <div style={{ padding: 6, borderTop: "1px solid var(--divider)" }}>
              <MenuItem trailing="⌘Q" onClick={onQuit}>
                Quit Kitchen Table
              </MenuItem>
            </div>
          </div>
        </>
      )}

      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        style={{
          width: "100%",
          display: "flex",
          alignItems: "center",
          gap: 9,
          padding: "6px 8px",
          borderRadius: 9,
          border: "none",
          background: open ? "var(--divider)" : "transparent",
          cursor: "pointer",
          textAlign: "left",
        }}
      >
        <Avatar size={28} />
        <div style={{ minWidth: 0, flex: 1 }}>
          <div
            style={{
              font: "600 12px var(--font-sans)",
              color: "var(--ink)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
          >
            This machine
          </div>
          <div style={{ font: "400 10px var(--font-mono)", color: "var(--faint)" }}>
            local only
          </div>
        </div>
        <ChevronUpIcon colour="var(--faint)" />
      </button>
    </div>
  );
}

function Avatar({ size }: { size: number }) {
  return (
    <span
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        flex: "none",
        borderRadius: size > 30 ? 9 : 8,
        background: "var(--chip-ink-bg)",
        color: "var(--chip-ink-fg)",
        font: `700 ${size > 30 ? 14 : 12}px var(--font-sans)`,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      ⌂
    </span>
  );
}

function MenuItem({
  children,
  trailing,
  onClick,
}: {
  children: React.ReactNode;
  trailing?: string;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      disabled={!onClick}
      style={{
        width: "100%",
        padding: "8px 10px",
        borderRadius: 7,
        border: "none",
        background: "transparent",
        cursor: onClick ? "pointer" : "default",
        font: "500 12.5px var(--font-sans)",
        color: onClick ? "var(--ink2)" : "var(--faint)",
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 10,
        textAlign: "left",
      }}
    >
      {children}
      {trailing && (
        <span style={{ font: "500 10.5px var(--font-mono)", color: "var(--faint)" }}>
          {trailing}
        </span>
      )}
    </button>
  );
}

function Heading({
  children,
  style,
}: {
  children: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <div
      style={{
        font: "600 10.5px var(--font-mono)",
        color: "var(--faint)",
        letterSpacing: "0.06em",
        textTransform: "uppercase",
        padding: "4px 10px 10px",
        ...style,
      }}
    >
      {children}
    </div>
  );
}

function NavRow({
  icon,
  label,
  count,
  badge,
  active,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  count?: number;
  badge?: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "9px 11px",
        borderRadius: 9,
        border: "none",
        cursor: "pointer",
        textAlign: "left",
        font: "600 13.5px var(--font-sans)",
        background: active ? "var(--accent-tint)" : "transparent",
        color: active ? "var(--accent)" : "var(--ink2)",
      }}
    >
      {icon}
      {label}
      {count !== undefined && (
        <span
          style={{
            marginLeft: "auto",
            font: "500 11px var(--font-mono)",
            opacity: 0.7,
          }}
        >
          {count}
        </span>
      )}
      {badge && (
        <span
          style={{
            marginLeft: "auto",
            font: "600 8.5px var(--font-mono)",
            letterSpacing: "0.06em",
            textTransform: "uppercase",
            color: "var(--accent)",
            background: "var(--accent-tint)",
            padding: "2px 6px",
            borderRadius: 5,
          }}
        >
          {badge}
        </span>
      )}
    </button>
  );
}
