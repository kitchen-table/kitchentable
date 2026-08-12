import { Mark } from "./Mark";

/**
 * Scaffold shell (checklist D0).
 *
 * Everything visible in this app is rendered from daemon socket state. The
 * socket does not exist yet, so the honest thing to show is the state the app
 * will genuinely be in whenever the daemon is down - which is a real state
 * worth designing, not a placeholder.
 */
export function App() {
  return (
    <main
      style={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 18,
        padding: 40,
        textAlign: "center",
      }}
    >
      <Mark size={44} />
      <h1 style={{ margin: 0, font: "800 26px/1.1 var(--font-sans)", letterSpacing: "-0.02em" }}>
        Kitchen Table
      </h1>
      <p
        style={{
          margin: 0,
          maxWidth: 340,
          font: "400 14px/1.6 var(--font-sans)",
          color: "var(--ink3)",
        }}
      >
        Waiting for the daemon. Once it is up, your workspace and everything you
        are serving appears here.
      </p>
      <span
        style={{
          display: "inline-flex",
          alignItems: "center",
          gap: 8,
          padding: "6px 12px",
          borderRadius: 999,
          background: "var(--chip)",
          font: "500 11px var(--font-mono)",
          color: "var(--muted)",
        }}
      >
        <span
          style={{
            width: 7,
            height: 7,
            borderRadius: "50%",
            background: "var(--faint)",
          }}
        />
        not connected
      </span>
    </main>
  );
}
