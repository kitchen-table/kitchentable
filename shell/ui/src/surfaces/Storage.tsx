import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { App, StorageMode } from "../generated";
import { call } from "../daemon";
import { SyncIcon, DesktopIcon, BackupIcon } from "../icons";

/**
 * Where an app's data lives, and what is in it.
 *
 * Two questions on one screen, in the order somebody asks them. The choice at
 * the top is the owner's - is this a list we share, or a thing each of us keeps
 * our own copy of - and the table underneath is what that choice has produced.
 *
 * The choice is deliberately not a switch. A switch has an off state and an
 * implied default, and neither is true here: both modes are ordinary, and
 * picking between them is a decision about the app rather than a setting being
 * turned on. Two cards, each saying what it means in the words somebody would
 * use to describe their own situation.
 */

/** The two cards, in the order the mockup puts them. */
const MODES: {
  value: StorageMode;
  title: string;
  body: string;
  icon: (colour: string) => React.ReactNode;
}[] = [
  {
    value: "synced",
    title: "Keep in sync everywhere",
    body: "Edits on your phone show up on your laptop and back. Everyone sees one shared set of data — best for a shared list, board, or tracker.",
    icon: (colour) => <SyncIcon size={19} colour={colour} />,
  },
  {
    value: "per_device",
    title: "Separate on each device",
    body: "Each phone or computer keeps its own private copy — nothing is shared between them. Best for personal notes, drafts, or per-user settings.",
    icon: (colour) => <DesktopIcon size={19} colour={colour} />,
  },
];

export function Storage({ app }: { app: App }) {
  const queryClient = useQueryClient();

  const setMode = useMutation({
    mutationFn: (params: { mode: StorageMode; backup?: boolean }) =>
      call("storage.set_mode", { slug: app.slug, ...params }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["apps"] }),
  });

  // Under Synced the data is already on this machine, so there is nothing to
  // copy and the switch has nothing to act on. Drawn as unavailable with the
  // reason rather than as a control that silently does nothing.
  const backupApplies = app.storage === "per_device";

  return (
    <div style={{ maxWidth: 820 }}>
      <Label>WHERE THIS APP'S DATA LIVES</Label>

      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: 12,
          marginBottom: 12,
        }}
      >
        {MODES.map((mode) => (
          <ModeCard
            key={mode.value}
            mode={mode}
            chosen={app.storage === mode.value}
            busy={setMode.isPending}
            onChoose={() => setMode.mutate({ mode: mode.value })}
          />
        ))}
      </div>

      <Backup
        app={app}
        applies={backupApplies}
        busy={setMode.isPending}
        onToggle={() =>
          setMode.mutate({ mode: app.storage, backup: !app.storage_backup })
        }
      />

      <Label>STORED DATA</Label>
      <StoredData app={app} />
    </div>
  );
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        font: "600 11px var(--font-mono)",
        color: "var(--muted)",
        letterSpacing: "0.05em",
        marginBottom: 10,
      }}
    >
      {children}
    </div>
  );
}

function ModeCard({
  mode,
  chosen,
  busy,
  onChoose,
}: {
  mode: (typeof MODES)[number];
  chosen: boolean;
  busy: boolean;
  onChoose: () => void;
}) {
  const accent = chosen ? "var(--accent)" : "var(--idle)";

  return (
    <button
      type="button"
      role="radio"
      aria-checked={chosen}
      disabled={busy}
      onClick={onChoose}
      style={{
        textAlign: "left",
        background: chosen ? "var(--accent-tint)" : "var(--paper)",
        border: `2px solid ${chosen ? "var(--accent)" : "var(--border2)"}`,
        borderRadius: 13,
        padding: 16,
        cursor: busy ? "wait" : "pointer",
        font: "inherit",
        color: "var(--ink)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 10, marginBottom: 9 }}>
        {mode.icon(chosen ? "var(--accent)" : "var(--ink2)")}
        <span style={{ font: "700 14.5px var(--font-sans)" }}>{mode.title}</span>
        <span
          aria-hidden="true"
          style={{
            marginLeft: "auto",
            width: 17,
            height: 17,
            flex: "none",
            borderRadius: "50%",
            border: `2px solid ${accent}`,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <span
            style={{
              width: 7,
              height: 7,
              borderRadius: "50%",
              background: chosen ? "var(--accent)" : "transparent",
            }}
          />
        </span>
      </div>
      <div
        style={{
          font: "400 12px/1.5 var(--font-sans)",
          color: "var(--ink3)",
          textWrap: "pretty",
        }}
      >
        {mode.body}
      </div>
    </button>
  );
}

/**
 * Whether this machine keeps a copy of what each device holds.
 *
 * The copy is written as each change arrives rather than on a schedule, which
 * is why there is no "last backup" clock to keep honest and no question about
 * what happens while the Mac is asleep: a device that could not reach the
 * daemon simply has not been copied yet, and says so the next time it can.
 */
function Backup({
  app,
  applies,
  busy,
  onToggle,
}: {
  app: App;
  applies: boolean;
  busy: boolean;
  onToggle: () => void;
}) {
  const on = applies && app.storage_backup;

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 14,
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: "15px 18px",
        marginBottom: 22,
        opacity: applies ? 1 : 0.55,
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 38,
          height: 38,
          flex: "none",
          borderRadius: 10,
          background: "var(--green-tint)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <BackupIcon size={20} colour="var(--green)" />
      </span>

      <div style={{ flex: 1 }}>
        <div style={{ font: "700 13.5px var(--font-sans)", marginBottom: 2 }}>
          Back up device data to this Mac
        </div>
        <div
          style={{
            font: "400 11.5px/1.45 var(--font-sans)",
            color: "var(--ink3)",
            textWrap: "pretty",
          }}
        >
          {!applies
            ? "Only applies when each device keeps its own copy. Right now this app's data is already here."
            : on
              ? "On — a copy of every device's data is saved here immediately, so nothing is lost if a phone breaks or gets wiped."
              : "Off — data stays only on each device. If a phone is lost or wiped, its data goes with it."}
        </div>
      </div>

      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label="Back up device data to this Mac"
        disabled={!applies || busy}
        onClick={onToggle}
        style={{
          width: 38,
          height: 22,
          flex: "none",
          border: "none",
          padding: 0,
          borderRadius: 22,
          background: on ? "var(--accent)" : "var(--idle)",
          position: "relative",
          cursor: applies && !busy ? "pointer" : "not-allowed",
          transition: "background .15s",
        }}
      >
        <span
          aria-hidden="true"
          style={{
            position: "absolute",
            top: 2,
            left: on ? 18 : 2,
            width: 18,
            height: 18,
            background: "#fff",
            borderRadius: "50%",
            transition: "left .15s",
          }}
        />
      </button>
    </div>
  );
}

/**
 * What is actually in the store.
 *
 * Not built yet, and drawn as such rather than as an empty table: an empty
 * table is a claim that the app has stored nothing, and until this is reading
 * the store that claim would be a guess.
 */
function StoredData({ app }: { app: App }) {
  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: "16px 18px",
        font: "400 13px var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      Reading {app.name}'s stored data is not built yet. Apps write to it with{" "}
      <span style={{ font: "400 12.5px var(--font-mono)", color: "var(--ink2)" }}>
        storage.get / set / list / delete
      </span>
      , and no backend code.
    </div>
  );
}
