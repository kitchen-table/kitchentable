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
  onCreate,
  onClose,
}: {
  workspace?: string;
  /**
   * Creates an empty app in the workspace. Absent until `app.create` is on the
   * socket, in which case the dialog explains the drop instead of pretending.
   */
  onCreate?: (name: string) => Promise<unknown>;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slug = slugify(name);

  async function create() {
    if (!onCreate || !slug) return;
    setBusy(true);
    setError(null);
    try {
      await onCreate(name.trim());
      onClose();
    } catch (raw) {
      setError(raw instanceof Error ? raw.message : String(raw));
      setBusy(false);
    }
  }

  return (
    <Modal title="Add an app" onClose={onClose}>
      <ModalHeading title="Add an app">
        Any folder becomes an app automatically. No config required to start.
      </ModalHeading>

      <div
        style={{
          border: "2px dashed var(--dash)",
          borderRadius: 14,
          padding: 34,
          textAlign: "center",
          marginBottom: 16,
        }}
      >
        <div
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
        </div>
        <div style={{ font: "600 14px var(--font-sans)", marginBottom: 4 }}>
          Drop a folder into your workspace
        </div>
        <div style={{ font: "400 12px var(--font-mono)", color: "var(--muted)" }}>
          HTML, CSS, JS, PDFs, images
        </div>
        {workspace && (
          <div
            style={{
              marginTop: 10,
              font: "400 11px var(--font-mono)",
              color: "var(--faint)",
              wordBreak: "break-all",
            }}
          >
            {workspace}
          </div>
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
          disabled={!onCreate || busy}
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
          disabled={!onCreate || !slug || busy}
          title={onCreate ? undefined : "Creating apps from here is not wired up yet"}
        >
          {busy ? "Adding…" : "Add app"}
        </ModalButton>
      </ModalActions>
    </Modal>
  );
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
