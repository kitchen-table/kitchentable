import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { App } from "../generated";
import { Qr } from "../Qr";
import { type InviteView, Sharing } from "../Sharing";
import { type AccessEvent, ago, describe, sentence, size } from "../activity";
import { call, useStatus } from "../daemon";
import { openInstead } from "../external";
import { handover } from "../relay";
import { useDevices } from "../devices";
import { usePresence, viewerName, viewerWhen, viewerWhere } from "../presence";
import { DangerZone } from "./DangerZone";
import { Devices } from "./Devices";
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
  // The address under the app's name, on every tab. For a published app that
  // is its public one: it is the name the owner chose, the name they sent
  // people, and the first place they look to check it took.
  const hand = handover(app, useStatus(true).data?.relay);

  // Opening failing silently is the bug this button just had. If the OS will
  // not take the URL, say so and show the address, so it can be pasted.
  const [openError, setOpenError] = useState<string | null>(null);

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
              {hand.url.replace(/^https?:\/\//, "")} · v{app.version} · {app.entry}
            </div>
          </div>

          <div style={{ display: "flex", gap: 10, flex: "none" }}>
            <a
              // Loopback, not the announced `.local` name and not the address
              // URL. Both of those resolve to this machine's *LAN* address, so
              // neither earns the own-machine exemption in the gate: clicking
              // this on a Private app was answered 403, and on an Invited one
              // it asked the owner to pair with their own machine. Loopback is
              // the one address that always opens for the person sitting here.
              href={app.loopback_url}
              title={`Open ${app.loopback_url} on this machine`}
              target="_blank"
              rel="noreferrer"
              // A webview has no second tab to open into, so `_blank` alone is
              // swallowed and this button did nothing in the one place it is
              // most likely to be pressed. See `external.ts`.
              onClick={openInstead(app.loopback_url, (error) => setOpenError(message(error)))}
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

        {openError && (
          <p
            role="alert"
            style={{
              margin: "10px 0 0",
              font: "400 12px var(--font-sans)",
              color: "var(--danger)",
            }}
          >
            Could not open a browser — {openError}. The address is{" "}
            <code style={{ fontFamily: "var(--font-mono)" }}>{app.loopback_url}</code>.
          </p>
        )}

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
        {tab === "overview" && (
          <Overview app={app} onTab={onTab} onNavigate={onNavigate} />
        )}
        {tab === "sharing" && <Sharing app={app} />}
        {tab === "devices" && <Devices />}
        {tab !== "overview" && tab !== "sharing" && tab !== "devices" && (
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

function Overview({
  app,
  onTab,
  onNavigate,
}: {
  app: App;
  onTab: (tab: AppTab) => void;
  onNavigate: (surface: Surface) => void;
}) {
  const log = useQuery({
    queryKey: ["log", app.slug],
    queryFn: () => call<AccessEvent[]>("log.query", { slug: app.slug, limit: 100 }),
    retry: false,
  });

  // Which address the QR should carry. Shares react-query's ["status"] entry
  // with the Sharing tab and the status bar, so asking costs nothing.
  const hand = handover(app, useStatus(true).data?.relay);

  // Shares react-query's ["devices"] entry with the pairing badge and the
  // Devices tab, so naming the rows costs no extra traffic.
  const devices = useDevices();
  const nameOf = new Map((devices.data ?? []).map((device) => [device.id, device.name]));

  const invites = useQuery({
    queryKey: ["invites", app.slug],
    queryFn: () => call<InviteView[]>("share.list_invites", { slug: app.slug }),
    retry: false,
  });
  // The newest link someone could still open. A revoked or expired one is not
  // worth a panel offering to hand it round.
  const live = (invites.data ?? []).filter((invite) => invite.active).at(0);

  const presence = usePresence(app.slug);
  const viewers = presence.data ?? [];

  const events = log.data ?? [];
  const opens = events.filter((event) => event.action === "opened").length;

  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 300px", gap: 20 }}>
      <div style={{ display: "flex", flexDirection: "column", gap: 18, minWidth: 0 }}>
        <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 12 }}>
          {/* A real number now that pages check in. Still a dash until the
              first answer arrives, because zero is a claim and "not asked yet"
              is not the same as "nobody". */}
          <Stat
            value={presence.isPending ? "—" : String(viewers.length)}
            label="online now"
            accent
          />
          <Stat value={String(opens)} label="total opens" />
          {/* "v7", not "7": the version reads as a name everywhere else in the
              product, and the label carries when it landed. */}
          <Stat
            value={`v${app.version}`}
            label={
              app.deployed_at ? `deployed ${ago(app.deployed_at)} ago` : "current version"
            }
          />
          <Stat value={size(app.size_bytes)} label="bundle size" />
        </div>

        {/* Above the history, as the mockup has it: who is reading it now is
            the question an owner asks first. Known only because the page says
            so - see the `live` module in kt-server - which is why an app nobody
            has opened since the daemon started shows nobody. */}
        <Panel title="Live now">
          {viewers.length === 0 ? (
            <Empty>Nobody has this open right now.</Empty>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
              {viewers.map((viewer) => (
                <div
                  key={viewer.device_id}
                  style={{ display: "flex", alignItems: "center", gap: 11 }}
                >
                  <span
                    aria-hidden="true"
                    style={{
                      width: 9,
                      height: 9,
                      borderRadius: "50%",
                      flex: "none",
                      background: "var(--green)",
                    }}
                  />
                  <span style={{ font: "600 13px var(--font-sans)" }}>
                    {viewerName(viewer, nameOf)}
                  </span>
                  <span style={{ font: "400 12px var(--font-sans)", color: "var(--muted)" }}>
                    viewing {viewerWhere(viewer)}
                  </span>
                  <span
                    style={{
                      marginLeft: "auto",
                      font: "400 11px var(--font-mono)",
                      color: "var(--faint)",
                    }}
                  >
                    {viewerWhen(viewer)}
                  </span>
                </div>
              ))}
            </div>
          )}
        </Panel>

        <Panel title="Recent activity" action={{ label: "View all", onClick: () => onTab("activity") }}>
          {events.length === 0 ? (
            <Empty>
              Nothing yet. Opens, pairings and refusals all land here.
            </Empty>
          ) : (
            <div style={{ display: "flex", flexDirection: "column" }}>
              {events.slice(0, 5).map((event, index) => (
                <Row
                  key={`${event.at}-${index}`}
                  event={event}
                  deviceName={event.device_id ? nameOf.get(event.device_id) : undefined}
                />
              ))}
            </div>
          )}
        </Panel>

        <DangerZone
          app={app}
          onGone={() => onNavigate({ kind: "library", filter: null })}
        />
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
          <Qr value={hand.url} size={264} title={`Scan to open ${app.name}`} />
          <div
            style={{
              marginTop: 10,
              font: "500 11px var(--font-mono)",
              color: "var(--muted)",
              wordBreak: "break-all",
            }}
          >
            {hand.url.replace(/^https?:\/\//, "")}
          </div>
          {hand.away && (
            // Worth saying out loud: this is the code that still works after
            // they have gone home, which is the difference the relay buys.
            <div
              style={{
                marginTop: 6,
                font: "500 10.5px var(--font-sans)",
                color: "var(--green)",
              }}
            >
              Works away from your network
            </div>
          )}
          {hand.alternative && (
            <div
              style={{
                marginTop: 6,
                font: "400 10.5px var(--font-mono)",
                color: "var(--faint)",
                wordBreak: "break-all",
              }}
            >
              or {hand.alternative.replace(/^https?:\/\//, "")}
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
          {/* The mockup shows the live invite link here, which is the thing
              someone actually wants to hand over. Without one there is nothing
              to show, so the panel says who can open the app instead - the
              question it is standing in for either way. */}
          {live ? (
            <>
              <div style={{ font: "600 12.5px var(--font-sans)", marginBottom: 8 }}>
                Invite link · active
              </div>
              <div
                style={{
                  font: "400 11.5px var(--font-mono)",
                  color: "var(--accent)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                  marginBottom: 10,
                }}
              >
                {live.url.replace(/^https?:\/\//, "")}
              </div>
            </>
          ) : (
            <div style={{ font: "600 12.5px var(--font-sans)", marginBottom: 8 }}>
              {VISIBILITY[app.visibility].label} · {VISIBILITY[app.visibility].blurb}
            </div>
          )}
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

/**
 * One access-log line. Shared by Overview and both Activity views.
 *
 * `deviceName` is looked up by the caller rather than fetched here: a list of
 * a hundred rows would otherwise be a hundred subscriptions to the same query.
 */
export function Row({
  event,
  app,
  deviceName,
}: {
  event: AccessEvent;
  app?: string;
  deviceName?: string;
}) {
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
        {sentence(event, deviceName)}
        {event.detail ? ` — ${event.detail}` : ""}
      </span>
      <span style={{ font: "400 11px var(--font-mono)", color: "var(--faint)", flex: "none" }}>
        {ago(event.at)}
      </span>
    </div>
  );
}

function message(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return String(error);
}

export function Empty({ children }: { children: React.ReactNode }) {
  return (
    <p style={{ margin: 0, font: "400 13px var(--font-sans)", color: "var(--ink3)" }}>
      {children}
    </p>
  );
}
