import { useState } from "react";
import { Mark } from "../Mark";
import type { App, SysStatus } from "../generated";
import { Rail } from "./Rail";
import { Screens } from "./Screens";
import {
  isSkippable,
  markComplete,
  next,
  previous,
  STEPS,
  stepIndex,
  type Step,
} from "./steps";

/**
 * First run.
 *
 * Two minutes from here to seeing your own app on your own phone
 * (onboarding.md section 1). Every OS prompt gets a screen before it that says
 * what is about to appear and what happens if you say no - the rule that
 * exists mostly for the macOS Local Network prompt, which is shown exactly
 * once and which a reflexive "Don't Allow" would silently kill the product
 * with.
 */
export function Onboarding({
  apps,
  status,
  onDone,
}: {
  apps: App[];
  status?: SysStatus;
  onDone: () => void;
}) {
  const [step, setStep] = useState<Step>("welcome");

  function finish() {
    markComplete();
    onDone();
  }

  return (
    <div style={{ display: "flex", height: "100%", minHeight: 0 }}>
      <Rail current={step} onJump={setStep} />

      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "44px 48px" }}>
          <Screens step={step} apps={apps} status={status} onNext={() => setStep(next(step))} />
        </div>

        <footer
          style={{
            flex: "none",
            height: 66,
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            padding: "0 30px",
            borderTop: "1px solid var(--border)",
            background: "var(--raised)",
          }}
        >
          <button
            type="button"
            onClick={() => setStep(previous(step))}
            disabled={step === "welcome"}
            style={{
              border: "none",
              background: "none",
              font: "600 13.5px var(--font-sans)",
              color: step === "welcome" ? "var(--faint)" : "var(--ink2)",
              cursor: step === "welcome" ? "default" : "pointer",
            }}
          >
            ← Back
          </button>

          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            {STEPS.map((s) => (
              <span
                key={s}
                aria-hidden="true"
                style={{
                  width: 7,
                  height: 7,
                  borderRadius: "50%",
                  background:
                    s === step
                      ? "var(--accent)"
                      : stepIndex(s) < stepIndex(step)
                        ? "var(--accent-bd)"
                        : "var(--border2)",
                }}
              />
            ))}
          </div>

          <div style={{ display: "flex", alignItems: "center", gap: 16 }}>
            <span style={{ font: "500 12px var(--font-mono)", color: "var(--faint)" }}>
              Step {stepIndex(step) + 1} of {STEPS.length}
            </span>
            {isSkippable(step) && (
              <button
                type="button"
                onClick={() => (step === "relay" ? finish() : setStep(next(step)))}
                style={{
                  border: "none",
                  background: "none",
                  font: "600 13.5px var(--font-sans)",
                  color: "var(--ink3)",
                  cursor: "pointer",
                }}
              >
                Skip
              </button>
            )}
            <button
              type="button"
              onClick={() => (step === "relay" ? finish() : setStep(next(step)))}
              style={{
                border: "none",
                borderRadius: 9,
                padding: "10px 22px",
                background: "var(--accent)",
                color: "#fff",
                font: "600 13.5px var(--font-sans)",
                cursor: "pointer",
              }}
            >
              {step === "relay" ? "Finish" : "Continue"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

/** The mark, sized for the welcome screen. */
export function BigMark() {
  return <Mark size={56} />;
}
