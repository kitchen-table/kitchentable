import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import type { SysStatus } from "../generated";
import { call, restartDaemon, useHealth } from "../daemon";
import { useLaunchAtLogin } from "../launchAtLogin";
import { type Appearance } from "../theme";

/**
 * The settings surface, from `uimockups/Desktop App.dc.html`.
 *
 * The mockup draws five cards. Four are here and one is not, and which is which
 * is the substance of this file - so it is written down rather than left to be
 * rediscovered:
 *
 * - **Workspace** is real, and is the reason this screen exists. The watched
 *   folder and launch-at-login both work.
 * - **Local network** keeps the mDNS row, which reports what the daemon is
 *   actually doing, and drops the mockup's certificate row: HTTPS is neither
 *   built nor designed, and a row whose only content is an explanation of its
 *   own absence is a paragraph nobody asked for. The status bar already says
 *   "Not encrypted" where it matters.
 * - **MCP server** says it is not running, because `kt-mcp` is a one-line stub.
 * - **Appearance** is real, and is the one control the mockup buries in a
 *   review-only card while describing genuine product behaviour.
 * - **Mockup states** is absent. It is scaffolding for reviewing the mockup -
 *   "flip the app into states it reaches in the field" - and shipping it would
 *   put a switch marked "Empty workspace" in front of somebody's real one.
 *
 * The rule the unavailable rows follow is the one the Sharing tab set for
 * Strict: render the row the mockup drew, disable it, and **say why**. A row
 * removed leaves somebody wondering; a row that looks live and refuses is worse
 * than either.
 */
export function Settings({
  status,
  appearance,
  onAppearance,
}: {
  status?: SysStatus;
  appearance: Appearance;
  onAppearance: (next: Appearance) => void;
}) {
  return (
    <div
      style={{
        padding: "26px 30px",
        maxWidth: 720,
        display: "flex",
        flexDirection: "column",
        gap: 16,
        overflowY: "auto",
      }}
    >
      <h1
        style={{
          font: "800 26px var(--font-sans)",
          letterSpacing: "-0.02em",
          margin: "0 0 4px",
        }}
      >
        Settings
      </h1>

      <Workspace status={status} />
      <LocalNetwork status={status} />
      <Mcp />
      <AppearanceCard appearance={appearance} onAppearance={onAppearance} />
      <RelayUpsell />
    </div>
  );
}

/* ------------------------------------------------------------------ workspace */

/**
 * Where the apps come from, and whether Kitchen Table starts with the Mac.
 *
 * The watched folder is stored by the daemon and read on its next start, so
 * this card's job after a successful pick is to say that plainly and offer the
 * restart - not to imply the change has already landed. Three states, all of
 * them real:
 *
 * - `KT_WORKSPACE` is set, so the folder is fixed for this run and Change would
 *   write a value nothing will read. The row says so.
 * - The choice is stored and matches what is being watched. Nothing to say.
 * - The choice is stored and does not match. A restart is pending, and the
 *   button appears only when the shell started the daemon it would restart.
 */
function Workspace({ status }: { status?: SysStatus }) {
  const queryClient = useQueryClient();
  const health = useHealth();
  const launch = useLaunchAtLogin();
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [restarting, setRestarting] = useState(false);

  const locked = status?.workspace_locked ?? false;

  const choose = useMutation({
    mutationFn: async () => {
      const chosen = await open({ directory: true, multiple: false });
      if (typeof chosen !== "string") return null;
      return call<{ pending: string; restart_required: boolean }>("sys.set_workspace", {
        path: chosen,
      });
    },
    onSuccess: (answer) => {
      setError(null);
      if (!answer) return; // cancelled
      setPending(answer.restart_required ? answer.pending : null);
    },
    // The daemon's refusals are written for somebody standing in a file picker
    // - "that is a git repository, and every folder inside a workspace gets an
    // app.json written into it" - so they are shown as they arrive.
    onError: (e: { message?: string }) => setError(e?.message ?? "That folder cannot be used."),
  });

  const restart = async () => {
    setRestarting(true);
    try {
      await restartDaemon();
      setPending(null);
      setError(null);
      // Everything the window is showing came from the old workspace.
      await queryClient.invalidateQueries();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRestarting(false);
    }
  };

  // `stop()` deliberately leaves a daemon the shell adopted alone - it may be
  // serving for a terminal session somebody still wants - so pressing Restart
  // on one would do nothing at all. Say who has to do it instead.
  const canRestart = health.data?.we_started_it === true;

  return (
    <Card title="Workspace">
      <Row
        title="Watched folder"
        detail={status?.workspace ?? "…"}
        mono
        action={
          locked ? undefined : (
            <TextButton onClick={() => choose.mutate()} disabled={choose.isPending}>
              {choose.isPending ? "Choosing…" : "Change"}
            </TextButton>
          )
        }
      />

      {locked && (
        <Note>
          Fixed by <Mono>KT_WORKSPACE</Mono> for this run, so it cannot be changed here. Unset it
          and restart to choose a folder in the window.
        </Note>
      )}

      {error && <Note tone="danger">{error}</Note>}

      {pending && (
        <Note tone="accent">
          Kitchen Table will watch <Mono>{pending}</Mono> from its next start. The apps below are
          still coming from the folder above.
          {canRestart ? (
            <>
              {" "}
              <TextButton onClick={restart} disabled={restarting}>
                {restarting ? "Restarting…" : "Restart now"}
              </TextButton>
            </>
          ) : (
            " The daemon is running outside this app, so restart it yourself to pick the change up."
          )}
        </Note>
      )}

      <Row
        title="Start Kitchen Table at login"
        detail={launchDetail(launch)}
        action={
          <Switch
            label="Start Kitchen Table at login"
            on={launch.on === true}
            // Not asked yet, or nothing to ask. Either way the switch must not
            // move: its position would be a claim about the owner's machine.
            disabled={!launch.available || launch.on === null || launch.busy}
            onToggle={launch.toggle}
          />
        }
      />
    </Card>
  );
}

function launchDetail(launch: ReturnType<typeof useLaunchAtLogin>): string {
  if (!launch.available) return "only available in the desktop app";
  if (launch.error) return launch.error;
  if (launch.on === null) return "checking…";
  return launch.on
    ? "your apps are served from the moment you log in"
    : "off — apps are served only while Kitchen Table is open";
}

/* -------------------------------------------------------------- local network */

/**
 * How apps are reached on this network.
 *
 * **The mDNS row is a status, not the mockup's switch, and that is a decision
 * rather than an omission.** Two reasons, either sufficient. Announcing is not
 * really ours to switch: on macOS 15+ it is the Local Network permission that
 * decides, and a switch drawn on beside a denied permission would be the exact
 * class of lie this screen exists to avoid. And turning it off would silently
 * take every app's storage with it - the storage API resolves an app from its
 * own hostname and there is no hostname without an announcement - so the switch
 * would quietly break a feature two tabs away. The status is what somebody
 * actually needs from this row.
 *
 * **The mockup's certificate row is not here.** HTTPS has not been designed yet,
 * let alone built - the route that avoids a certificate warning on a phone is
 * still an open question - and a row explaining an absence is a paragraph about
 * something the reader never asked for. The status bar's "Not encrypted" badge
 * already says the true thing at the moment it matters. Add the row when there
 * is a certificate to report on, and the card title gains its "& HTTPS" back
 * with it.
 */
function LocalNetwork({ status }: { status?: SysStatus }) {
  const serving = status?.serving;
  const degraded = serving?.state === "degraded" ? serving : undefined;
  const mdnsDown =
    degraded?.reason === "mdns_unavailable" || degraded?.reason === "local_network_denied";

  return (
    <Card title="Local network">
      <Row
        title="Names on the network"
        detail={
          !status
            ? "…"
            : mdnsDown
              ? degraded?.message ?? "Friendly .local names are not available."
              : "apps answer at appname.local · fallback: this Mac's IP address /appname"
        }
        action={
          <Chip tone={!status ? "neutral" : mdnsDown ? "gold" : "green"}>
            {!status ? "checking" : mdnsDown ? "unavailable" : "announcing"}
          </Chip>
        }
      />
      {mdnsDown && (
        <Note tone="gold">
          Apps are still reachable by IP address. Their storage is not: an app's data is keyed to
          its own hostname, so without a name on the network each app falls back to keeping data in
          the browser only.
        </Note>
      )}
    </Card>
  );
}

/* ------------------------------------------------------------------------ mcp */

/**
 * The agent surface, which does not exist yet.
 *
 * Rendered because the mockup's shape is worth keeping and because "can an
 * agent drive this" is a question people arrive with. `kt-mcp` is a one-line
 * stub, so the card says not running and does not list tools it cannot expose.
 */
function Mcp() {
  return (
    <Card
      title="MCP server"
      badge={<Chip tone="neutral">not running</Chip>}
    >
      <Note>
        Any MCP-capable agent on this machine will be able to do everything this window can, over
        the same socket. Not built yet — the server is a stub, so there is nothing to point an agent
        at and no tools to list.
      </Note>
    </Card>
  );
}

/* ----------------------------------------------------------------- appearance */

const APPEARANCES: { value: Appearance; label: string }[] = [
  { value: "system", label: "System" },
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
];

/**
 * Three choices, not a switch.
 *
 * The title bar's control is a button that flips light and dark, because a
 * button cycling through three states is a state nobody can see the effect of.
 * "Follows macOS" is the default and is a real third answer, so this is the one
 * place it can be picked - which is what `theme.ts` has always said would
 * happen here.
 */
function AppearanceCard({
  appearance,
  onAppearance,
}: {
  appearance: Appearance;
  onAppearance: (next: Appearance) => void;
}) {
  return (
    <Card title="Appearance">
      <Row
        title="Theme"
        detail={
          appearance === "system"
            ? "following macOS, and changing with it at sunset"
            : `always ${appearance}, whatever macOS is doing`
        }
        action={
          <div role="radiogroup" aria-label="Theme" style={{ display: "flex", gap: 6 }}>
            {APPEARANCES.map(({ value, label }) => {
              const on = appearance === value;
              return (
                <button
                  key={value}
                  type="button"
                  role="radio"
                  aria-checked={on}
                  onClick={() => onAppearance(value)}
                  style={{
                    font: "600 12px var(--font-sans)",
                    padding: "6px 11px",
                    borderRadius: 8,
                    cursor: "pointer",
                    color: on ? "var(--accent)" : "var(--ink2)",
                    background: on ? "var(--accent-tint)" : "transparent",
                    border: `1px solid ${on ? "var(--accent-bd)" : "var(--border2)"}`,
                  }}
                >
                  {label}
                </button>
              );
            })}
          </div>
        }
      />
    </Card>
  );
}

/* ---------------------------------------------------------------------- relay */

/**
 * The paid tier, drawn and inert.
 *
 * Deliberate, per CLAUDE.md rule 10: the relay's server half is not in this
 * repo and must not be implemented here. It says what the tier is and does not
 * pretend there is anything to press - an Upgrade button going nowhere would be
 * a worse promise than none.
 */
function RelayUpsell() {
  return (
    <section
      style={{
        background: "linear-gradient(135deg, var(--accent-tint), var(--gold-tint))",
        border: "1px solid var(--accent-bd)",
        borderRadius: 12,
        padding: 18,
      }}
    >
      <div style={{ font: "700 14px var(--font-sans)", marginBottom: 3 }}>
        The relay · works away from home
      </div>
      <div style={{ font: "400 12px/1.5 var(--font-sans)", color: "var(--ink3)" }}>
        Stable public URLs, snapshot failover, multi-device sync and remote MCP. Not available yet —
        there is nothing to sign up to.
      </div>
    </section>
  );
}

/* ----------------------------------------------------------------- furniture */

function Card({
  title,
  badge,
  children,
}: {
  title: string;
  badge?: React.ReactNode;
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
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 14 }}>
        <h2 style={{ font: "700 14px var(--font-sans)", margin: 0 }}>{title}</h2>
        {badge}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>{children}</div>
    </section>
  );
}

function Row({
  title,
  detail,
  mono,
  action,
}: {
  title: string;
  detail: string;
  mono?: boolean;
  action?: React.ReactNode;
}) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ font: "500 13px var(--font-sans)" }}>{title}</div>
        <div
          style={{
            font: mono ? "400 11.5px var(--font-mono)" : "400 11.5px var(--font-sans)",
            color: "var(--muted)",
            marginTop: 2,
            // A workspace path is long and its tail is the part that
            // identifies it, so it wraps rather than being truncated.
            overflowWrap: "anywhere",
          }}
        >
          {detail}
        </div>
      </div>
      {action}
    </div>
  );
}

function Note({
  tone = "neutral",
  children,
}: {
  tone?: "neutral" | "accent" | "gold" | "danger";
  children: React.ReactNode;
}) {
  const colour = {
    neutral: "var(--ink3)",
    accent: "var(--accent)",
    gold: "var(--gold)",
    danger: "var(--danger)",
  }[tone];

  return (
    <p
      style={{
        font: "400 11.5px/1.5 var(--font-sans)",
        color: colour,
        margin: 0,
        textWrap: "pretty",
      }}
    >
      {children}
    </p>
  );
}

function Mono({ children }: { children: React.ReactNode }) {
  return <span style={{ font: "400 11px var(--font-mono)" }}>{children}</span>;
}

function Chip({
  tone,
  children,
}: {
  tone: "neutral" | "green" | "gold";
  children: React.ReactNode;
}) {
  const { fg, bg } = {
    neutral: { fg: "var(--muted)", bg: "var(--chip)" },
    green: { fg: "var(--green)", bg: "var(--green-tint)" },
    gold: { fg: "var(--gold)", bg: "var(--gold-tint)" },
  }[tone];

  return (
    <span
      style={{
        font: "600 10.5px var(--font-mono)",
        color: fg,
        background: bg,
        padding: "4px 9px",
        borderRadius: 6,
        whiteSpace: "nowrap",
        flex: "none",
      }}
    >
      {children}
    </span>
  );
}

function TextButton({
  onClick,
  disabled,
  children,
}: {
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      style={{
        font: "600 12px var(--font-sans)",
        color: disabled ? "var(--faint)" : "var(--accent)",
        background: "none",
        border: "none",
        padding: 0,
        cursor: disabled ? "not-allowed" : "pointer",
        flex: "none",
      }}
    >
      {children}
    </button>
  );
}

/** Exported for the Notifications surface, which draws four of them. */
export function Switch({
  label,
  on,
  disabled,
  onToggle,
}: {
  label: string;
  on: boolean;
  disabled?: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={onToggle}
      style={{
        width: 38,
        height: 22,
        flex: "none",
        border: "none",
        padding: 0,
        borderRadius: 22,
        background: on ? "var(--accent)" : "var(--idle)",
        position: "relative",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.6 : 1,
        transition: "background .15s",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          position: "absolute",
          top: 2,
          left: on ? 18 : 2,
          width: 18,
          height: 18,
          background: "#fff",
          borderRadius: "50%",
          transition: "left .15s",
        }}
      />
    </button>
  );
}
