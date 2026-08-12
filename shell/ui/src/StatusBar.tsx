import type { SysStatus } from "./generated";

/**
 * The serving state, along the bottom of the window.
 *
 * Degraded is the interesting case: onboarding.md insists every denial has a
 * visible recovery path, so each reason gets a sentence someone can act on
 * rather than a generic warning.
 */
export function StatusBar({ status }: { status?: SysStatus }) {
  if (!status) return null;

  const { colour, label, detail } = describe(status);

  return (
    <footer
      style={{
        flex: "none",
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "9px 16px",
        borderTop: "1px solid var(--border)",
        background: "var(--raised)",
        font: "500 11.5px var(--font-mono)",
        color: "var(--muted)",
      }}
    >
      <span
        aria-hidden="true"
        style={{ width: 7, height: 7, borderRadius: "50%", background: colour, flex: "none" }}
      />
      <span style={{ color: "var(--ink2)" }}>{label}</span>
      {detail && (
        <span
          style={{
            minWidth: 0,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {detail}
        </span>
      )}
      <span style={{ marginLeft: "auto", color: "var(--faint)" }}>
        v{status.daemon_version}
      </span>
    </footer>
  );
}

function describe(status: SysStatus): {
  colour: string;
  label: string;
  detail?: string;
} {
  const serving = status.serving;

  switch (serving.state) {
    case "serving":
      return { colour: "var(--green)", label: "Serving" };
    case "paused":
      return {
        colour: "var(--faint)",
        label: "Paused",
        detail: "nothing is reachable until you resume",
      };
    case "degraded":
      return {
        colour: "var(--gold)",
        label: headline(serving.reason),
        detail: serving.message,
      };
  }
}

function headline(reason: Extract<SysStatus["serving"], { state: "degraded" }>["reason"]): string {
  switch (reason) {
    case "local_network_denied":
      return "Not visible on your network";
    case "port_fallback":
      return "Serving on a different port";
    case "https_unavailable":
      return "Not encrypted";
    case "mdns_unavailable":
      return "No .local names";
  }
}
