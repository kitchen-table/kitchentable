import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Mark } from "../Mark";
import { call, restartDaemon } from "../daemon";
import {
  type Device,
  describeAgent,
  deviceShape,
  pending as pendingDevices,
  useDeviceActions,
  useDevices,
} from "../devices";
import type { App, SysStatus } from "../generated";
import { DeviceShapeIcon, FolderIcon } from "../icons";
import { useLaunchAtLogin } from "../launchAtLogin";
import { useLocalNetwork } from "../localnet";
import { useAskPermission, useNotifyState } from "../notify";
import { Qr } from "../Qr";
import { Switch } from "../surfaces/Settings";
import type { Step } from "./steps";

/**
 * The app onboarding should point a camera at.
 *
 * Not simply the first one. A workspace can contain folders that are not apps
 * in any useful sense - the daemon registers any directory, and one without an
 * `index.html` serves a 404 - so the first folder alphabetically is a QR code
 * pointing at nothing, which is exactly what onboarding did on a machine whose
 * workspace was a code directory.
 *
 * The sample app first, because it is the one the flow is written around and
 * the one whose visibility invites the pairing step. Then anything that can
 * actually be served. Only then the first of whatever is left, which at least
 * names something real.
 */
export function showcase(apps: App[]): App | undefined {
  return (
    apps.find((app) => app.slug === "welcome" && app.entry_exists) ??
    apps.find((app) => app.entry_exists) ??
    apps[0]
  );
}

/** The body of each step. */
export function Screens({
  step,
  apps,
  status,
  onNext,
}: {
  step: Step;
  apps: App[];
  status?: SysStatus;
  onNext: () => void;
}) {
  switch (step) {
    case "welcome":
      return <Welcome onNext={onNext} />;
    case "workspace":
      return <Workspace status={status} />;
    case "network":
      return <Network status={status} />;
    case "first-app":
      return <FirstApp apps={apps} />;
    case "notifications":
      return <Notifications apps={apps} />;
    case "pair":
      return <Pair apps={apps} />;
    case "ready":
      return <Ready />;
    case "agent":
      return <Agent />;
    case "relay":
      return <Relay />;
  }
}

function Welcome({ onNext }: { onNext: () => void }) {
  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        textAlign: "center",
      }}
    >
      <Mark size={56} />
      <h1
        style={{
          margin: "24px 0 14px",
          font: "800 40px/1.05 var(--font-sans)",
          letterSpacing: "-0.03em",
        }}
      >
        Welcome to Kitchen Table
      </h1>
      <p
        style={{
          margin: "0 0 32px",
          maxWidth: 460,
          font: "400 17px/1.5 var(--font-sans)",
          color: "var(--ink2)",
        }}
      >
        Kitchen Table turns folders on this Mac into apps you can share with the
        people you choose.
      </p>
      <button
        type="button"
        onClick={onNext}
        style={{
          border: "none",
          borderRadius: 11,
          padding: "14px 30px",
          background: "var(--accent)",
          color: "#fff",
          font: "600 15px var(--font-sans)",
          cursor: "pointer",
        }}
      >
        Get started
      </button>
      <p style={{ marginTop: 18, font: "400 12.5px var(--font-sans)", color: "var(--faint)" }}>
        Free · open source · your files stay yours
      </p>
    </div>
  );
}

/**
 * Step 2, from `uimockups/Onboarding - macOS.dc.html`.
 *
 * The mockup offers a way out of the default and this screen did not, which
 * made "Choose your workspace" a heading over a folder you could not choose.
 * The picker is the same one Settings uses, and it goes through the same socket
 * method, because there is one place that decides whether a folder is a
 * workspace and it is not the window.
 *
 * Picking here restarts the daemon, which Settings deliberately does not do.
 * The difference is what happens next: onboarding walks straight on to a step
 * that shows the apps in the workspace, so a choice that only takes effect
 * "next time" would leave somebody looking at the old folder's contents under
 * the new folder's name. The daemon is seconds old and serving nothing, so
 * restarting it costs nothing here.
 */
function Workspace({ status }: { status?: SysStatus }) {
  const client = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const locked = status?.workspace_locked ?? false;

  const choose = useMutation({
    mutationFn: async () => {
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked !== "string") return false; // cancelled

      await call("sys.set_workspace", { path: picked });
      // Stored, not applied: the daemon reads this when it starts. So start it
      // again, and only then tell the window to re-read its status.
      await restartDaemon();
      return true;
    },
    onSuccess: (changed) => {
      setError(null);
      if (!changed) return;
      void client.invalidateQueries({ queryKey: ["status"] });
      void client.invalidateQueries({ queryKey: ["apps"] });
    },
    onError: (raw) =>
      setError(
        typeof raw === "object" && raw !== null && "message" in raw
          ? String((raw as { message: unknown }).message)
          : String(raw),
      ),
  });

  return (
    <Step eyebrow="Step 2 · Workspace" title="Choose your workspace">
      <p style={body}>
        This is the one folder Kitchen Table watches. Drop any folder inside it
        and it becomes an app — automatically, within seconds.
      </p>

      <div
        style={{
          background: "var(--paper)",
          border: "1.5px solid var(--accent)",
          borderRadius: 12,
          padding: "16px 18px",
          display: "flex",
          alignItems: "center",
          gap: 14,
          marginBottom: 12,
        }}
      >
        <span
          aria-hidden="true"
          style={{
            width: 40,
            height: 34,
            flex: "none",
            borderRadius: 7,
            background: "var(--accent-tint)",
            color: "var(--accent)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <FolderIcon size={20} />
        </span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ font: "600 14px var(--font-sans)" }}>
            {choose.isPending ? "Switching…" : (status?.workspace ?? "~/KitchenTable")}
          </div>
          <div style={{ font: "400 12px var(--font-mono)", color: "var(--muted)" }}>
            watching · {status?.app_count ?? 0}{" "}
            {status?.app_count === 1 ? "app" : "apps"}
          </div>
        </div>
        <span
          style={{
            flex: "none",
            font: "600 11px var(--font-mono)",
            color: "var(--green)",
            background: "var(--green-tint)",
            padding: "4px 9px",
            borderRadius: 6,
          }}
        >
          Ready
        </span>
      </div>

      {/* The mockup's way out of the default. Absent while `KT_WORKSPACE` is
          set, because there the environment outranks anything picked here and
          the button would write a value nothing will ever read. */}
      {locked ? (
        <p style={{ ...body, marginBottom: 22, color: "var(--muted)" }}>
          This folder is fixed by <code>KT_WORKSPACE</code> for this run, so it
          cannot be changed from here.
        </p>
      ) : (
        <button
          type="button"
          disabled={choose.isPending}
          onClick={() => choose.mutate()}
          style={{
            display: "block",
            border: "none",
            background: "none",
            padding: 0,
            marginBottom: 22,
            font: "500 13px var(--font-sans)",
            color: "var(--accent)",
            cursor: choose.isPending ? "not-allowed" : "pointer",
          }}
        >
          Choose a different folder…
        </button>
      )}

      {error && (
        <div style={{ marginBottom: 18 }}>
          <Note tone="gold" title="That folder cannot be the workspace.">
            {error}
          </Note>
        </div>
      )}

      <Note tone="gold" title="Why this default?">
        We stay out of Desktop, Documents and Downloads, so macOS never has to
        ask you for extra folder access.
      </Note>
    </Step>
  );
}

/**
 * Step 3, from `uimockups/Onboarding - macOS.dc.html`.
 *
 * The mockup draws three states and this screen has exactly those: waiting to
 * be asked, with a **Show me the prompt** button; allowed; and denied, with the
 * route back through System Settings and a **try again**.
 *
 * The button is the substance of the screen. macOS has no call that raises the
 * Local Network prompt - the prompt is what happens when an app first touches
 * the network - so this presses on it deliberately, at the moment onboarding
 * has just explained what the dialog is for. Before, this screen drew the
 * dialog, asked nothing, and then said "Allowed" because the daemon had not
 * reported a fault, which is a claim nobody had checked.
 *
 * Nothing here treats silence as a refusal. See `localnet.rs`: an empty network
 * and an unreachable one look identical from a socket, and the difference
 * between them is somebody's phone working or not.
 */
function Network({ status }: { status?: SysStatus }) {
  const network = useLocalNetwork();
  const state = network.data?.state;

  // The daemon's own view, which is about names rather than reachability: it
  // can announce nothing while the network is perfectly reachable.
  const noMdns =
    status?.serving.state === "degraded" &&
    status.serving.reason === "mdns_unavailable";

  const denied =
    state === "denied" ||
    (status?.serving.state === "degraded" &&
      status.serving.reason === "local_network_denied");

  return (
    <Step eyebrow="Step 3 · Network" title="Let your phone see your apps">
      <p style={body}>
        Your Mac will ask whether Kitchen Table can talk to devices on your
        network. That is exactly how your phone finds your apps. Here is the
        prompt you will see:
      </p>

      {/* The pre-frame the whole flow exists for: macOS shows this once, and a
          reflexive Deny would silently kill the product. */}
      <div
        style={{
          background: "var(--raised)",
          borderRadius: 14,
          padding: 22,
          display: "flex",
          justifyContent: "center",
          marginBottom: 20,
        }}
      >
        <div
          style={{
            width: 270,
            background: "var(--paper)",
            border: "1px solid var(--border)",
            borderRadius: 13,
            padding: 18,
            textAlign: "center",
          }}
        >
          <div
            style={{
              width: 44,
              height: 44,
              margin: "0 auto 10px",
              borderRadius: 10,
              background: "var(--accent)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Mark size={26} />
          </div>
          <p style={{ margin: "0 0 14px", font: "600 12.5px/1.4 var(--font-sans)" }}>
            “Kitchen Table” would like to find and connect to devices on your
            local network.
          </p>
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            <FakeButton>Don’t Allow</FakeButton>
            <FakeButton primary>Allow</FakeButton>
          </div>
        </div>
      </div>

      {denied && (
        <Note tone="gold" title="Devices on your network can’t see your apps yet.">
          Everything still works on this Mac. Turn it on in System Settings ›
          Privacy &amp; Security › Local Network — or{" "}
          <Again onClick={() => network.refetch()} busy={network.isFetching}>
            try again
          </Again>
          .
        </Note>
      )}

      {state === "allowed" && !denied && (
        <Note tone="green" title="Allowed">
          Your apps are now reachable across your network.
        </Note>
      )}

      {noMdns && !denied && (
        <Note tone="gold" title="Friendly .local names are not available.">
          Your apps are still reachable by IP address, and the QR codes always
          encode an address that works.
        </Note>
      )}

      <div style={{ display: "flex", alignItems: "center", gap: 14, marginTop: 18 }}>
        {/* Offered until something answers. "Unknown" keeps it: a network that
            told us nothing is a network we have not been let onto as far as
            anybody can prove, and pressing again costs nothing. */}
        {state !== "allowed" && !denied && (
          <button
            type="button"
            disabled={network.isFetching}
            onClick={() => network.refetch()}
            style={{
              flex: "none",
              border: "none",
              background: "var(--accent)",
              color: "#fff",
              padding: "11px 22px",
              borderRadius: 10,
              font: "600 14px var(--font-sans)",
              cursor: network.isFetching ? "not-allowed" : "pointer",
              opacity: network.isFetching ? 0.7 : 1,
            }}
          >
            {network.isFetching ? "Asking your Mac…" : "Show me the prompt"}
          </button>
        )}
        <span style={{ font: "400 12px var(--font-sans)", color: "var(--faint)" }}>
          macOS shows this once. A stray “Don’t Allow” is easy to recover from —
          you will never be stuck.
        </span>
      </div>
    </Step>
  );
}

/** The "try again" inside the denied note. A link in a sentence, as drawn. */
function Again({
  children,
  onClick,
  busy,
}: {
  children: React.ReactNode;
  onClick: () => void;
  busy: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      style={{
        border: "none",
        background: "none",
        padding: 0,
        font: "inherit",
        color: "var(--accent)",
        cursor: busy ? "not-allowed" : "pointer",
      }}
    >
      {children}
    </button>
  );
}

/**
 * Step 4, from `uimockups/Onboarding - macOS.dc.html`.
 *
 * The mockup puts a QR code beside the app's address, and it is the point of
 * the step: onboarding.md's two-minute promise ends with a phone camera pointed
 * at this screen. It used to say "run `kt url` in a terminal for a QR code to
 * point a camera at", which is the one instruction this flow exists to avoid.
 *
 * The mockup's HTTPS row is absent, because HTTPS is: the local CA and leaf
 * certificates are not built (checklist D7), so a row offering to turn on
 * encryption would be a button with nothing behind it. The address shown is the
 * one that actually works.
 */
function FirstApp({ apps }: { apps: App[] }) {
  const first = showcase(apps);

  if (!first) {
    return (
      <Step eyebrow="Step 4 · Your first app" title="Your first app is live">
        <p style={body}>
          Drop a folder with an <code style={{ fontFamily: "var(--font-mono)" }}>index.html</code>{" "}
          into your workspace and it will appear here within seconds.
        </p>
        <Note tone="blue" title="Nothing to open yet">
          This step fills itself in the moment the workspace has an app in it —
          you can leave the window open and drag one in now.
        </Note>
      </Step>
    );
  }

  return (
    <Step eyebrow="Step 4 · Your first app" title="Your first app is live">
      <p style={body}>
        <b>{first.name}</b> is being served from this Mac right now. Point your
        phone&rsquo;s camera at this:
      </p>

      <div
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderRadius: 14,
          padding: 22,
          marginBottom: 16,
          display: "flex",
          alignItems: "center",
          gap: 22,
        }}
      >
        <Qr value={first.url} size={132} title={`QR code for ${first.url}`} />
        <div style={{ minWidth: 0 }}>
          <div style={{ font: "700 18px var(--font-sans)", marginBottom: 4 }}>
            {first.name}
          </div>
          <a
            href={first.url}
            target="_blank"
            rel="noreferrer"
            style={{ font: "400 13px var(--font-mono)", color: "var(--accent)" }}
          >
            {first.url}
          </a>
          <div
            style={{
              marginTop: 12,
              font: "400 11.5px var(--font-mono)",
              color: "var(--faint)",
            }}
          >
            or {first.fallback_url} if .local does not resolve
          </div>
        </div>
      </div>

      <p style={{ ...body, color: "var(--faint)", marginBottom: 0 }}>
        Both addresses only work on your own network. Nothing here is reachable
        from the internet.
      </p>
    </Step>
  );
}

/**
 * Step 5, from `uimockups/Onboarding - macOS.dc.html`.
 *
 * Three states, as the mockup draws them: not asked, with the **Enable
 * notifications** button; on; and refused, with the route back and a **try
 * again**. The banner above them is a drawing of the real thing, which is the
 * point of asking - that alert is how an access request gets answered when the
 * window is closed, which is nearly always.
 *
 * macOS asks once per app, ever. So the button disappears once there is an
 * answer either way, and the refused state points at System Settings rather
 * than offering a second prompt that would never appear. `try again` re-reads
 * the state, for somebody who has just flipped the switch over there.
 *
 * The two states this screen cannot reach are the honest ones: a development
 * build has no bundle for macOS to attach a permission to, and every platform
 * that is not macOS has none of this yet. Both say so instead of offering a
 * button that does nothing.
 */
function Notifications({ apps }: { apps: App[] }) {
  const permission = useNotifyState();
  const ask = useAskPermission();
  const state = permission.data?.state;
  const app = showcase(apps);

  return (
    <Step eyebrow="Step 5 · Notifications" title="Know when someone knocks">
      <p style={body}>
        The moment a new device asks to open one of your apps, Kitchen Table
        pings you — that alert is how you approve or deny. Your Mac will ask
        permission to notify you.
      </p>

      <div
        style={{
          background: "var(--raised)",
          borderRadius: 14,
          padding: 22,
          display: "flex",
          justifyContent: "center",
          marginBottom: 20,
        }}
      >
        <div
          style={{
            width: 300,
            background: "var(--paper)",
            border: "1px solid var(--border)",
            borderRadius: 12,
            padding: "12px 14px",
            display: "flex",
            gap: 12,
            alignItems: "flex-start",
          }}
        >
          <span
            aria-hidden="true"
            style={{
              width: 34,
              height: 34,
              flex: "none",
              borderRadius: 9,
              background: "var(--accent)",
              color: "#fff",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <DeviceShapeIcon shape="phone" size={18} />
          </span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ font: "700 12px var(--font-sans)" }}>Kitchen Table</div>
            <div style={{ font: "400 11.5px/1.4 var(--font-sans)", color: "var(--ink2)" }}>
              A new device wants to open {app ? app.name : "one of your apps"}
            </div>
          </div>
          <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
            <span
              style={{
                font: "600 10px var(--font-sans)",
                color: "var(--accent)",
                background: "var(--accent-tint)",
                padding: "3px 8px",
                borderRadius: 5,
              }}
            >
              Approve
            </span>
            <span
              style={{
                font: "600 10px var(--font-sans)",
                color: "var(--muted)",
                background: "var(--chip)",
                padding: "3px 8px",
                borderRadius: 5,
              }}
            >
              Deny
            </span>
          </div>
        </div>
      </div>

      {state === "granted" && (
        <Note tone="green" title="Notifications on">
          You will be pinged the moment a device knocks.
        </Note>
      )}

      {state === "denied" && (
        <Note tone="gold" title="You won’t get pop-ups for access requests.">
          Nothing breaks — requests still appear as a badge on the Kitchen Table
          window and in the menu bar. Turn them on in System Settings ›
          Notifications, or{" "}
          <Again onClick={() => permission.refetch()} busy={permission.isFetching}>
            try again
          </Again>
          .
        </Note>
      )}

      {state === "unbundled" && (
        <Note tone="gold" title="Not in a development build">
          macOS attaches this permission to an installed app, and there is no
          bundle to attach it to here. It works in the built app.
        </Note>
      )}

      {state === "unsupported" && (
        <Note tone="gold" title="macOS only, for now">
          Desktop notifications are not built for this platform yet.
        </Note>
      )}

      <div style={{ display: "flex", alignItems: "center", gap: 14, marginTop: 18 }}>
        {state === "not_determined" && (
          <button
            type="button"
            disabled={ask.isPending}
            onClick={() => ask.mutate()}
            style={{
              flex: "none",
              border: "none",
              background: "var(--accent)",
              color: "#fff",
              padding: "11px 22px",
              borderRadius: 10,
              font: "600 14px var(--font-sans)",
              cursor: ask.isPending ? "not-allowed" : "pointer",
              opacity: ask.isPending ? 0.7 : 1,
            }}
          >
            {ask.isPending ? "Asking your Mac…" : "Enable notifications"}
          </button>
        )}
        <span style={{ font: "400 12px var(--font-sans)", color: "var(--faint)" }}>
          Recommended — the fastest way to catch a pairing request.
        </span>
      </div>
    </Step>
  );
}

/**
 * Step 6, from `uimockups/Onboarding - macOS.dc.html`.
 *
 * Three states, as the mockup has them: nothing has asked yet, a device is
 * asking, and it is done.
 *
 * The third one is the point of the step and was missing, which made approving
 * look like a failure: the request card vanished the instant you answered it -
 * correctly, there was nothing pending any more - and the screen fell back to
 * "waiting for a device", as though the phone you had just let in had never
 * arrived. The daemon had it right all along; the screen simply never said so.
 *
 * "Paired" is read from the device list rather than remembered from the button
 * press, so it survives going back a step and returning, and it is true even if
 * the approval happened from the banner or another window.
 */
function Pair({ apps }: { apps: App[] }) {
  const first = showcase(apps);
  const devices = useDevices();
  const waiting = pendingDevices(devices.data);
  const asking = waiting[0];

  // The owner's own machine is approved by definition and is not the lesson.
  const paired = (devices.data ?? [])
    .filter((device) => device.status === "approved")
    .sort((a, b) => b.last_seen - a.last_seen)[0];

  if (paired && !asking) {
    return (
      <Step eyebrow="Step 6 · Pair your phone" title="Paired! 🎉">
        <p style={body}>
          <b>{paired.name}</b> is now showing {first ? first.name : "your app"}. That is the
          whole loop — build, serve, share.
        </p>

        <div
          style={{
            background: "var(--paper)",
            border: "1px solid var(--border)",
            borderRadius: 14,
            padding: "16px 18px",
            display: "flex",
            alignItems: "center",
            gap: 14,
            maxWidth: 420,
            marginBottom: 18,
          }}
        >
          <span
            aria-hidden="true"
            style={{
              width: 38,
              height: 38,
              flex: "none",
              borderRadius: 10,
              background: "var(--green-tint)",
              color: "var(--green)",
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <DeviceShapeIcon shape={deviceShape(paired.user_agent)} size={19} />
          </span>
          <div style={{ minWidth: 0 }}>
            <div style={{ font: "600 13.5px var(--font-sans)" }}>{paired.name}</div>
            <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)" }}>
              {paired.fingerprint} · {describeAgent(paired.user_agent)} · approved
            </div>
          </div>
        </div>

        <Note tone="blue" title="It stays yours to revoke">
          Approved devices keep the name you gave them and appear in the Devices tab of
          every app. Rename or revoke them whenever you like.
        </Note>
      </Step>
    );
  }

  return (
    <Step
      eyebrow="Step 6 · Pair your phone"
      title={asking ? "Approve your phone" : "New devices always ask first"}
    >
      <p style={body}>
        {asking
          ? "Your phone just opened the link. New devices always ask first — like pairing Bluetooth. This is the trust mechanic behind every share you will ever make."
          : "When someone opens a shared app for the first time, you get to approve their device — like pairing Bluetooth. This is the trust mechanic behind every share you will ever make."}
      </p>

      {asking ? (
        <Waiting device={asking} />
      ) : (
        <div
          style={{
            background: "var(--paper)",
            border: "1px dashed var(--dash)",
            borderRadius: 14,
            padding: 22,
            maxWidth: 380,
            marginBottom: 18,
          }}
        >
          <div style={{ font: "600 14px var(--font-sans)", marginBottom: 4 }}>
            Waiting for a device
          </div>
          <div style={{ font: "400 12px/1.5 var(--font-sans)", color: "var(--muted)" }}>
            Open {first ? <b>{first.name}</b> : "one of your apps"} on your phone
            and its request will appear here.
          </div>
        </div>
      )}

      <Note tone="blue" title="Nothing is shared until you say so">
        Every app starts Private. A folder becoming an app never makes it
        reachable by anyone else.
      </Note>
    </Step>
  );
}

/** A real device at the door, named and answered without leaving the flow. */
function Waiting({ device }: { device: Device }) {
  const [name, setName] = useState(device.name);
  const { approve, deny } = useDeviceActions();
  const busy = approve.isPending || deny.isPending;

  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--gold-bd)",
        borderLeft: "3px solid var(--gold)",
        borderRadius: 14,
        padding: 22,
        maxWidth: 380,
        marginBottom: 18,
      }}
    >
      <div style={{ font: "700 15px var(--font-sans)", marginBottom: 4 }}>
        {device.name} wants to open your app
      </div>
      <div
        style={{
          font: "400 11px var(--font-mono)",
          color: "var(--muted)",
          marginBottom: 16,
        }}
      >
        fingerprint · {device.fingerprint} · {describeAgent(device.user_agent)}
      </div>

      <label
        style={{
          display: "block",
          font: "600 10px var(--font-mono)",
          color: "var(--muted)",
          letterSpacing: "0.05em",
          marginBottom: 5,
        }}
      >
        NAME THIS DEVICE
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          disabled={busy}
          style={{
            display: "block",
            width: "100%",
            marginTop: 5,
            border: "1px solid var(--border2)",
            borderRadius: 9,
            padding: "9px 11px",
            background: "var(--paper)",
            color: "var(--ink)",
            font: "400 13px var(--font-sans)",
            outline: "none",
          }}
        />
      </label>

      <div style={{ display: "flex", gap: 9, marginTop: 14 }}>
        <button
          type="button"
          disabled={busy}
          onClick={() => approve.mutate({ id: device.id, name: name.trim() || undefined })}
          style={{
            flex: 1,
            border: "none",
            borderRadius: 9,
            padding: "10px 12px",
            background: "var(--accent)",
            color: "#fff",
            font: "600 12.5px var(--font-sans)",
            cursor: busy ? "not-allowed" : "pointer",
          }}
        >
          Approve &amp; pair
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => deny.mutate(device.id)}
          style={{
            flex: "none",
            border: "1px solid var(--border2)",
            borderRadius: 9,
            padding: "10px 14px",
            background: "transparent",
            color: "var(--danger)",
            font: "600 12.5px var(--font-sans)",
            cursor: busy ? "not-allowed" : "pointer",
          }}
        >
          Deny
        </button>
      </div>
    </div>
  );
}

/**
 * Step 6, from `uimockups/Onboarding - macOS.dc.html`.
 *
 * Onboarding.md section 2 puts the launch-at-login choice on this step, and it
 * belongs here rather than only in Settings: the sentence above it - that
 * closing the window does not stop serving - is a promise the product cannot
 * keep across a restart unless this is on. The toggle was the missing half of
 * the paragraph.
 *
 * It reads its own position back from the OS rather than remembering what it
 * asked for, and says so when it cannot; see `launchAtLogin.ts`.
 */
function Ready() {
  const launch = useLaunchAtLogin();

  return (
    <Step eyebrow="Step 7 · You’re all set" title="Drop a folder, make an app">
      <p style={body}>
        This is your library. Everything you build lands here, and every app you
        share stays yours to revoke.
      </p>

      <div
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderRadius: 12,
          padding: "14px 18px",
          display: "flex",
          alignItems: "center",
          gap: 14,
          maxWidth: 540,
          marginBottom: 18,
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ font: "600 13.5px var(--font-sans)" }}>
            Start Kitchen Table when I log in
          </div>
          <div style={{ font: "400 11.5px/1.45 var(--font-mono)", color: "var(--muted)" }}>
            {launch.error
              ? launch.error
              : !launch.available
                ? "only available in the installed app"
                : "so your apps are there after a restart"}
          </div>
        </div>
        <Switch
          label="Start Kitchen Table when I log in"
          on={launch.on ?? false}
          disabled={!launch.available || launch.on === null || launch.busy}
          onToggle={launch.toggle}
        />
      </div>

      <Note tone="blue" title="Closing the window does not stop serving">
        Kitchen Table keeps running in the menu bar — click its icon up top
        anytime.
      </Note>
    </Step>
  );
}

/**
 * Optional step, from the mockup.
 *
 * The command is copyable because it is a command: the mockup draws it in a
 * terminal block, and a block of monospace you have to retype by hand is the
 * kind of small friction this flow is meant to remove.
 */
function Agent() {
  const [copied, setCopied] = useState(false);
  const command = "claude mcp add kitchentable -- kt mcp";

  return (
    <Step eyebrow="Optional · Connect your agent" title="Let your agent do it for you">
      <p style={body}>
        Kitchen Table speaks MCP, so Claude Code and other agents can build,
        deploy, and set sharing end to end.
      </p>
      <div
        style={{
          background: "var(--term)",
          borderRadius: 10,
          padding: "14px 16px",
          display: "flex",
          alignItems: "center",
          gap: 14,
          marginBottom: 16,
        }}
      >
        <code style={{ flex: 1, minWidth: 0, font: "500 12.5px var(--font-mono)", color: "#d8d3ca" }}>
          {command}
        </code>
        <button
          type="button"
          onClick={() => {
            void navigator.clipboard
              ?.writeText(command)
              .then(() => setCopied(true))
              // A clipboard that refuses is not worth an error dialog; the
              // command is on screen and can be selected.
              .catch(() => setCopied(false));
          }}
          style={{
            flex: "none",
            border: "1px solid rgba(255,255,255,.18)",
            borderRadius: 8,
            padding: "6px 11px",
            background: "transparent",
            color: "#d8d3ca",
            font: "600 11.5px var(--font-sans)",
            cursor: "pointer",
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <p style={{ ...body, color: "var(--faint)" }}>
        The MCP server itself lands in a later release; the command above is
        what you will run when it does.
      </p>
    </Step>
  );
}

function Relay() {
  return (
    <Step eyebrow="Optional · Away from home" title="Take your apps anywhere">
      <p style={body}>
        The relay gives your apps a link that works when you are out, and keeps
        a snapshot alive while this Mac sleeps. You choose per app whether a
        copy may live on our servers, or whether the link simply stops when your
        machine does.
      </p>
      <Note tone="blue" title="This never gates anything local">
        Everything you have set up so far works without it, forever.
      </Note>
    </Step>
  );
}

// ---- shared bits ----------------------------------------------------------

const body: React.CSSProperties = {
  margin: "0 0 22px",
  maxWidth: 540,
  font: "400 15.5px/1.55 var(--font-sans)",
  color: "var(--ink2)",
};

function Step({
  eyebrow,
  title,
  children,
}: {
  eyebrow: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div style={{ maxWidth: 560 }}>
      <div
        style={{
          font: "600 12px var(--font-mono)",
          color: "var(--accent)",
          letterSpacing: "0.05em",
          textTransform: "uppercase",
          marginBottom: 12,
        }}
      >
        {eyebrow}
      </div>
      <h1
        style={{
          margin: "0 0 12px",
          font: "800 30px/1.1 var(--font-sans)",
          letterSpacing: "-0.02em",
        }}
      >
        {title}
      </h1>
      {children}
    </div>
  );
}

function Note({
  tone,
  title,
  children,
}: {
  tone: "gold" | "green" | "blue";
  title: string;
  children: React.ReactNode;
}) {
  const colours = {
    gold: { fg: "var(--gold)", bg: "var(--gold-tint)" },
    green: { fg: "var(--green)", bg: "var(--green-tint)" },
    blue: { fg: "var(--accent)", bg: "var(--accent-tint)" },
  }[tone];

  return (
    <div
      style={{
        background: colours.bg,
        borderRadius: 11,
        padding: "14px 16px",
        maxWidth: 540,
      }}
    >
      <div style={{ font: "700 12.5px var(--font-sans)", color: colours.fg, marginBottom: 3 }}>
        {title}
      </div>
      <div style={{ font: "400 12.5px/1.5 var(--font-sans)", color: "var(--ink3)" }}>
        {children}
      </div>
    </div>
  );
}

/** A button-shaped thing in an illustration. Not interactive on purpose. */
function FakeButton({
  children,
  primary,
  danger,
}: {
  children: React.ReactNode;
  primary?: boolean;
  danger?: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      style={{
        flex: 1,
        textAlign: "center",
        padding: "9px 12px",
        borderRadius: 9,
        font: "600 12.5px var(--font-sans)",
        background: primary ? "var(--accent)" : "var(--chip)",
        color: primary ? "#fff" : danger ? "var(--danger)" : "var(--ink3)",
      }}
    >
      {children}
    </span>
  );
}
