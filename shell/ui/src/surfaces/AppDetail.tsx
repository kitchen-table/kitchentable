import { useQuery } from "@tanstack/react-query";
import type { App } from "../generated";
import { Qr } from "../Qr";
import { Sharing } from "../Sharing";
import { type AccessEvent, ago, describe, size } from "../activity";
import { call } from "../daemon";
import { APP_TABS, APP_TAB_LABELS, type AppTab, type Surface } from "../navigation";
import { VISIBILITY, tileColour } from "../visibility";

/**
 * One app, in depth: the six tabs from the desktop mockup.
 *
 * The header is shared by every tab so the app you are looking at, its URL and
 * its visibility never leave the screen - changing sharing is the most
 * consequential thing you can do here, and doing it without the app's name in
 * view is how people share the wrong one.
 */
export function AppDetail({
  app,
  tab,
  onTab,
  onNavigate,
}: {
  app: App;
  tab: AppTab;
  onTab: (tab: AppTab) => void;
  onNavigate: (surface: Surface) => void;
}) {
  const vis = VISIBILITY[app.visibility];

  return (
    <div
      className="kt-scroll"
      style={{ flex: 1, minWidth: 0, overflowY: "auto", display: "flex", flexDirection: "column" }}
    >
      <header
        style={{
          flex: "none",
          padding: "22px 30px 0",
          background: "var(--raised)",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <button
          type="button"
          onClick={() => onNavigate({ kind: "library", filter: null })}
          style={{
            display: "inline-flex",
            alignItems: "center",
            gap: 6,
            border: "none",
            background: "none",
            padding: 0,
            marginBottom: 16,
            font: "500 12.5px var(--font-sans)",
            color: "var(--muted)",
            cursor: "pointer",
          }}
        >
          ← Library
        </button>

        <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
          <span
            aria-hidden="true"
            style={{
              width: 56,
              height: 56,
              borderRadius: 14,
              flex: "none",
              background: tileColour(app.slug),
              color: "#fff",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              font: "700 24px var(--font-sans)",
            }}
          >
            {app.name.charAt(0).toUpperCase()}
          </span>

          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <h1
                style={{
                  margin: 0,
                  font: "800 24px var(--font-sans)",
                  letterSpacing: "-0.02em",
                }}
              >
                {app.name}
              </h1>
              <span
                style={{
                  font: "600 10.5px var(--font-mono)",
                  color: vis.colour,
                  background: vis.tint,
                  padding: "4px 9px",
                  borderRadius: 6,
                }}
              >
                {vis.label}
              </span>
            </div>
            <div
              style={{
                marginTop: 4,
                font: "400 12px var(--font-mono)",
                color: "var(--muted)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
            >
              {app.url.replace(/^https?:\/\//, "")} · v{app.version} · {app.entry}
            </div>
          </div>

          <div style={{ display: "flex", gap: 10, flex: "none" }}>
            <a
              href={app.url}
              target="_blank"
              rel="noreferrer"
              style={{
                padding: "9px 15px",
                borderRadius: 9,
                border: "1px solid var(--border2)",
                font: "600 12.5px var(--font-sans)",
                color: "var(--ink2)",
              }}
            >
              Open ↗
            </a>
            <button
              type="button"
              onClick={() => onTab("sharing")}
              style={{
                padding: "9px 15px",
                borderRadius: 9,
                border: "none",
                background: "var(--accent)",
                color: "#fff",
                font: "600 12.5px var(--font-sans)",
                cursor: "pointer",
              }}
            >
              Share
            </button>
          </div>
        </div>

        <div role="tablist" style={{ display: "flex", gap: 24, marginTop: 20 }}>
          {APP_TABS.map((value) => {
            const active = value === tab;
            return (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={active}
                onClick={() => onTab(value)}
                style={{
                  padding: "0 0 13px",
                  border: "none",
                  borderBottom: `2px solid ${active ? "var(--accent)" : "transparent"}`,
                  background: "none",
                  font: "600 13.5px var(--font-sans)",
                  color: active ? "var(--accent)" : "var(--ink3)",
                  cursor: "pointer",
                }}
              >
                {APP_TAB_LABELS[value]}
              </button>
            );
          })}
        </div>
      </header>

      <div role="tabpanel" style={{ padding: "24px 30px", flex: 1, minHeight: 0 }}>
        {!app.entry_exists && <NoEntry app={app} />}
        {tab === "overview" && <Overview app={app} onTab={onTab} />}
        {tab === "sharing" && <Sharing app={app} />}
        {tab !== "overview" && tab !== "sharing" && (
          <p style={{ font: "400 14px var(--font-sans)", color: "var(--faint)" }}>
            {APP_TAB_LABELS[tab]} is not built yet.
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * The one failure that hides completely.
 *
 * An app with no entry file is registered, announced and gated exactly like a
 * working one; the only symptom is a 404, on a phone, after someone has already
 * been sent the link. So it is stated at the top of every tab, with both fixes
 * spelled out rather than described.
 */
function NoEntry({ app }: { app: App }) {
  return (
    <div
      role="alert"
      style={{
        background: "var(--danger-tint)",
        border: "1px solid var(--danger)",
        borderRadius: 12,
        padding: 16,
        marginBottom: 20,
      }}
    >
      <div
        style={{
          font: "700 13.5px var(--font-sans)",
          color: "var(--danger)",
          marginBottom: 5,
        }}
      >
        This app has no page to open
      </div>
      <div style={{ font: "400 12.5px/1.6 var(--font-sans)", color: "var(--ink2)" }}>
        Its folder has no{" "}
        <code style={{ fontFamily: "var(--font-mono)" }}>{app.entry}</code>, so the
        address above answers 404. Either rename a page in the folder to{" "}
        <code style={{ fontFamily: "var(--font-mono)" }}>{app.entry}</code>, or point{" "}
        <code style={{ fontFamily: "var(--font-mono)" }}>entry</code> in{" "}
        <code style={{ fontFamily: "var(--font-mono)" }}>app.json</code> at the file
        you want opened first.
      </div>
      <div
        style={{
          marginTop: 8,
          font: "400 11px var(--font-mono)",
          color: "var(--faint)",
          wordBreak: "break-all",
        }}
      >
        {app.path}
      </div>
    </div>
  );
}

function Overview({ app, onTab }: { app: App; onTab: (tab: AppTab) => void }) {
  const log = useQuery({
    queryKey: ["log", app.slug],
    queryFn: () => call<AccessEvent[]>("log.query", { slug: app.slug, limit: 100 }),
    retry: false,
  });

  const events = log.data ?? [];
  const opens = events.filter((event) => event.action === "opened").length;

  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 300px", gap: 20 }}>
      <div style={{ display: "flex", flexDirection: "column", gap: 18, minWidth: 0 }}>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 12 }}>
          {/* "Online now" needs a live view of who is connected, which arrives
              with event.* subscriptions. A dash is honest; a zero would not be. */}
          <Stat value="—" label="online now" accent />
          <Stat value={String(opens)} label="total opens" />
          <Stat
            value={String(app.version)}
            label={
              app.deployed_at ? `changed ${ago(app.deployed_at)} ago` : "current version"
            }
          />
          <Stat value={size(app.size_bytes)} label="bundle size" />
        </div>

        <Panel title="Recent activity" action={{ label: "View all", onClick: () => onTab("activity") }}>
          {events.length === 0 ? (
            <Empty>
              Nothing yet. Opens, pairings and refusals all land here.
            </Empty>
          ) : (
            <div style={{ display: "flex", flexDirection: "column" }}>
              {events.slice(0, 5).map((event, index) => (
                <Row key={`${event.at}-${index}`} event={event} />
              ))}
            </div>
          )}
        </Panel>
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <div
          style={{
            background: "var(--paper)",
            border: "1px solid var(--border)",
            borderRadius: 12,
            padding: 18,
          }}
        >
          <div style={{ font: "700 13px var(--font-sans)", marginBottom: 12 }}>
            Hand it to a phone
          </div>
          <Qr value={app.url} size={264} title={`Scan to open ${app.name}`} />
          <div
            style={{
              marginTop: 10,
              font: "500 11px var(--font-mono)",
              color: "var(--muted)",
              wordBreak: "break-all",
            }}
          >
            {app.url.replace(/^https?:\/\//, "")}
          </div>
          {app.hostname === undefined && (
            <div
              style={{
                marginTop: 6,
                font: "400 10.5px var(--font-mono)",
                color: "var(--faint)",
                wordBreak: "break-all",
              }}
            >
              or {app.fallback_url.replace(/^https?:\/\//, "")}
            </div>
          )}
        </div>

        <button
          type="button"
          onClick={() => onTab("sharing")}
          style={{
            background: "var(--accent-tint)",
            border: "1px solid var(--accent-bd)",
            borderRadius: 12,
            padding: 16,
            textAlign: "left",
            cursor: "pointer",
          }}
        >
          <div style={{ font: "600 12.5px var(--font-sans)", marginBottom: 8 }}>
            {VISIBILITY[app.visibility].label} · {VISIBILITY[app.visibility].blurb}
          </div>
          <span
            style={{
              display: "block",
              textAlign: "center",
              background: "var(--accent)",
              color: "#fff",
              padding: 9,
              borderRadius: 9,
              font: "600 12.5px var(--font-sans)",
            }}
          >
            Manage sharing
          </span>
        </button>
      </div>
    </div>
  );
}

function Stat({
  value,
  label,
  accent,
}: {
  value: string;
  label: string;
  accent?: boolean;
}) {
  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: 15,
      }}
    >
      <div
        style={{
          font: "800 24px var(--font-sans)",
          color: accent ? "var(--accent)" : "var(--ink)",
        }}
      >
        {value}
      </div>
      <div
        style={{
          font: "400 11.5px var(--font-sans)",
          color: "var(--muted)",
          marginTop: 2,
        }}
      >
        {label}
      </div>
    </div>
  );
}

export function Panel({
  title,
  action,
  children,
}: {
  title: string;
  action?: { label: string; onClick: () => void };
  children: React.ReactNode;
}) {
  return (
    <section
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: 18,
      }}
    >
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          marginBottom: 12,
        }}
      >
        <h2 style={{ margin: 0, font: "700 14px var(--font-sans)" }}>{title}</h2>
        {action && (
          <button
            type="button"
            onClick={action.onClick}
            style={{
              border: "none",
              background: "none",
              font: "500 12px var(--font-sans)",
              color: "var(--accent)",
              cursor: "pointer",
            }}
          >
            {action.label}
          </button>
        )}
      </div>
      {children}
    </section>
  );
}

/** One access-log line. Shared by Overview and both Activity views. */
export function Row({ event, app }: { event: AccessEvent; app?: string }) {
  const kind = describe(event.action);

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "9px 0",
        borderTop: "1px solid var(--divider)",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 26,
          height: 26,
          borderRadius: 7,
          flex: "none",
          background: kind.tint,
          color: kind.colour,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          font: "600 11px var(--font-mono)",
        }}
      >
        {kind.glyph}
      </span>
      <span
        style={{
          font: "500 12.5px var(--font-sans)",
          flex: 1,
          minWidth: 0,
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
        }}
      >
        {app && <b style={{ fontWeight: 700 }}>{app}</b>}
        {app && " · "}
        {kind.title}
        {event.detail ? ` — ${event.detail}` : ""}
      </span>
      <span style={{ font: "400 11px var(--font-mono)", color: "var(--faint)", flex: "none" }}>
        {ago(event.at)}
      </span>
    </div>
  );
}

export function Empty({ children }: { children: React.ReactNode }) {
  return (
    <p style={{ margin: 0, font: "400 13px var(--font-sans)", color: "var(--ink3)" }}>
      {children}
    </p>
  );
}
