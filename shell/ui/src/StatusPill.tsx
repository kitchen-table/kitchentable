import type { ServingState } from "./generated";

/**
 * The serving state, as one small pill.
 *
 * Exhaustive over `ServingState`: the union is generated from the Rust enum, so
 * adding a variant in kt-types breaks this switch at typecheck time rather than
 * silently rendering nothing. That is the whole point of the generated types.
 */
export function StatusPill({ serving }: { serving?: ServingState }) {
  const { label, colour } = describe(serving);

  return (
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
      <span style={{ width: 7, height: 7, borderRadius: "50%", background: colour }} />
      {label}
    </span>
  );
}

function describe(serving?: ServingState): { label: string; colour: string } {
  if (!serving) return { label: "not connected", colour: "var(--faint)" };

  switch (serving.state) {
    case "serving":
      return { label: "serving", colour: "var(--green)" };
    case "paused":
      return { label: "paused", colour: "var(--faint)" };
    case "degraded":
      return { label: degradedLabel(serving.reason), colour: "var(--gold)" };
  }
}

function degradedLabel(reason: Extract<ServingState, { state: "degraded" }>["reason"]): string {
  switch (reason) {
    case "local_network_denied":
      return "not on your network";
    case "port_fallback":
      return "on a fallback port";
    case "https_unavailable":
      return "not encrypted";
    case "mdns_unavailable":
      return "fallback URL only";
  }
}
