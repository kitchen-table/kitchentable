import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { App, StorageMode } from "../generated";
import { call } from "../daemon";
import { size } from "../activity";
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

  // Whether keeping a copy here is a choice, or a consequence. Syncing works
  // by holding the data on this machine, so under Synced the copy exists
  // whatever anybody would prefer - and the switch says so rather than
  // pretending to be a control.
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
 * Whether this machine keeps a copy of the app's data.
 *
 * A separate question from where the data lives, and the two are only related
 * in one direction: syncing works *by* keeping the copy here, so under Synced
 * this is on and cannot be turned off. It is drawn on and locked rather than
 * greyed and off, which is what it used to be and was a plain lie - the Mac
 * holds every byte of a synced app, and a switch reading "off" said otherwise.
 *
 * Under Separate on each device it is an ordinary choice, and the copy is
 * written as each change arrives rather than on a schedule - so there is no
 * "last backup" clock to keep honest and no question about what happens while
 * the Mac is asleep. A device that could not reach the daemon has simply not
 * been copied yet, and is the next time it can.
 */
function Backup({
  app,
  applies,
  busy,
  onToggle,
}: {
  app: App;
  /** Whether this is a choice here, rather than a consequence of syncing. */
  applies: boolean;
  busy: boolean;
  onToggle: () => void;
}) {
  // Syncing puts the data here by definition, so the honest reading is "on".
  const on = applies ? app.storage_backup : true;

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
          {applies ? "Back up device data to this Mac" : "A copy is kept on this Mac"}
        </div>
        <div
          style={{
            font: "400 11.5px/1.45 var(--font-sans)",
            color: "var(--ink3)",
            textWrap: "pretty",
          }}
        >
          {!applies
            ? "On — keeping this app in sync works by holding the data here, so a copy is always on this Mac. Choose “Separate on each device” if you want that to be your choice."
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

/** What one `storage.query` answers with, in either of its two shapes. */
interface Stored {
  bytes: number;
  entries?: { key: string; value: string; kind: string; updated_at: number }[];
  devices?: { device: string; name: string; keys: number }[];
}

/**
 * What is actually in the store.
 *
 * Two shapes of the same table. Per-app is the shared store's keys and values;
 * per-viewer is a row per device with how much each holds - and deliberately
 * not what is in it. The owner's window is not a reason to build a way to read
 * somebody's private notes, and the daemon does not send them.
 */
function StoredData({ app }: { app: App }) {
  const [scope, setScope] = useState<"app" | "devices">("app");
  const stored = useQuery({
    queryKey: ["storage", app.slug, scope],
    queryFn: () => call<Stored>("storage.query", { slug: app.slug, scope }),
    retry: false,
  });

  const columns = scope === "app" ? "minmax(0,1fr) minmax(0,1.4fr) 70px" : "minmax(0,1fr) minmax(0,1.4fr) 70px";
  const headings = scope === "app" ? ["KEY", "VALUE", "TYPE"] : ["VIEWER", "DATA", "KEYS"];

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 16,
        }}
      >
        <div
          role="tablist"
          style={{
            display: "inline-flex",
            background: "var(--raised)",
            border: "1px solid var(--border)",
            borderRadius: 9,
            padding: 3,
          }}
        >
          {(["app", "devices"] as const).map((value) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={scope === value}
              onClick={() => setScope(value)}
              style={{
                padding: "7px 14px",
                borderRadius: 7,
                border: "none",
                cursor: "pointer",
                font: "600 12.5px var(--font-sans)",
                background: scope === value ? "var(--paper)" : "transparent",
                color: scope === value ? "var(--ink)" : "var(--muted)",
              }}
            >
              {value === "app" ? "Per-app" : "Per-viewer"}
            </button>
          ))}
        </div>
        <div style={{ font: "400 12px var(--font-mono)", color: "var(--muted)" }}>
          SQLite · isolated · {stored.data ? size(stored.data.bytes) : "…"}
        </div>
      </div>

      <div
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border)",
          borderRadius: 12,
          overflow: "hidden",
        }}
      >
        <div
          style={{
            display: "grid",
            gridTemplateColumns: columns,
            gap: 16,
            padding: "12px 18px",
            background: "var(--raised)",
            font: "600 10.5px var(--font-mono)",
            color: "var(--muted)",
            letterSpacing: "0.05em",
          }}
        >
          {headings.map((heading) => (
            <span key={heading}>{heading}</span>
          ))}
        </div>

        {stored.isPending ? (
          <Note>Reading the store…</Note>
        ) : stored.isError ? (
          <Note>The daemon did not answer. This will fill in when it does.</Note>
        ) : scope === "app" ? (
          (stored.data?.entries ?? []).length === 0 ? (
            <Note>
              Nothing stored yet. {app.name} writes here by calling{" "}
              <Mono>storage.set</Mono>.
            </Note>
          ) : (
            stored.data?.entries?.map((entry) => (
              <Line key={entry.key} columns={columns}>
                <Cell>{entry.key}</Cell>
                <Cell accent>{entry.value}</Cell>
                <Small>{entry.kind}</Small>
              </Line>
            ))
          )
        ) : (stored.data?.devices ?? []).length === 0 ? (
          <Note>
            No device has stored anything here. Devices keep their own data only
            when this app is set to keep it separate.
          </Note>
        ) : (
          stored.data?.devices?.map((row) => (
            <Line key={row.device} columns={columns}>
              <Cell>{row.name}</Cell>
              {/* What each device holds, not what is in it. */}
              <Cell accent>kept on this device</Cell>
              <Small>
                {row.keys} {row.keys === 1 ? "key" : "keys"}
              </Small>
            </Line>
          ))
        )}
      </div>

      <div
        style={{
          marginTop: 14,
          font: "400 12px/1.5 var(--font-sans)",
          color: "var(--muted)",
        }}
      >
        Exposed to the app frontend as{" "}
        <Mono>storage.get / set / list / delete</Mono> plus matching REST
        endpoints. No backend code required.
      </div>
    </>
  );
}

function Line({
  columns,
  children,
}: {
  columns: string;
  children: React.ReactNode;
}) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: columns,
        gap: 16,
        padding: "13px 18px",
        borderTop: "1px solid var(--divider)",
        alignItems: "center",
      }}
    >
      {children}
    </div>
  );
}

function Cell({ children, accent }: { children: React.ReactNode; accent?: boolean }) {
  return (
    <span
      style={{
        font: `${accent ? 400 : 500} 12.5px var(--font-mono)`,
        color: accent ? "var(--accent)" : "var(--ink)",
        minWidth: 0,
        overflow: "hidden",
        textOverflow: "ellipsis",
        whiteSpace: "nowrap",
      }}
    >
      {children}
    </span>
  );
}

function Small({ children }: { children: React.ReactNode }) {
  return (
    <span style={{ font: "500 10.5px var(--font-mono)", color: "var(--muted)" }}>
      {children}
    </span>
  );
}

function Mono({ children }: { children: React.ReactNode }) {
  return (
    <span style={{ font: "400 12px var(--font-mono)", color: "var(--ink2)" }}>
      {children}
    </span>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return (
    <p
      style={{
        margin: 0,
        padding: "16px 18px",
        font: "400 13px var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      {children}
    </p>
  );
}
