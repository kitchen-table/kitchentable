import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { type AccessEvent, describe, meta, sentence, when } from "../activity";
import { call } from "../daemon";
import {
  type Device,
  describeAgent,
  deviceShape,
  pending as pendingDevices,
  useDeviceActions,
  useDevices,
} from "../devices";
import type { App, NotifyPrefs } from "../generated";
import { DeviceShapeIcon } from "../icons";
import {
  type NotifyState,
  pairingSpec,
  show,
  useAskPermission,
  useNotifyPrefs,
  useNotifyState,
  useSetNotifyPrefs,
} from "../notify";
import { VISIBILITY } from "../visibility";
import { Switch } from "./Settings";

/**
 * The Notifications surface, from `uimockups/Desktop App.dc.html`.
 *
 * Four blocks, in the mockup's order: what needs an answer, what happened while
 * you were away, which of those are worth interrupting you on the desktop, and
 * a drawing of what that looks like when it does.
 *
 * Two things here are not in the mockup, and both exist because the mockup is a
 * drawing and this is a program that has to tell the truth about macOS:
 *
 * - **The permission row.** macOS decides whether any of this appears at all,
 *   it asks once, and it never asks again. A page of switches over a permission
 *   that was refused would be four controls that do nothing.
 * - **Two switches carry a reason instead of working.** Nothing writes a deploy
 *   (product.md 5.10) and nothing detects an app going offline, so those two
 *   can only ever be settings for events that never arrive. Drawn, disabled,
 *   and told why - the rule the Sharing tab set for Strict.
 */
export function Notifications() {
  const devices = useDevices();
  const waiting = pendingDevices(devices.data);
  const prefs = useNotifyPrefs();
  const permission = useNotifyState();

  return (
    <div className="kt-scroll" style={{ flex: 1, overflowY: "auto" }}>
      <div
        style={{
          padding: "26px 30px",
          maxWidth: 860,
          display: "flex",
          flexDirection: "column",
          gap: 22,
        }}
      >
        <Header waiting={waiting} permission={permission.data} />
        <NeedsApproval waiting={waiting} loading={devices.isPending} />
        <Earlier />
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "1fr 1fr",
            gap: 16,
            alignItems: "start",
          }}
        >
          <Preferences
            prefs={prefs.data}
            permission={permission.data}
            loading={prefs.isPending}
          />
          <BannerPreview />
        </div>
      </div>
    </div>
  );
}

/**
 * The title, and the mockup's Preview button.
 *
 * The preview posts a real notification rather than drawing one, because the
 * question it answers is "will I see these?" - and on macOS that depends on
 * Focus, on the alert style, and on a permission this app cannot read the whole
 * of. Only the real thing answers it. It borrows a waiting device when there is
 * one so the buttons in the preview are the buttons you would actually press.
 */
function Header({
  waiting,
  permission,
}: {
  waiting: Device[];
  permission?: NotifyState;
}) {
  const usable = permission?.state === "granted";

  return (
    <div style={{ display: "flex", alignItems: "flex-end", justifyContent: "space-between" }}>
      <div>
        <h1
          style={{
            margin: "0 0 4px",
            font: "800 26px var(--font-sans)",
            letterSpacing: "-0.02em",
          }}
        >
          Notifications
        </h1>
        <p style={{ margin: 0, font: "400 13px var(--font-mono)", color: "var(--muted)" }}>
          access requests and what happened while you were away
        </p>
      </div>

      <button
        type="button"
        disabled={!usable}
        title={usable ? undefined : "macOS is not showing notifications from Kitchen Table yet"}
        onClick={() => {
          const device = waiting[0];
          show(
            device
              ? pairingSpec(device)
              : {
                  title: "Kitchen Table is set up",
                  body: "This is what an access request will look like.",
                  sound: true,
                },
          );
        }}
        style={{
          font: "600 12.5px var(--font-sans)",
          color: usable ? "var(--ink2)" : "var(--faint)",
          background: "none",
          border: "1px solid var(--border2)",
          padding: "9px 14px",
          borderRadius: 9,
          cursor: usable ? "pointer" : "not-allowed",
        }}
      >
        Preview desktop banner
      </button>
    </div>
  );
}

/* -------------------------------------------------------- needs your approval */

function NeedsApproval({ waiting, loading }: { waiting: Device[]; loading: boolean }) {
  return (
    <section>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 12 }}>
        <h2
          style={{
            margin: 0,
            font: "600 11px var(--font-mono)",
            color: "var(--muted)",
            letterSpacing: "0.05em",
          }}
        >
          NEEDS YOUR APPROVAL
        </h2>
        {waiting.length > 0 && (
          <span
            style={{
              font: "700 10px var(--font-mono)",
              color: "#fff",
              background: "var(--danger)",
              padding: "2px 7px",
              borderRadius: 8,
            }}
          >
            {waiting.length}
          </span>
        )}
      </div>

      {waiting.length === 0 ? (
        <div
          style={{
            background: "var(--paper)",
            border: "1px solid var(--border)",
            borderRadius: 12,
            padding: 26,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            textAlign: "center",
          }}
        >
          {/* "Reading" and "nobody is waiting" are different claims, and this is
              the panel somebody checks to be sure nothing is at the door. */}
          {loading ? (
            <div style={{ font: "500 13px var(--font-sans)", color: "var(--ink3)" }}>
              Checking…
            </div>
          ) : (
            <>
              <svg
                viewBox="0 0 24 24"
                width="26"
                height="26"
                fill="none"
                stroke="var(--green)"
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
                style={{ marginBottom: 8 }}
              >
                <path d="M20 7 10 17l-5-5" />
              </svg>
              <div style={{ font: "600 13.5px var(--font-sans)" }}>You&rsquo;re all caught up</div>
              <div style={{ font: "400 12px var(--font-sans)", color: "var(--muted)" }}>
                No devices are waiting for access.
              </div>
            </>
          )}
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          {waiting.map((device) => (
            <Request key={device.id} device={device} />
          ))}
        </div>
      )}
    </section>
  );
}

/** One device at the door: who, which app, and the two buttons. */
function Request({ device }: { device: Device }) {
  const { approve, deny } = useDeviceActions();
  const busy = approve.isPending || deny.isPending;
  const asked = useAskedFor(device.id);

  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--gold-bd)",
        borderLeft: "3px solid var(--gold)",
        borderRadius: 12,
        padding: "15px 17px",
        display: "flex",
        alignItems: "center",
        gap: 14,
        flexWrap: "wrap",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 40,
          height: 40,
          flex: "none",
          borderRadius: 10,
          background: "var(--gold-tint)",
          color: "var(--gold)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <DeviceShapeIcon shape={deviceShape(device.user_agent)} size={19} />
      </span>

      <div style={{ flex: 1, minWidth: 180 }}>
        <div style={{ font: "700 14px var(--font-sans)" }}>
          {asked.name ? `${device.name} wants to open ${asked.name}` : `${device.name} wants access`}
        </div>
        <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)", marginTop: 2 }}>
          {device.fingerprint} · {describeAgent(device.user_agent)} · {when(device.first_seen)}
        </div>
      </div>

      {asked.level && (
        <span
          style={{
            flex: "none",
            font: "600 10px var(--font-mono)",
            color: "var(--ink3)",
            background: "var(--chip)",
            padding: "4px 9px",
            borderRadius: 6,
          }}
        >
          {asked.level}
        </span>
      )}

      <button
        type="button"
        disabled={busy}
        onClick={() => approve.mutate({ id: device.id })}
        style={{
          flex: "none",
          border: "none",
          background: "var(--accent)",
          color: "#fff",
          padding: "9px 15px",
          borderRadius: 9,
          font: "600 12.5px var(--font-sans)",
          cursor: busy ? "not-allowed" : "pointer",
        }}
      >
        Approve
      </button>
      <button
        type="button"
        disabled={busy}
        onClick={() => deny.mutate(device.id)}
        style={{
          flex: "none",
          border: "none",
          background: "none",
          color: "var(--danger)",
          font: "600 12.5px var(--font-sans)",
          cursor: busy ? "not-allowed" : "pointer",
        }}
      >
        Deny
      </button>
    </div>
  );
}

/**
 * Which app a waiting device asked for, and how that app is shared.
 *
 * Devices are trusted across the whole workspace and carry no app of their own,
 * so this is recovered from the `requested` row the gate writes. Both halves are
 * optional on purpose: a request can arrive before its log row is readable, and
 * a row can name an app that has since been forgotten.
 */
function useAskedFor(deviceId: string): { name?: string; level?: string } {
  const log = useQuery({
    queryKey: ["log", null],
    queryFn: () => call<AccessEvent[]>("log.query", { limit: 500 }),
    retry: false,
  });
  const apps = useQuery({
    queryKey: ["apps"],
    queryFn: () => call<App[]>("app.list"),
    retry: false,
  });

  return useMemo(() => {
    const slug = (log.data ?? []).find(
      (entry) => entry.device_id === deviceId && entry.action === "requested",
    )?.app_slug;
    if (!slug) return {};

    const app = (apps.data ?? []).find((candidate) => candidate.slug === slug);
    return { name: app?.name ?? slug, level: app && VISIBILITY[app.visibility].label };
  }, [apps.data, deviceId, log.data]);
}

/* --------------------------------------------------------------------- earlier */

/** How many rows the surface keeps. The daemon caps at 1000 regardless. */
const EARLIER_LIMIT = 50;

/**
 * What happened while you were away, with the unread ones marked.
 *
 * Everything except the requests above, which have their own block: a row that
 * still needs an answer should not be sitting in a list of things that are over.
 *
 * Opening the surface marks it read, at the time of the newest row we actually
 * had - not at "now", which would swallow anything logged between the fetch and
 * the write.
 */
function Earlier() {
  const client = useQueryClient();
  const log = useQuery({
    queryKey: ["log", null],
    queryFn: () => call<AccessEvent[]>("log.query", { limit: 500 }),
    retry: false,
  });
  const devices = useDevices();
  const seen = useQuery({
    queryKey: ["notify-seen"],
    queryFn: () => call<{ at: number }>("notify.seen"),
    retry: false,
  });

  const rows = (log.data ?? [])
    .filter((entry) => entry.action !== "requested")
    .slice(0, EARLIER_LIMIT);

  const newest = rows[0]?.at;
  const [openedAt] = useState(() => Date.now());

  const markSeen = useMutation({
    mutationFn: (at: number) => call<{ at: number }>("notify.mark_seen", { at }),
    onSuccess: (answer) => client.setQueryData(["notify-seen"], answer),
  });

  // Drawn from the mark as it was when this surface opened, so rows do not
  // lose their dot while somebody is still reading them.
  const [markAtOpen, setMarkAtOpen] = useState<number | null>(null);
  if (markAtOpen === null && seen.data) setMarkAtOpen(seen.data.at);

  // In an effect, not in the render: writing during a render fires again on
  // every re-render until the answer lands, which is a request per keystroke
  // elsewhere in the window. The daemon's answer updates `seen`, which is what
  // stops this from running a second time.
  const mark = markSeen.mutate;
  const seenAt = seen.data?.at;
  useEffect(() => {
    if (newest === undefined || seenAt === undefined) return;
    if (newest > seenAt) mark(newest);
  }, [mark, newest, seenAt]);

  const named = new Map((devices.data ?? []).map((device) => [device.id, device]));

  return (
    <section>
      <h2
        style={{
          margin: "0 0 12px",
          font: "600 11px var(--font-mono)",
          color: "var(--muted)",
          letterSpacing: "0.05em",
        }}
      >
        EARLIER
      </h2>
      <div
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderRadius: 12,
          padding: "4px 18px",
        }}
      >
        {log.isError ? (
          <Note>The daemon did not answer. This list will fill in when it does.</Note>
        ) : log.isPending ? (
          <Note>Reading the log…</Note>
        ) : rows.length === 0 ? (
          <Note>Nothing has happened yet.</Note>
        ) : (
          rows.map((entry, index) => {
            const device = entry.device_id ? named.get(entry.device_id) : undefined;
            const kind = describe(entry.action);
            const unread = markAtOpen !== null && entry.at > markAtOpen;

            return (
              <div
                key={`${entry.at}-${entry.action}-${index}`}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 14,
                  padding: "14px 0",
                  borderBottom: "1px solid var(--divider)",
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    width: 32,
                    height: 32,
                    flex: "none",
                    borderRadius: 8,
                    background: kind.tint,
                    color: kind.colour,
                    font: "700 14px var(--font-sans)",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                  }}
                >
                  {kind.glyph}
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ font: "600 13.5px var(--font-sans)" }}>
                    {sentence(entry, device?.name)}
                  </div>
                  <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)" }}>
                    {meta(entry, {
                      fingerprint: device?.fingerprint,
                      agent: device ? describeAgent(device.user_agent) : undefined,
                    })}
                  </div>
                </div>
                {unread && (
                  <span
                    aria-label="Unread"
                    role="img"
                    style={{
                      width: 7,
                      height: 7,
                      flex: "none",
                      borderRadius: "50%",
                      background: "var(--accent)",
                    }}
                  />
                )}
                <span
                  style={{
                    flex: "none",
                    font: "400 11px var(--font-mono)",
                    color: "var(--faint)",
                    whiteSpace: "nowrap",
                  }}
                >
                  {when(entry.at, openedAt / 1000)}
                </span>
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}

/* ----------------------------------------------------------------- preferences */

/**
 * The four switches, over a row that says whether macOS will honour any of them.
 *
 * Each switch is described by what it is a setting *for*, so the two that
 * cannot fire yet say so in the same place the others say what they do.
 */
const SWITCHES: Array<{
  key: keyof NotifyPrefs;
  label: string;
  hint: string;
  /** Why this cannot fire yet, if it cannot. */
  unavailable?: string;
}> = [
  { key: "requests", label: "A device requests access", hint: "the one that needs your call" },
  { key: "opens", label: "Someone opens an app", hint: "can get chatty" },
  {
    key: "deploys",
    label: "A deploy finishes",
    hint: "yours or an agent's",
    unavailable: "Nothing records a deploy until versions are built",
  },
  {
    key: "offline",
    label: "An app goes offline",
    hint: "crash, sleep, or network drop",
    unavailable: "Nothing detects an app going offline yet",
  },
];

function Preferences({
  prefs,
  permission,
  loading,
}: {
  prefs?: NotifyPrefs;
  permission?: NotifyState;
  loading: boolean;
}) {
  const save = useSetNotifyPrefs();

  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: 18,
      }}
    >
      <h2 style={{ margin: "0 0 3px", font: "700 14px var(--font-sans)" }}>
        Notify me on the desktop when…
      </h2>
      <p
        style={{
          margin: "0 0 6px",
          font: "400 11.5px var(--font-sans)",
          color: "var(--muted)",
        }}
      >
        macOS banners, shown even when Kitchen Table is in the background.
      </p>

      <Permission state={permission} />

      {SWITCHES.map(({ key, label, hint, unavailable }) => (
        <div
          key={key}
          style={{
            display: "flex",
            alignItems: "center",
            gap: 12,
            padding: "11px 0",
            borderTop: "1px solid var(--divider)",
          }}
          title={unavailable}
        >
          <div style={{ flex: 1 }}>
            <div
              style={{
                font: "500 13px var(--font-sans)",
                color: unavailable ? "var(--ink3)" : "var(--ink)",
              }}
            >
              {label}
            </div>
            <div style={{ font: "400 10.5px var(--font-mono)", color: "var(--muted)" }}>
              {unavailable ?? hint}
            </div>
          </div>
          <Switch
            label={label}
            on={prefs?.[key] ?? false}
            disabled={loading || unavailable !== undefined || save.isPending}
            onToggle={() => save.mutate({ [key]: !(prefs?.[key] ?? false) })}
          />
        </div>
      ))}
    </div>
  );
}

/**
 * What macOS says, and the one action that can change it.
 *
 * `denied` deliberately offers no button. macOS asks once and remembers; a
 * second prompt never appears, so a button here would do nothing and look
 * broken. System Settings is the only way back, and saying that is more use
 * than a control that lies.
 */
function Permission({ state }: { state?: NotifyState }) {
  const ask = useAskPermission();
  if (!state) return null;

  const line = (tone: string, text: string, action?: React.ReactNode) => (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "10px 0",
        borderTop: "1px solid var(--divider)",
      }}
    >
      <span
        aria-hidden="true"
        style={{ width: 8, height: 8, borderRadius: "50%", background: tone, flex: "none" }}
      />
      <span style={{ flex: 1, font: "400 12px/1.5 var(--font-sans)", color: "var(--ink2)" }}>
        {text}
      </span>
      {action}
    </div>
  );

  switch (state.state) {
    case "granted":
      return state.alerts
        ? line("var(--green)", "macOS is showing these.")
        : line(
            "var(--gold)",
            "Allowed, but banners are switched off, so these go straight to Notification Center. System Settings › Notifications › Kitchen Table.",
          );

    case "not_determined":
      return line(
        "var(--gold)",
        "macOS has not been asked yet.",
        <button
          type="button"
          disabled={ask.isPending}
          onClick={() => ask.mutate()}
          style={{
            flex: "none",
            border: "none",
            background: "var(--accent)",
            color: "#fff",
            padding: "7px 12px",
            borderRadius: 8,
            font: "600 12px var(--font-sans)",
            cursor: ask.isPending ? "not-allowed" : "pointer",
          }}
        >
          {ask.isPending ? "Asking…" : "Turn on"}
        </button>,
      );

    case "denied":
      return line(
        "var(--danger)",
        "macOS is blocking notifications from Kitchen Table. It only asks once, so this has to be changed in System Settings › Notifications › Kitchen Table.",
      );

    case "unbundled":
      return line(
        "var(--muted)",
        "Notifications need the built app: macOS will not deliver them to a development build.",
      );

    case "unsupported":
      return line("var(--muted)", "Desktop notifications are macOS-only for now.");
  }
}

/* --------------------------------------------------------------- banner preview */

/** The mockup's illustration of what lands on the desktop. */
function BannerPreview() {
  return (
    <div>
      <h2
        style={{
          margin: "0 0 10px",
          font: "600 11px var(--font-mono)",
          color: "var(--muted)",
          letterSpacing: "0.05em",
        }}
      >
        HOW THEY LOOK ON YOUR DESKTOP
      </h2>
      <div
        style={{
          borderRadius: 16,
          padding: "20px 16px",
          background: "linear-gradient(160deg,#4a6b9c,#2d3c5c)",
          display: "flex",
          flexDirection: "column",
          gap: 12,
        }}
      >
        <div
          style={{
            background: "rgba(250,249,247,.82)",
            backdropFilter: "blur(20px)",
            borderRadius: 16,
            boxShadow: "0 12px 30px -8px rgba(0,0,0,.4)",
            padding: "13px 15px",
            display: "flex",
            gap: 12,
          }}
        >
          <span
            aria-hidden="true"
            style={{
              width: 38,
              height: 38,
              flex: "none",
              borderRadius: 9,
              background: "var(--accent)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <svg viewBox="0 0 32 32" width="20" height="20" aria-hidden="true">
              <rect x="4" y="11" width="24" height="5" rx="2.5" fill="#fff" />
              <rect x="8" y="16" width="3.4" height="11" rx="1.7" fill="#0f2a55" />
              <rect x="20.6" y="16" width="3.4" height="11" rx="1.7" fill="#0f2a55" />
            </svg>
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ display: "flex", alignItems: "baseline", gap: 6 }}>
              <span style={{ font: "700 12.5px var(--font-sans)", color: "#1a1a1a" }}>
                Kitchen Table
              </span>
              <span
                style={{
                  font: "400 10px var(--font-mono)",
                  color: "#6b6b6b",
                  marginLeft: "auto",
                }}
              >
                now
              </span>
            </div>
            <div
              style={{
                font: "400 12px/1.35 var(--font-sans)",
                color: "#1a1a1a",
                marginTop: 1,
              }}
            >
              Priya&rsquo;s iPad wants to open <b>Trip Planner</b>
            </div>
            <div style={{ display: "flex", gap: 7, marginTop: 9 }}>
              <span
                style={{
                  flex: 1,
                  textAlign: "center",
                  background: "#2f6df0",
                  color: "#fff",
                  padding: 7,
                  borderRadius: 8,
                  font: "600 11.5px var(--font-sans)",
                }}
              >
                Approve
              </span>
              <span
                style={{
                  flex: 1,
                  textAlign: "center",
                  background: "rgba(0,0,0,.06)",
                  color: "#1a1a1a",
                  padding: 7,
                  borderRadius: 8,
                  font: "600 11.5px var(--font-sans)",
                }}
              >
                Deny
              </span>
            </div>
          </div>
        </div>
      </div>
      <p
        style={{
          margin: "9px 0 0",
          font: "400 11px/1.5 var(--font-sans)",
          color: "var(--muted)",
          textWrap: "pretty",
        }}
      >
        Approve or deny right from the banner — no need to open the app. On macOS the buttons
        appear when the banner is expanded, or straight away if you set Kitchen Table&rsquo;s
        style to Alerts in System Settings.
      </p>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return (
    <p
      style={{
        margin: 0,
        padding: "16px 0",
        font: "400 13px var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      {children}
    </p>
  );
}
