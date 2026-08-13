import { useEffect, useRef } from "react";

/**
 * The dialog shape every modal in the desktop mockup shares.
 *
 * Escape and a backdrop click both close, because every modal here is
 * cancellable - including the pairing one, where dismissing means "decide
 * later" rather than "deny". A modal that could destroy something passes
 * `dismissable={false}` and forces an explicit choice.
 */
export function Modal({
  title,
  width = 440,
  dismissable = true,
  onClose,
  children,
}: {
  /** Accessible name. Rendered by the caller, so it can be styled per design. */
  title: string;
  width?: number;
  dismissable?: boolean;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!dismissable) return;
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [dismissable, onClose]);

  // Move focus into the dialog so the keyboard follows the eye, and so Escape
  // reaches the handler above rather than whatever was focused behind it.
  useEffect(() => {
    const first = panel.current?.querySelector<HTMLElement>(
      "button:not([disabled]), input, select, textarea, [tabindex]",
    );
    (first ?? panel.current)?.focus();
  }, []);

  return (
    <div
      onMouseDown={(event) => {
        if (dismissable && event.target === event.currentTarget) onClose();
      }}
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 100,
        background: "rgba(28,25,21,.42)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        padding: 24,
        animation: "kt-fade .16s ease-out",
      }}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        style={{
          width,
          maxWidth: "100%",
          maxHeight: "100%",
          overflowY: "auto",
          background: "var(--paper)",
          borderRadius: 18,
          boxShadow: "0 40px 80px -20px rgba(0,0,0,.5)",
          padding: 26,
          outline: "none",
        }}
      >
        {children}
      </div>
    </div>
  );
}

/** The title/subtitle pair at the top of every modal in the mockup. */
export function ModalHeading({
  title,
  children,
}: {
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <>
      <div style={{ font: "800 18px var(--font-sans)", marginBottom: 4 }}>{title}</div>
      {children && (
        <div
          style={{
            font: "400 13px/1.5 var(--font-sans)",
            color: "var(--muted)",
            marginBottom: 18,
          }}
        >
          {children}
        </div>
      )}
    </>
  );
}

/** The right-aligned Cancel / confirm pair. */
export function ModalActions({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>{children}</div>
  );
}

export function ModalButton({
  children,
  onClick,
  variant = "secondary",
  disabled,
  title,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  variant?: "primary" | "secondary" | "danger";
  disabled?: boolean;
  title?: string;
}) {
  const skin = {
    primary: { background: "var(--accent)", color: "#fff", border: "none" },
    secondary: {
      background: "transparent",
      color: "var(--ink3)",
      border: "1px solid var(--border2)",
    },
    danger: { background: "var(--danger)", color: "#fff", border: "none" },
  }[variant];

  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      style={{
        padding: "10px 16px",
        borderRadius: 9,
        font: "600 13px var(--font-sans)",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.5 : 1,
        ...skin,
      }}
    >
      {children}
    </button>
  );
}
