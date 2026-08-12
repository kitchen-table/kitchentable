import { useState } from "react";
import { Mark } from "./Mark";
import { restartDaemon, type Health } from "./daemon";

/**
 * What the window shows when there is no daemon to render from.
 *
 * Each cause gets its own sentence, because "something went wrong" leaves
 * someone with nothing to do. Every state here has a recovery path, which is
 * the rule onboarding.md sets for degraded states generally.
 */
export function Disconnected({
  health,
  onRetry,
}: {
  health?: Health;
  onRetry: () => void;
}) {
  const [restarting, setRestarting] = useState(false);
  const { title, detail } = describe(health);

  async function restart() {
    setRestarting(true);
    try {
      await restartDaemon();
    } catch {
      // The health poll below reports the outcome either way; a failed restart
      // just leaves this screen up.
    } finally {
      setRestarting(false);
      onRetry();
    }
  }

  return (
    <main
      style={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 16,
        padding: 40,
        textAlign: "center",
      }}
    >
      <Mark size={44} />
      <h1
        style={{
          margin: 0,
          font: "800 22px/1.2 var(--font-sans)",
          letterSpacing: "-0.02em",
        }}
      >
        {title}
      </h1>
      <p
        style={{
          margin: 0,
          maxWidth: 380,
          font: "400 14px/1.6 var(--font-sans)",
          color: "var(--ink3)",
        }}
      >
        {detail}
      </p>
      <button
        type="button"
        onClick={restart}
        disabled={restarting}
        style={{
          marginTop: 4,
          padding: "10px 20px",
          borderRadius: 10,
          border: "none",
          background: "var(--accent)",
          color: "#fff",
          font: "600 13.5px var(--font-sans)",
          cursor: restarting ? "default" : "pointer",
          opacity: restarting ? 0.6 : 1,
        }}
      >
        {restarting ? "Starting…" : "Start it again"}
      </button>
      <p
        style={{
          margin: 0,
          font: "400 11.5px var(--font-mono)",
          color: "var(--faint)",
        }}
      >
        your apps and files are untouched
      </p>
    </main>
  );
}

function describe(health?: Health): { title: string; detail: string } {
  switch (health?.state) {
    case "wrong_version":
      return {
        title: "Kitchen Table was updated",
        detail:
          "The background service is still running the old version. Restarting it takes a second and nothing is lost.",
      };
    case "unhealthy":
      return {
        title: "The background service stopped responding",
        detail: health.reason,
      };
    case "not_running":
    default:
      return {
        title: "Nothing is being served right now",
        detail:
          "The background service that serves your apps is not running, so links to them will not open.",
      };
  }
}
