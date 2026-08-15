import { useState } from "react";
import { Modal, ModalActions, ModalButton, ModalHeading } from "./Modal";

/**
 * Add an app.
 *
 * Two ways in, both of which end the same place: a folder inside the watched
 * workspace. Dropping one there already works without this dialog - the
 * registry watcher picks it up within seconds - so this exists to name that
 * fact for someone who has not learned it yet, and to give the folder a name up
 * front when they would rather type than drag.
 */
export function NewAppModal({
  workspace,
  dropping,
  onCreate,
  onPick,
  onPickFile,
  onClose,
}: {
  workspace?: string;
  /** True while a dropped folder is being copied in. */
  dropping?: boolean;
  onCreate: (name: string) => Promise<unknown>;
  /** Opens the native folder picker. Resolves to null if cancelled. */
  onPick: () => Promise<unknown | null>;
  /** Opens the native file picker. Resolves to null if cancelled. */
  onPickFile: () => Promise<unknown | null>;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slug = slugify(name);
  const working = busy || dropping === true;

  async function run(action: () => Promise<unknown>, closeOnNull = false) {
    setBusy(true);
    setError(null);
    try {
      const result = await action();
      // The picker resolves to null when it is cancelled, which is not a
      // success to close on.
      if (result !== null || closeOnNull) onClose();
      else setBusy(false);
    } catch (raw) {
      setError(message(raw));
      setBusy(false);
    }
  }

  const create = () => (slug ? run(() => onCreate(name.trim()), true) : undefined);
  const pick = () => run(onPick);
  const pickFile = () => run(onPickFile);

  return (
    <Modal title="Add an app" onClose={onClose}>
      <ModalHeading title="Add an app">
        Any folder becomes an app automatically. No config required to start.
      </ModalHeading>

      {/* A drop target rather than a button. It used to be one, and clicking
          it opened a folder-only picker - so the obvious thing to press
          silently refused half of what the dialog offered to host. The two
          ways in are now spelled out below it, as equals. */}
      <div
        style={{
          display: "block",
          width: "100%",
          border: `2px dashed ${dropping ? "var(--accent)" : "var(--dash)"}`,
          background: dropping ? "var(--accent-tint)" : "transparent",
          borderRadius: 14,
          padding: "28px 34px 24px",
          textAlign: "center",
          marginBottom: 16,
          transition: "border-color .15s, background .15s",
        }}
      >
        <span
          aria-hidden="true"
          style={{
            width: 44,
            height: 44,
            borderRadius: 12,
            background: "var(--accent-tint)",
            margin: "0 auto 12px",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            font: "700 20px var(--font-sans)",
            color: "var(--accent)",
          }}
        >
          ＋
        </span>
        <span
          style={{
            display: "block",
            font: "600 14px var(--font-sans)",
            color: "var(--ink)",
            marginBottom: 4,
          }}
        >
          {dropping ? "Copying it in…" : "Drop a folder or a file here"}
        </span>
        <span
          style={{ display: "block", font: "400 12px var(--font-mono)", color: "var(--muted)" }}
        >
          a folder, or a single page, PDF or image
        </span>

        <div style={{ display: "flex", gap: 10, justifyContent: "center", marginTop: 14 }}>
          <ChooseButton onClick={pick} disabled={working}>
            Choose a folder
          </ChooseButton>
          <ChooseButton onClick={pickFile} disabled={working}>
            Choose a file
          </ChooseButton>
        </div>

        {workspace && (
          <span
            style={{
              display: "block",
              marginTop: 12,
              font: "400 11px var(--font-mono)",
              color: "var(--faint)",
              wordBreak: "break-all",
            }}
          >
            copied into {workspace}
          </span>
        )}
      </div>

      <label style={{ display: "block", marginBottom: 16 }}>
        <span
          style={{
            display: "block",
            font: "600 11px var(--font-mono)",
            color: "var(--muted)",
            letterSpacing: "0.05em",
            textTransform: "uppercase",
            marginBottom: 7,
          }}
        >
          Or start an empty one
        </span>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void create();
          }}
          placeholder="Packing list"
          disabled={working}
          style={{
            width: "100%",
            border: "1px solid var(--border2)",
            borderRadius: 10,
            padding: "10px 12px",
            background: "var(--paper)",
            color: "var(--ink)",
            font: "400 13.5px var(--font-sans)",
            outline: "none",
          }}
        />
      </label>

      <div
        style={{
          background: "var(--raised)",
          borderRadius: 11,
          padding: 14,
          font: "400 12px/1.7 var(--font-mono)",
          color: "var(--ink3)",
          marginBottom: 16,
        }}
      >
        <div style={{ color: "var(--muted)" }}>
          // app.json (optional — generated for you)
        </div>
        <div>
          <Key>"name"</Key>: <Value>"{slug || "packing-list"}"</Value>,
        </div>
        <div>
          <Key>"visibility"</Key>: <Value>"private"</Value>,
        </div>
        <div>
          <Key>"entry"</Key>: <Value>"index.html"</Value>
        </div>
      </div>

      {error && (
        <p
          role="alert"
          style={{
            margin: "0 0 14px",
            font: "500 12.5px var(--font-sans)",
            color: "var(--danger)",
          }}
        >
          {error}
        </p>
      )}

      <ModalActions>
        <ModalButton onClick={onClose}>Cancel</ModalButton>
        <ModalButton
          variant="primary"
          onClick={() => void create()}
          disabled={!slug || working}
          title={slug ? undefined : "Give the app a name first"}
        >
          {working ? "Adding…" : "Add app"}
        </ModalButton>
      </ModalActions>
    </Modal>
  );
}

/**
 * Daemon errors arrive as `{code, message}` rather than as an `Error`, and the
 * message is already written for a person - "Trip Planner is already in your
 * workspace" - so it is shown as-is.
 */
/**
 * One of the two ways in, drawn the same as the other.
 *
 * Two buttons rather than a cleverer single one: the platform dialog is a
 * folder picker or a file picker - `directory` is a boolean, and macOS cannot
 * be asked for either through this plugin. Equal weight because neither is the
 * unusual case: a folder is a site, a file is a menu.
 */
function ChooseButton({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      style={{
        border: "1px solid var(--border2)",
        borderRadius: 9,
        padding: "8px 14px",
        background: "var(--paper)",
        color: "var(--ink2)",
        font: "600 12.5px var(--font-sans)",
        cursor: disabled ? "wait" : "pointer",
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </button>
  );
}

function message(raw: unknown): string {
  if (typeof raw === "object" && raw !== null && "message" in raw) {
    return String((raw as { message: unknown }).message);
  }
  return raw instanceof Error ? raw.message : String(raw);
}

function Key({ children }: { children: React.ReactNode }) {
  return <span style={{ color: "var(--accent)" }}>{children}</span>;
}

function Value({ children }: { children: React.ReactNode }) {
  return <span style={{ color: "var(--green)" }}>{children}</span>;
}

/**
 * The slug preview.
 *
 * Deliberately the same shape as the registry's rule - lowercase, non-alphanum
 * to hyphens, collapsed and trimmed - so what the dialog shows is what the
 * daemon will pick. The daemon still has the last word, including on
 * collisions, which it resolves by suffixing.
 */
export function slugify(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
