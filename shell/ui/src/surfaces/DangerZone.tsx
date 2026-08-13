/**
 * The two ways to stop serving an app.
 *
 * Deliberately different, and the wording is the feature: pausing is
 * reversible from this window and keeps every link and device, while forgetting
 * takes the app out of the library and leaves the folder exactly where it is.
 * Neither one deletes anything from disk, and the panel says so, because
 * "Delete" next to a folder full of someone's work needs to be unambiguous.
 */
import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { App } from "../generated";
import { call } from "../daemon";
import { Modal, ModalHeading } from "../chrome/Modal";

export function DangerZone({ app, onGone }: { app: App; onGone: () => void }) {
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState<"pause" | "forget" | null>(null);

  const pause = useMutation({
    mutationFn: (paused: boolean) =>
      call(paused ? "app.pause" : "app.resume", { slug: app.slug }),
    onSuccess: () => {
      setConfirming(null);
      void queryClient.invalidateQueries({ queryKey: ["apps"] });
    },
  });

  const forget = useMutation({
    mutationFn: () => call("app.forget", { slug: app.slug }),
    onSuccess: () => {
      setConfirming(null);
      void queryClient.invalidateQueries({ queryKey: ["apps"] });
      // Nothing left to look at: the app this surface is about is gone.
      onGone();
    },
  });

  return (
    <>
      <section
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderLeft: "3px solid var(--danger)",
          borderRadius: 12,
          padding: 18,
        }}
      >
        <h2 style={{ margin: 0, font: "700 14px var(--font-sans)" }}>Danger zone</h2>
        <p
          style={{
            margin: "2px 0 14px",
            font: "400 12px var(--font-sans)",
            color: "var(--muted)",
          }}
        >
          Take {app.name} offline, or remove it from Kitchen Table.
        </p>

        <Row
          title={app.paused ? "Paused" : "Pause serving"}
          detail={
            app.paused
              ? "Nobody can open it, including you. Their links still work."
              : "Stop serving without deleting. Resume any time."
          }
        >
          <button
            type="button"
            disabled={pause.isPending}
            onClick={() => (app.paused ? pause.mutate(false) : setConfirming("pause"))}
            style={app.paused ? resumeButton : pauseButton}
          >
            {app.paused ? "Resume" : "Pause"}
          </button>
        </Row>

        <Row
          title="Delete this app"
          detail="Kitchen Table forgets it. Your folder on disk is left untouched."
        >
          <button
            type="button"
            disabled={forget.isPending}
            onClick={() => setConfirming("forget")}
            style={deleteButton}
          >
            Delete
          </button>
        </Row>

        {(pause.isError || forget.isError) && (
          <p
            role="alert"
            style={{ margin: "10px 0 0", font: "400 12px var(--font-sans)", color: "var(--danger)" }}
          >
            {message(pause.error) ?? message(forget.error)}
          </p>
        )}
      </section>

      {confirming === "pause" && (
        <Confirm
          title={`Pause ${app.name}?`}
          body="It will stop answering for everyone, including you. Nothing is deleted, and every link and paired device keeps working when you resume."
          confirmLabel="Pause serving"
          onConfirm={() => pause.mutate(true)}
          onClose={() => setConfirming(null)}
          working={pause.isPending}
        />
      )}

      {confirming === "forget" && (
        <Confirm
          title={`Delete ${app.name}?`}
          body={`Kitchen Table stops serving it and takes it out of your library. The folder stays exactly where it is on disk — nothing in ${app.name} is deleted. Its links and paired devices are forgotten.`}
          confirmLabel="Delete from Kitchen Table"
          destructive
          onConfirm={() => forget.mutate()}
          onClose={() => setConfirming(null)}
          working={forget.isPending}
        />
      )}
    </>
  );
}

function Row({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 13,
        padding: "12px 0",
        borderTop: "1px solid var(--divider)",
      }}
    >
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ font: "600 13px var(--font-sans)" }}>{title}</div>
        <div style={{ font: "400 11.5px/1.4 var(--font-sans)", color: "var(--ink3)" }}>
          {detail}
        </div>
      </div>
      {children}
    </div>
  );
}

/**
 * Both of these are asked about first.
 *
 * Pausing is reversible and still worth a question, because it stops the app
 * for people who are reading it right now and they get no warning at all.
 */
function Confirm({
  title,
  body,
  confirmLabel,
  destructive,
  working,
  onConfirm,
  onClose,
}: {
  title: string;
  body: string;
  confirmLabel: string;
  destructive?: boolean;
  working: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Modal title={title} onClose={onClose} width={460}>
      <ModalHeading title={title} />
      <p
        style={{
          margin: "0 0 18px",
          font: "400 13px/1.6 var(--font-sans)",
          color: "var(--ink2)",
        }}
      >
        {body}
      </p>
      <div style={{ display: "flex", justifyContent: "flex-end", gap: 10 }}>
        <button type="button" onClick={onClose} style={cancelButton}>
          Cancel
        </button>
        <button
          type="button"
          onClick={onConfirm}
          disabled={working}
          style={destructive ? deleteButton : pauseButton}
        >
          {working ? "Working…" : confirmLabel}
        </button>
      </div>
    </Modal>
  );
}

function message(error: unknown): string | null {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return error ? String(error) : null;
}

const base: React.CSSProperties = {
  font: "600 12.5px var(--font-sans)",
  padding: "8px 14px",
  borderRadius: 9,
  whiteSpace: "nowrap",
  cursor: "pointer",
};

const pauseButton: React.CSSProperties = {
  ...base,
  color: "var(--gold)",
  border: "1px solid var(--gold-bd)",
  background: "var(--gold-tint)",
};

const resumeButton: React.CSSProperties = {
  ...base,
  color: "var(--accent)",
  border: "1px solid var(--accent-bd)",
  background: "var(--accent-tint)",
};

const deleteButton: React.CSSProperties = {
  ...base,
  color: "#fff",
  border: "none",
  background: "var(--danger)",
};

const cancelButton: React.CSSProperties = {
  ...base,
  color: "var(--ink2)",
  border: "1px solid var(--border2)",
  background: "none",
};
