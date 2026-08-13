import { useState } from "react";
import { ago } from "../activity";
import {
  type Device,
  approved,
  describeAgent,
  pending,
  revoked,
  useDeviceActions,
  useDevices,
} from "../devices";
import { DeviceIcon } from "../icons";
import { Empty, Panel } from "./AppDetail";

/**
 * Everything that has ever asked to open something, and what was decided.
 *
 * Devices are trusted across the whole workspace rather than per app, which is
 * what "like pairing Bluetooth" means in the product copy: you approve a phone,
 * not a phone-and-an-app. So this is the same list on every app's tab.
 */
export function Devices() {
  const devices = useDevices();
  const all = devices.data ?? [];

  const waiting = pending(all);
  const trusted = approved(all);
  const refused = revoked(all);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 18, maxWidth: 720 }}>
      {waiting.map((device) => (
        <Waiting key={device.id} device={device} />
      ))}

      <Panel title={`Approved devices · ${trusted.length}`}>
        {trusted.length === 0 ? (
          <Empty>
            Nothing has been paired yet. The first device to open a shared link
            will ask.
          </Empty>
        ) : (
          trusted.map((device) => <Row key={device.id} device={device} />)
        )}
      </Panel>

      {refused.length > 0 && (
        <Panel title={`Refused · ${refused.length}`}>
          {refused.map((device) => (
            <Row key={device.id} device={device} />
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
        background: "var(--accent-tint)",
        border: "1px solid var(--accent-bd)",
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
          background: "var(--paper)",
          color: "var(--accent)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
        }}
      >
        <DeviceIcon size={19} />
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

function Row({ device }: { device: Device }) {
  const { revoke, rename } = useDeviceActions();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(device.name);

  const owner = device.status === "owner";
  const gone = device.status === "revoked";

  function commit() {
    setEditing(false);
    const trimmed = name.trim();
    if (trimmed && trimmed !== device.name) rename.mutate({ id: device.id, name: trimmed });
    else setName(device.name);
  }

  return (
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
        <DeviceIcon size={16} />
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
          locking yourself out of your own library. */}
      {!owner && !gone && (
        <Action danger disabled={revoke.isPending} onClick={() => revoke.mutate(device.id)}>
          Revoke
        </Action>
      )}
    </div>
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
