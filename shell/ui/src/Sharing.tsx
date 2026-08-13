import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { App, Visibility } from "./generated";
import { call } from "./daemon";
import { VISIBILITY, VISIBILITY_ORDER } from "./visibility";

export interface InviteView {
  token: string;
  app_slug: string;
  label: string;
  url: string;
  expires_at: number | null;
  pinned: boolean;
  pin_to_first_device: boolean;
  auto_approve_at_home: boolean;
  redemptions: number;
  revoked: boolean;
  active: boolean;
}

/**
 * Who can open this app, and the links that let them.
 *
 * The visibility picker is the product's most consequential control, so it
 * spells out what each level means rather than relying on the label.
 *
 * A tab of the app detail view, which owns the app's name, URL and the way
 * back. This renders only the controls.
 */
export function Sharing({ app }: { app: App }) {
  const queryClient = useQueryClient();

  const invites = useQuery({
    queryKey: ["invites", app.slug],
    queryFn: () => call<InviteView[]>("share.list_invites", { slug: app.slug }),
    retry: false,
  });

  const setVisibility = useMutation({
    mutationFn: (visibility: Visibility) =>
      call("share.set_visibility", { slug: app.slug, visibility }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["apps"] }),
  });

  const createInvite = useMutation({
    mutationFn: (label: string) =>
      call<InviteView>("share.create_invite", { slug: app.slug, label }),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["invites", app.slug] }),
  });

  const revoke = useMutation({
    mutationFn: (token: string) => call("share.revoke_invite", { token }),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["invites", app.slug] }),
  });

  return (
    <section aria-label={`Sharing for ${app.name}`} style={{ maxWidth: 720 }}>
      <Heading>Who can open {app.name}</Heading>
      <div style={{ display: "flex", flexDirection: "column", gap: 8, marginBottom: 28 }}>
        {VISIBILITY_ORDER.map((level) => (
          <LevelRow
            key={level}
            level={level}
            selected={app.visibility === level}
            busy={setVisibility.isPending}
            onSelect={() => setVisibility.mutate(level)}
          />
        ))}
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 12, marginBottom: 10 }}>
        <Heading style={{ margin: 0 }}>Share links</Heading>
        <button
          type="button"
          onClick={() => createInvite.mutate("Share link")}
          disabled={createInvite.isPending}
          style={{
            marginLeft: "auto",
            border: "none",
            background: "none",
            cursor: "pointer",
            font: "600 12px var(--font-sans)",
            color: "var(--accent)",
          }}
        >
          + New link
        </button>
      </div>

      {app.visibility !== "invited" && (
        <p
          style={{
            margin: "0 0 12px",
            font: "400 12.5px/1.5 var(--font-sans)",
            color: "var(--ink3)",
          }}
        >
          Links only let someone in while this app is set to{" "}
          <b style={{ fontWeight: 600 }}>Invited</b>.
        </p>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        {(invites.data ?? []).map((invite) => (
          <InviteRow
            key={invite.token}
            invite={invite}
            onRevoke={() => revoke.mutate(invite.token)}
          />
        ))}
        {invites.data?.length === 0 && (
          <p style={{ margin: 0, font: "400 13px var(--font-sans)", color: "var(--ink3)" }}>
            No links yet. Make one and send it however you like.
          </p>
        )}
      </div>
    </section>
  );
}

function LevelRow({
  level,
  selected,
  busy,
  onSelect,
}: {
  level: Visibility;
  selected: boolean;
  busy: boolean;
  onSelect: () => void;
}) {
  const info = VISIBILITY[level];

  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={busy}
      aria-pressed={selected}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 12,
        padding: "13px 14px",
        textAlign: "left",
        cursor: busy ? "default" : "pointer",
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderLeft: `4px solid ${selected ? info.colour : "transparent"}`,
        borderRadius: 10,
      }}
    >
      <span style={{ flex: 1 }}>
        <span style={{ display: "block", font: "600 13.5px var(--font-sans)" }}>
          {info.label}
        </span>
        <span
          style={{
            display: "block",
            font: "400 12px var(--font-sans)",
            color: "var(--muted)",
          }}
        >
          {info.blurb}
        </span>
      </span>
      <span
        aria-hidden="true"
        style={{
          width: 17,
          height: 17,
          borderRadius: "50%",
          flex: "none",
          border: `2px solid ${selected ? info.colour : "var(--border2)"}`,
          background: selected
            ? `radial-gradient(circle, ${info.colour} 0 4px, transparent 4px)`
            : "none",
        }}
      />
    </button>
  );
}

function InviteRow({
  invite,
  onRevoke,
}: {
  invite: InviteView;
  onRevoke: () => void;
}) {
  const [copied, setCopied] = useState(false);

  async function copy() {
    await navigator.clipboard.writeText(invite.url);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 11,
        padding: 14,
        opacity: invite.active ? 1 : 0.55,
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
        <span style={{ font: "700 13px var(--font-sans)" }}>{invite.label}</span>
        <span
          style={{
            marginLeft: "auto",
            font: "500 10.5px var(--font-mono)",
            color: "var(--muted)",
          }}
        >
          {invite.redemptions} {invite.redemptions === 1 ? "use" : "uses"}
        </span>
      </div>

      <div
        style={{
          font: "400 11.5px var(--font-mono)",
          color: "var(--accent)",
          overflow: "hidden",
          textOverflow: "ellipsis",
          whiteSpace: "nowrap",
          marginBottom: 10,
        }}
        title={invite.url}
      >
        {invite.url}
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <Tag>{invite.expires_at ? "expires" : "no expiry"}</Tag>
        <Tag>{invite.pinned ? "in use" : invite.pin_to_first_device ? "pins to first device" : "shareable"}</Tag>
        {invite.revoked && <Tag>revoked</Tag>}

        {invite.active && (
          <>
            <button
              type="button"
              onClick={onRevoke}
              style={{
                marginLeft: "auto",
                border: "none",
                background: "none",
                cursor: "pointer",
                font: "600 11.5px var(--font-sans)",
                color: "var(--danger)",
              }}
            >
              Revoke
            </button>
            <button
              type="button"
              onClick={copy}
              style={{
                border: "none",
                background: "none",
                cursor: "pointer",
                font: "600 11.5px var(--font-sans)",
                color: "var(--accent)",
              }}
            >
              {copied ? "Copied" : "Copy"}
            </button>
          </>
        )}
      </div>
    </div>
  );
}

function Tag({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        font: "500 10.5px var(--font-mono)",
        color: "var(--muted)",
        background: "var(--chip)",
        padding: "3px 7px",
        borderRadius: 5,
      }}
    >
      {children}
    </span>
  );
}

function Heading({
  children,
  style,
}: {
  children: React.ReactNode;
  style?: React.CSSProperties;
}) {
  return (
    <h2
      style={{
        font: "600 11px var(--font-mono)",
        color: "var(--muted)",
        letterSpacing: "0.05em",
        textTransform: "uppercase",
        margin: "0 0 10px",
        ...style,
      }}
    >
      {children}
    </h2>
  );
}
