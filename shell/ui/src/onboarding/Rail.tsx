import { Mark } from "../Mark";
import { LABELS, OPTIONAL, REQUIRED, stepIndex, type Step } from "./steps";

/** The step list down the left, showing where you are and what is left. */
export function Rail({
  current,
  onJump,
}: {
  current: Step;
  onJump: (step: Step) => void;
}) {
  return (
    <nav
      aria-label="Setup steps"
      style={{
        width: 236,
        flex: "none",
        background: "var(--raised)",
        borderRight: "1px solid var(--border)",
        padding: "22px 16px",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 9, padding: "0 8px 20px" }}>
        <Mark size={22} />
        <span style={{ font: "800 15px var(--font-sans)", letterSpacing: "-0.02em" }}>
          Kitchen Table
        </span>
      </div>

      <Group label="Set up" steps={REQUIRED} current={current} onJump={onJump} />
      <Group label="Optional" steps={OPTIONAL} current={current} onJump={onJump} />

      <div
        style={{
          marginTop: "auto",
          padding: 8,
          font: "400 10.5px var(--font-mono)",
          color: "var(--faint)",
        }}
      >
        macOS · v0.1.0
      </div>
    </nav>
  );
}

function Group({
  label,
  steps,
  current,
  onJump,
}: {
  label: string;
  steps: Step[];
  current: Step;
  onJump: (step: Step) => void;
}) {
  return (
    <>
      <div
        style={{
          font: "600 10px var(--font-mono)",
          color: "var(--faint)",
          letterSpacing: "0.08em",
          textTransform: "uppercase",
          padding: "12px 8px 8px",
        }}
      >
        {label}
      </div>
      {steps.map((step) => {
        const done = stepIndex(step) < stepIndex(current);
        const here = step === current;

        return (
          <button
            key={step}
            type="button"
            onClick={() => onJump(step)}
            aria-current={here ? "step" : undefined}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 11,
              padding: "7px 8px",
              borderRadius: 8,
              border: "none",
              textAlign: "left",
              cursor: "pointer",
              background: here ? "var(--accent-tint)" : "transparent",
            }}
          >
            <span
              aria-hidden="true"
              style={{
                width: 22,
                height: 22,
                borderRadius: "50%",
                flex: "none",
                display: "flex",
                alignItems: "center",
                justifyContent: "center",
                font: "700 11px var(--font-mono)",
                background: done ? "var(--green)" : here ? "var(--accent)" : "transparent",
                color: done || here ? "#fff" : "var(--faint)",
                border: done || here ? "1.5px solid transparent" : "1.5px solid var(--border2)",
              }}
            >
              {done ? "✓" : stepIndex(step) + 1}
            </span>
            <span
              style={{
                font: "600 12.5px var(--font-sans)",
                color: here ? "var(--ink)" : done ? "var(--ink2)" : "var(--muted)",
              }}
            >
              {LABELS[step]}
            </span>
          </button>
        );
      })}
    </>
  );
}
