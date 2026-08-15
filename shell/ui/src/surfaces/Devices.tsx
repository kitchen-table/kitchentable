import { useState } from "react";
import { ago } from "../activity";
import { Modal, ModalHeading } from "../chrome/Modal";
import {
  type Device,
  type DeviceGroup,
  approved,
  describeAgent,
  deviceShape,
  group,
  pending,
  revoked,
  sessionCount,
  useDeviceActions,
  useDevices,
} from "../devices";
import { DeviceShapeIcon } from "../icons";
import { Empty, Panel } from "./AppDetail";

/**
 * Everything that has ever asked to open something, and what was decided.
 *
 * Devices are trusted across the whole workspace rather than per app, which is
 * what "like pairing Bluetooth" means in the product copy: you approve a phone,
 * not a phone-and-an-app. So this is the same list on every app's tab.
 *
 * Rows are grouped, and that is the whole point of this surface working at all.
 * A device id is minted fresh whenever a request arrives without a usable
 * cookie, so one phone that has cleared its cookies is several rows - this
 * machine had thirty-six rows for four physical devices, which made the one
 * list an owner uses to recognise what is in their house unreadable. Grouping
 * is presentation and revocation only; see [`group`] for why it can never be
 * the reason anybody is let in.
 */
export function Devices() {
  const devices = useDevices();
  const all = devices.data ?? [];

  const waiting = pending(all);
  const trusted = group(approved(all));
  const refused = group(revoked(all));

  const sessions = approved(all).length;
  const heading =
    sessions > trusted.length
      ? `Approved devices · ${trusted.length} (${sessions} sessions)`
      : `Approved devices · ${trusted.length}`;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 18, maxWidth: 720 }}>
      {waiting.map((device) => (
        <Waiting key={device.id} device={device} />
      ))}

      <Panel title={heading}>
        {trusted.length === 0 ? (
          <Empty>
            Nothing has been paired yet. The first device to open a shared link
            will ask.
          </Empty>
        ) : (
          trusted.map((entry) => <Row key={entry.head.id} group={entry} />)
        )}
      </Panel>

      {refused.length > 0 && (
        <Panel title={`Refused · ${refused.length}`}>
          {refused.map((entry) => (
            <Row key={entry.head.id} group={entry} />
          ))}
        </Panel>
      )}
    </div>
  );
}

/** A device asking right now. Loud, because someone is staring at a spinner. */
function Waiting({ device }: { device: Device }) {
  const [name, setName] = useState(device.name);
  const { approve, deny } = useDeviceActions();
  const busy = approve.isPending || deny.isPending;

  return (
    <div
      style={{
        // Gold, as the mockup has it, and not the accent. Gold is what this
        // palette uses for something waiting on you - the Household level, the
        // `requested` row in the activity list - and a device at the door is
        // exactly that. The accent is for what you press, which here is Review.
        background: "var(--gold-tint)",
        border: "1px solid var(--gold-bd)",
        borderRadius: 12,
        padding: 16,
        display: "flex",
        alignItems: "center",
        gap: 14,
        flexWrap: "wrap",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 38,
          height: 38,
          flex: "none",
          borderRadius: 10,
          background: "var(--gold-tint)",
          color: "var(--gold)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <DeviceShapeIcon shape={deviceShape(device.user_agent)} size={18} />
      </span>

      <div style={{ flex: 1, minWidth: 180 }}>
        <div style={{ font: "700 13.5px var(--font-sans)", marginBottom: 3 }}>
          {device.name} wants access
        </div>
        <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)" }}>
          fingerprint · {device.fingerprint} · {describeAgent(device.user_agent)} ·
          asked {ago(device.first_seen)}
        </div>
      </div>

      <input
        value={name}
        onChange={(event) => setName(event.target.value)}
        aria-label="Name this device"
        disabled={busy}
        style={{
          flex: "none",
          width: 150,
          border: "1px solid var(--border2)",
          borderRadius: 9,
          padding: "8px 10px",
          background: "var(--paper)",
          color: "var(--ink)",
          font: "400 12.5px var(--font-sans)",
          outline: "none",
        }}
      />

      <div style={{ display: "flex", gap: 8, flex: "none" }}>
        <Action danger disabled={busy} onClick={() => deny.mutate(device.id)}>
          Deny
        </Action>
        <Action primary disabled={busy} onClick={() => approve.mutate({ id: device.id, name })}>
          Approve
        </Action>
      </div>
    </div>
  );
}

function Row({ group: entry }: { group: DeviceGroup }) {
  const device = entry.head;
  const { revokeAll, rename, forget } = useDeviceActions();
  const [editing, setEditing] = useState(false);
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [name, setName] = useState(device.name);

  const owner = device.status === "owner";
  const gone = device.status === "revoked";
  const ids = entry.sessions.map((session) => session.id);
  const count = sessionCount(entry);

  function commit() {
    setEditing(false);
    const trimmed = name.trim();
    if (trimmed && trimmed !== device.name) rename.mutate({ id: device.id, name: trimmed });
    else setName(device.name);
  }

  return (
    <>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 12,
          padding: "11px 0",
          borderTop: "1px solid var(--divider)",
          opacity: gone ? 0.55 : 1,
        }}
      >
        <span
          aria-hidden="true"
          style={{
            width: 30,
            height: 30,
            flex: "none",
            borderRadius: 8,
            background: "var(--chip)",
            color: "var(--ink3)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <DeviceShapeIcon shape={deviceShape(device.user_agent)} size={17} />
        </span>

        <div style={{ flex: 1, minWidth: 0 }}>
          {editing ? (
            <input
              value={name}
              autoFocus
              onChange={(event) => setName(event.target.value)}
              onBlur={commit}
              onKeyDown={(event) => {
                if (event.key === "Enter") commit();
                if (event.key === "Escape") {
                  setName(device.name);
                  setEditing(false);
                }
              }}
              aria-label={`Rename ${device.name}`}
              style={{
                width: "100%",
                maxWidth: 240,
                border: "1px solid var(--border2)",
                borderRadius: 7,
                padding: "5px 8px",
                background: "var(--paper)",
                color: "var(--ink)",
                font: "600 13px var(--font-sans)",
                outline: "none",
              }}
            />
          ) : (
            <button
              type="button"
              onClick={() => !owner && setEditing(true)}
              title={owner ? undefined : "Rename"}
              style={{
                border: "none",
                background: "none",
                padding: 0,
                font: "600 13px var(--font-sans)",
                color: "var(--ink)",
                cursor: owner ? "default" : "text",
                textAlign: "left",
              }}
            >
              {device.name}
            </button>
          )}
          <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)" }}>
            {device.fingerprint} · {describeAgent(device.user_agent)} · last seen{" "}
            {ago(device.last_seen)}
            {count && (
              <>
                {" · "}
                {/* The count is the honest thing to show, so it is not hidden
                    behind the expander: an owner scanning the list should see
                    that a row stands for nine, not discover it by clicking. */}
                <button
                  type="button"
                  onClick={() => setOpen((was) => !was)}
                  aria-expanded={open}
                  style={{
                    border: "none",
                    background: "none",
                    padding: 0,
                    font: "400 11px var(--font-mono)",
                    color: "var(--accent)",
                    cursor: "pointer",
                  }}
                >
                  {count} {open ? "▾" : "▸"}
                </button>
              </>
            )}
          </div>
        </div>

        <span
          style={{
            flex: "none",
            font: "600 10.5px var(--font-mono)",
            color: owner ? "var(--accent)" : gone ? "var(--muted)" : "var(--green)",
            background: owner
              ? "var(--accent-tint)"
              : gone
                ? "var(--chip)"
                : "var(--green-tint)",
            padding: "4px 9px",
            borderRadius: 6,
          }}
        >
          {owner ? "This machine" : gone ? "Refused" : "Approved"}
        </span>

        {/* The owner's own machine is where the files are; revoking it would mean
            locking yourself out of your own library, and removing it would put
            it straight back on the next request under a name nobody chose. */}
        {!owner && !gone && (
          <Action
            danger
            disabled={revokeAll.isPending}
            onClick={() => revokeAll.mutate(ids)}
          >
            Revoke
          </Action>
        )}
        {!owner && (
          <Action disabled={forget.isPending} onClick={() => setConfirming(true)}>
            Remove
          </Action>
        )}
      </div>

      {open &&
        entry.sessions.map((session) => (
          <div
            key={session.id}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 10,
              padding: "6px 0 6px 42px",
              font: "400 11px var(--font-mono)",
              color: "var(--muted)",
            }}
          >
            <span style={{ flex: 1, minWidth: 0 }}>
              {session.fingerprint} · first seen {ago(session.first_seen)} · last seen{" "}
              {ago(session.last_seen)}
            </span>
            <button
              type="button"
              disabled={forget.isPending}
              onClick={() => forget.mutate([session.id])}
              style={{
                border: "none",
                background: "none",
                padding: 0,
                font: "500 11px var(--font-mono)",
                color: "var(--ink3)",
                cursor: "pointer",
              }}
            >
              Remove
            </button>
          </div>
        ))}

      {confirming && (
        <Confirm
          title={
            entry.sessions.length > 1
              ? `Remove ${entry.sessions.length} sessions for ${device.name}?`
              : `Remove ${device.name}?`
          }
          // The second sentence is the one that matters and it is different for
          // a refused device, where removing the row undoes the refusal. Saying
          // "this does not lock anything out" and leaving it there would let an
          // owner tidy away a refusal believing it still held.
          body={
            gone
              ? `Kitchen Table stops knowing about ${device.name}. It is currently refused — removing the row drops that refusal, so if that browser comes back it will ask again rather than being turned away.`
              : `Kitchen Table stops knowing about ${device.name}. If that browser opens a shared app again it will ask to be let in, as a new device.`
          }
          working={forget.isPending}
          onConfirm={() => {
            forget.mutate(ids);
            setConfirming(false);
          }}
          onClose={() => setConfirming(false)}
        />
      )}
    </>
  );
}

function Confirm({
  title,
  body,
  working,
  onConfirm,
  onClose,
}: {
  title: string;
  body: string;
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
        <Action onClick={onClose}>Cancel</Action>
        <Action danger disabled={working} onClick={onConfirm}>
          {working ? "Working…" : "Remove"}
        </Action>
      </div>
    </Modal>
  );
}

function Action({
  children,
  onClick,
  primary,
  danger,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  primary?: boolean;
  danger?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      style={{
        flex: "none",
        border: primary ? "none" : "1px solid var(--border2)",
        borderRadius: 9,
        padding: "8px 13px",
        background: primary ? "var(--accent)" : "transparent",
        color: primary ? "#fff" : danger ? "var(--danger)" : "var(--ink3)",
        font: "600 12.5px var(--font-sans)",
        cursor: disabled ? "not-allowed" : "pointer",
        opacity: disabled ? 0.55 : 1,
      }}
    >
      {children}
    </button>
  );
}
