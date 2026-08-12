import { Mark } from "./Mark";
import { StatusPill } from "./StatusPill";
import type { ServingState } from "./generated";

/**
 * Scaffold shell (checklist D0).
 *
 * Everything visible in this app is rendered from daemon socket state. The
 * socket does not exist yet, so `serving` is undefined and the app shows the
 * disconnected state - which is a real state worth designing, not a
 * placeholder. D3 replaces the prop with a live subscription.
 */
export function App({ serving }: { serving?: ServingState }) {
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
        {serving
          ? "Your workspace and everything you are serving appears here."
          : "Waiting for the daemon. Once it is up, your workspace and everything you are serving appears here."}
      </p>
      <StatusPill serving={serving} />
    </main>
  );
}
