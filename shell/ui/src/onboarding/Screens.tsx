import { Mark } from "../Mark";
import type { App, SysStatus } from "../generated";
import type { Step } from "./steps";

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

function Workspace({ status }: { status?: SysStatus }) {
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
          marginBottom: 22,
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ font: "600 14px var(--font-sans)" }}>
            {status?.workspace ?? "~/KitchenTable"}
          </div>
          <div style={{ font: "400 12px var(--font-mono)", color: "var(--muted)" }}>
            watching · {status?.app_count ?? 0}{" "}
            {status?.app_count === 1 ? "app" : "apps"}
          </div>
        </div>
        <span
          style={{
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

      <Note tone="gold" title="Why this default?">
        We stay out of Desktop, Documents and Downloads, so macOS never has to
        ask you for extra folder access.
      </Note>
    </Step>
  );
}

function Network({ status }: { status?: SysStatus }) {
  const denied =
    status?.serving.state === "degraded" &&
    status.serving.reason === "local_network_denied";
  const noMdns =
    status?.serving.state === "degraded" &&
    status.serving.reason === "mdns_unavailable";

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
          Privacy &amp; Security › Local Network.
        </Note>
      )}
      {noMdns && !denied && (
        <Note tone="gold" title="Friendly .local names are not available.">
          Your apps are still reachable by IP address, and the QR codes always
          encode an address that works.
        </Note>
      )}
      {!denied && !noMdns && status && (
        <Note tone="green" title="Allowed">
          Your apps are reachable across your network.
        </Note>
      )}

      <p style={{ ...body, color: "var(--faint)", marginTop: 18 }}>
        macOS shows this once. A stray “Don’t Allow” is easy to recover from —
        you will never be stuck.
      </p>
    </Step>
  );
}

function FirstApp({ apps }: { apps: App[] }) {
  const first = apps[0];

  return (
    <Step eyebrow="Step 4 · Your first app" title="Your first app is live">
      {first ? (
        <>
          <p style={body}>
            <b>{first.name}</b> is being served from this Mac right now. Open it
            on your phone:
          </p>
          <div
            style={{
              background: "var(--paper)",
              border: "1px solid var(--border)",
              borderRadius: 14,
              padding: 22,
              marginBottom: 16,
            }}
          >
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
          <p style={{ ...body, color: "var(--faint)" }}>
            Run <code style={{ fontFamily: "var(--font-mono)" }}>kt url {first.slug}</code> in a
            terminal for a QR code to point a camera at.
          </p>
        </>
      ) : (
        <p style={body}>
          Drop a folder with an <code style={{ fontFamily: "var(--font-mono)" }}>index.html</code>{" "}
          into your workspace and it will appear here within seconds.
        </p>
      )}
    </Step>
  );
}

function Pair({ apps }: { apps: App[] }) {
  const first = apps[0];

  return (
    <Step eyebrow="Step 5 · Pair your phone" title="New devices always ask first">
      <p style={body}>
        When someone opens a shared app for the first time, you get to approve
        their device — like pairing Bluetooth. This is the trust mechanic behind
        every share you will ever make.
      </p>

      <div
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderRadius: 14,
          padding: 22,
          maxWidth: 380,
          marginBottom: 18,
        }}
      >
        <div style={{ font: "700 15px var(--font-sans)", marginBottom: 4 }}>
          A device wants to open {first?.name ?? "your app"}
        </div>
        <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)", marginBottom: 16 }}>
          fingerprint · f3:9a:21 · Safari · iOS
        </div>
        <div style={{ display: "flex", gap: 9 }}>
          <FakeButton primary>Approve &amp; pair</FakeButton>
          <FakeButton danger>Deny</FakeButton>
        </div>
      </div>

      <Note tone="blue" title="Nothing is shared until you say so">
        Every app starts Private. A folder becoming an app never makes it
        reachable by anyone else.
      </Note>
    </Step>
  );
}

function Ready() {
  return (
    <Step eyebrow="Step 6 · You’re all set" title="Drop a folder, make an app">
      <p style={body}>
        This is your library. Everything you build lands here, and every app you
        share stays yours to revoke.
      </p>
      <Note tone="blue" title="Closing the window does not stop serving">
        Kitchen Table keeps running in the menu bar — click its icon up top
        anytime.
      </Note>
    </Step>
  );
}

function Agent() {
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
          font: "500 12.5px var(--font-mono)",
          color: "#d8d3ca",
          marginBottom: 16,
        }}
      >
        claude mcp add kitchentable -- kt mcp
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
