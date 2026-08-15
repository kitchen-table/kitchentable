import { useState } from "react";
import { DeviceShapeIcon } from "../icons";
import {
  type Device,
  describeAgent,
  deviceShape,
  nameSource,
  useDeviceActions,
} from "../devices";
import { Modal, ModalActions, ModalButton } from "./Modal";

/**
 * A device is waiting to be let in.
 *
 * This is the whole trust mechanic of the product in one dialog, and until it
 * existed a shared app simply did not work for anyone who could not talk to the
 * socket by hand: the viewer's phone sat on the wait page forever.
 *
 * Naming happens here rather than afterwards. The owner is looking at a
 * fingerprint and a browser string and deciding whether it is the phone in
 * front of them; that is the only moment they know what to call it, and a list
 * of four devices all called "iPhone" is not a list anyone can revoke from.
 */
export function PairingModal({
  device,
  /** How many others are also waiting, so a queue is not a surprise. */
  waiting,
  onClose,
}: {
  device: Device;
  waiting: number;
  onClose: () => void;
}) {
  const [name, setName] = useState(device.name);
  const { approve, deny } = useDeviceActions();
  const busy = approve.isPending || deny.isPending;

  return (
    <Modal title="A device wants access" width={420} onClose={onClose}>
      <div style={{ display: "flex", alignItems: "center", gap: 13, marginBottom: 16 }}>
        <span
          aria-hidden="true"
          style={{
            width: 46,
            height: 46,
            flex: "none",
            borderRadius: 12,
            background: "var(--accent-tint)",
            color: "var(--accent)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
          }}
        >
          <DeviceShapeIcon shape={deviceShape(device.user_agent)} size={22} />
        </span>
        <div style={{ minWidth: 0 }}>
          <div style={{ font: "800 17px var(--font-sans)" }}>New device wants access</div>
          <div style={{ font: "400 11px var(--font-mono)", color: "var(--muted)" }}>
            fingerprint · {device.fingerprint} · {describeAgent(device.user_agent)}
          </div>
        </div>
      </div>

      <p
        style={{
          margin: "0 0 18px",
          font: "400 13px/1.6 var(--font-sans)",
          color: "var(--ink2)",
        }}
      >
        Someone opened one of your shared apps on a device Kitchen Table has not
        seen before. Nothing is served to it until you say so.
      </p>

      <label style={{ display: "block", marginBottom: 18 }}>
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
          Name this device
        </span>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !busy) {
              approve.mutate({ id: device.id, name }, { onSuccess: onClose });
            }
          }}
          disabled={busy}
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
        <span
          style={{
            display: "block",
            marginTop: 6,
            font: "400 11px var(--font-sans)",
            color: device.named_by === "network" ? "var(--green)" : "var(--faint)",
          }}
        >
          {device.named_by === "network" ? "✓ " : ""}
          {nameSource(device) ?? "Your name for it."} You can rename or revoke it
          any time.
        </span>
      </label>

      {(approve.error || deny.error) && (
        <p
          role="alert"
          style={{
            margin: "0 0 14px",
            font: "500 12.5px var(--font-sans)",
            color: "var(--danger)",
          }}
        >
          {String(
            (approve.error as { message?: string })?.message ??
              (deny.error as { message?: string })?.message ??
              "That did not work.",
          )}
        </p>
      )}

      <ModalActions>
        {waiting > 1 && (
          <span
            style={{
              marginRight: "auto",
              alignSelf: "center",
              font: "500 11.5px var(--font-mono)",
              color: "var(--faint)",
            }}
          >
            {waiting - 1} more waiting
          </span>
        )}
        <ModalButton
          variant="danger"
          disabled={busy}
          onClick={() => deny.mutate(device.id, { onSuccess: onClose })}
        >
          Deny
        </ModalButton>
        <ModalButton
          variant="primary"
          disabled={busy}
          onClick={() => approve.mutate({ id: device.id, name }, { onSuccess: onClose })}
        >
          {approve.isPending ? "Approving…" : "Approve & pair"}
        </ModalButton>
      </ModalActions>
    </Modal>
  );
}
