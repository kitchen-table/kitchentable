import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { App, RelayMode, Visibility } from "./generated";
import { call } from "./daemon";
import { VISIBILITY, VISIBILITY_ORDER } from "./visibility";
import {
  RELAY,
  STRICT_AVAILABLE,
  reachability,
  relayApplies,
  strandedRelay,
  suggestedFor,
} from "./relay";
import { expiry } from "./activity";
import { useStatus } from "./daemon";
import { useAccount, useBeginUpgrade } from "./account";
import { Modal } from "./chrome/Modal";

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
 * Who can open this app, the links that let them, and whether it leaves the
 * house at all.
 *
 * The two halves are deliberately separate controls and separate socket calls.
 * **Visibility says who may open an app; the relay says whether it is on the
 * internet.** Changing who may open something must never publish it as a side
 * effect, so the picker cannot reach `share.set_relay` and the relay block
 * cannot reach `share.set_visibility`.
 *
 * A tab of the app detail view, which owns the app's name, URL and the way
 * back. This renders only the controls.
 */
export function Sharing({ app }: { app: App }) {
  const queryClient = useQueryClient();
  const [gateOpen, setGateOpen] = useState(false);

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

  const setRelay = useMutation({
    mutationFn: (mode: RelayMode) =>
      call("share.set_relay", { slug: app.slug, mode }),
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

  function publish(mode: RelayMode) {
    setGateOpen(false);
    setRelay.mutate(mode);
  }

  return (
    <section aria-label={`Sharing for ${app.name}`}>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 20 }}>
        <div>
          <Heading>Who can open {app.name}</Heading>
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
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
        </div>

        <div>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              marginBottom: 10,
            }}
          >
            <Heading style={{ margin: 0 }}>Invite links</Heading>
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
              <p
                style={{
                  margin: 0,
                  font: "400 13px var(--font-sans)",
                  color: "var(--ink3)",
                }}
              >
                No links yet. Make one and send it however you like.
              </p>
            )}
            <p
              style={{
                margin: 0,
                background: "var(--gold-tint)",
                border: "1px solid var(--gold-bd)",
                borderRadius: 11,
                padding: 13,
                font: "400 12px/1.5 var(--font-sans)",
                color: "var(--ink3)",
              }}
            >
              <b style={{ color: "var(--gold)" }}>Tip.</b> Pin a link to the
              first device that opens it, so a forwarded link can't be reused by
              a stranger.
            </p>
          </div>
        </div>
      </div>

      {/*
       * No `onChoose` here any more: with Strict deferred there is one mode, so
       * the block has nothing to switch between. `setRelay.mutate(mode)` is the
       * line to put back alongside the card grid when Strict ships.
       */}
      <RelayBlock
        app={app}
        busy={setRelay.isPending}
        onTurnOn={() => setGateOpen(true)}
        onTurnOff={() => setRelay.mutate("off")}
      />

      {gateOpen && (
        <RelayGate
          app={app}
          onChoose={publish}
          onClose={() => setGateOpen(false)}
        />
      )}
    </section>
  );
}

/**
 * Whether this app is on the internet, and on what terms.
 *
 * Below the picker rather than beside it, because it is a second decision taken
 * after the first one and not a fifth visibility level.
 */
function RelayBlock({
  app,
  busy,
  onTurnOn,
  onTurnOff,
}: {
  app: App;
  busy: boolean;
  onTurnOn: () => void;
  onTurnOff: () => void;
}) {
  const on = app.relay !== "off";
  const applies = relayApplies(app.visibility);
  const stranded = strandedRelay(app.visibility, app.relay);
  const status = useStatus(true);
  // The badge reads the tunnel, not the manifest. See `reachability`.
  const reach = reachability(on, status.data?.relay);
  // No handle means no hostname exists for any app on this machine, so there is
  // nowhere to publish to. Asked here as well as in `PublicUrl` because the
  // switch has to know before it is pressed, not after.
  //
  // Three states, not two. Until the daemon has answered we do not know, and
  // swapping the card out on arrival would flash one screen into another - so
  // the offer stays drawn with its button disabled, the same treatment the
  // launch-at-login switch gives an answer it has not received.
  // Called unconditionally: `known && !useRelaySuffix()` short-circuits, which
  // skips a hook on some renders and takes the whole surface down with it.
  const suffix = useRelaySuffix();
  const known = status.data !== undefined;
  const noHandle = known && !suffix;

  return (
    <div style={{ marginTop: 22 }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 9,
          marginBottom: 10,
        }}
      >
        <Heading style={{ margin: 0 }}>Relay · reachable away from home</Heading>
        <span
          style={{
            display: "flex",
            alignItems: "center",
            gap: 5,
            font: "600 9.5px var(--font-mono)",
            color: reach.colour,
            background: reach.tint,
            padding: "3px 8px",
            borderRadius: 5,
          }}
        >
          {on && (
            <span
              aria-hidden="true"
              style={{
                width: 6,
                height: 6,
                borderRadius: "50%",
                background: reach.colour,
                animation: reach.busy ? "kt-pulse 1.1s ease-in-out infinite" : undefined,
              }}
            />
          )}
          {reach.label}
        </span>
      </div>

      {reach.detail && (
        <p
          role="status"
          style={{
            margin: "0 0 10px",
            font: "400 12px/1.5 var(--font-sans)",
            color: reach.colour,
          }}
        >
          {reach.detail}
        </p>
      )}

      {stranded && <RelayStranded level={app.visibility} />}

      {!applies && !on && <RelayDoesNotApply level={app.visibility} />}

      {applies && !on && noHandle && <RelayHasNowhereToGo app={app} />}

      {applies && !on && !noHandle && (
        <div
          style={{
            background:
              "linear-gradient(135deg, var(--accent-tint), var(--gold-tint))",
            border: "1px solid var(--accent-bd)",
            borderRadius: 13,
            padding: 18,
            display: "flex",
            alignItems: "center",
            gap: 16,
          }}
        >
          <div style={{ flex: 1 }}>
            <div
              style={{
                font: "700 14px var(--font-sans)",
                marginBottom: 3,
              }}
            >
              Turn on the relay for {app.name}
            </div>
            <div
              style={{
                font: "400 12px/1.5 var(--font-sans)",
                color: "var(--ink3)",
              }}
            >
              Give it a stable public URL that works away from your network.
              Nothing else in your workspace is published by this.
            </div>
          </div>
          <button
            type="button"
            onClick={onTurnOn}
            disabled={busy || !known}
            style={{
              background: "var(--accent)",
              color: "#fff",
              border: "none",
              padding: "11px 17px",
              borderRadius: 10,
              font: "600 13px var(--font-sans)",
              whiteSpace: "nowrap",
              cursor: busy || !known ? "default" : "pointer",
              opacity: known ? 1 : 0.6,
            }}
          >
            Turn on relay
          </button>
        </div>
      )}

      {on && (
        <>
          {/*
           * One card, not two. Strict is deferred, so there is one mode - and a
           * pair of radio buttons with one working answer is a choice drawn
           * where none exists. That is the same fault the whole tab was built
           * to remove.
           *
           * Not the same case as the disabled Strict option below: a control
           * somebody came looking for gets told why it will not work, but a
           * control nobody asked for is just a dead switch. Restore the grid
           * when Strict ships.
           */}
          <ModeCard mode="standard" only busy={busy} onSelect={() => {}} />

          <PublicUrl app={app} />

          <RelayNote mode={app.relay === "strict" ? "strict" : "standard"} />

          <div
            style={{
              marginTop: 12,
              display: "flex",
              alignItems: "center",
              gap: 10,
            }}
          >
            <span
              style={{
                display: "flex",
                alignItems: "center",
                gap: 7,
                font: "500 12px var(--font-sans)",
                color: "var(--ink3)",
              }}
            >
              <span
                aria-hidden="true"
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: "50%",
                  background:
                    app.relay === "strict" ? "var(--strict)" : "var(--gold)",
                }}
              />
              No snapshot stored · this app goes dark when your Mac sleeps
            </span>
            <button
              type="button"
              onClick={onTurnOff}
              disabled={busy}
              style={{
                marginLeft: "auto",
                border: "none",
                background: "none",
                cursor: busy ? "default" : "pointer",
                font: "600 12px var(--font-sans)",
                color: "var(--danger)",
              }}
            >
              Turn off relay
            </button>
          </div>
        </>
      )}
    </div>
  );
}

/**
 * Published, at a level the relay can never serve.
 *
 * One click away: publish as Invited, then change your mind about who may open
 * it. Nothing leaks - the gate refuses every relayed request - but the app is
 * left registered as published, and the owner is left believing one of two
 * different wrong things depending on which control they happen to look at.
 *
 * Said loudly, with the way out immediately below it: the relay block is now
 * always rendered while the relay is on, precisely so there is one.
 */
function RelayStranded({ level }: { level: Visibility }) {
  const info = VISIBILITY[level];
  return (
    <p
      role="alert"
      style={{
        margin: "0 0 10px",
        background: "var(--gold-tint)",
        border: "1px solid var(--gold-bd)",
        borderRadius: 12,
        padding: "13px 16px",
        font: "400 12.5px/1.55 var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      <b style={{ color: "var(--gold)" }}>
        This app is published but nothing can reach it.
      </b>{" "}
      The relay is on, and <b style={{ fontWeight: 600 }}>{info.label}</b> means
      it can only ever be opened{" "}
      {level === "private" ? "on this machine" : "from your home network"} — so
      every request from outside is refused. Set it back to{" "}
      <b style={{ fontWeight: 600 }}>Invited</b>, or turn the relay off below.
    </p>
  );
}

/**
 * Why there is no switch here.
 *
 * Private is satisfiable only on loopback and Household means being on the home
 * network, so a relayed request against either can only ever be refused. An
 * inert switch that does nothing but refuse would be the same class of lie as a
 * blurb promising reach the app does not have, so the reason is stated instead.
 */
function RelayDoesNotApply({ level }: { level: Visibility }) {
  const info = VISIBILITY[level];
  return (
    <p
      style={{
        margin: 0,
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 13,
        padding: 16,
        font: "400 12.5px/1.55 var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      <b style={{ color: "var(--ink2)", fontWeight: 600 }}>
        {info.label} apps stay on this network.
      </b>{" "}
      {level === "private"
        ? "Private means your own devices, so there is nothing for a relay to reach."
        : "Household means being on a network you've marked as home, which is exactly what being away is not."}{" "}
      Set this app to <b style={{ fontWeight: 600 }}>Invited</b> to reach it from
      your phone away from home.
    </p>
  );
}

/**
 * One relay mode, as a card.
 *
 * `only` is the case where it is the *sole* mode - Strict is deferred, so
 * there is nothing to choose between. It renders as a statement rather than a
 * control: no radio, no pressed state, not a button. A radio group with one
 * option is a choice drawn where none exists, which is the fault this tab was
 * built to remove.
 */
function ModeCard({
  mode,
  selected = false,
  only = false,
  busy,
  onSelect,
}: {
  mode: Exclude<RelayMode, "off">;
  selected?: boolean;
  only?: boolean;
  busy: boolean;
  onSelect: () => void;
}) {
  const info = RELAY[mode];
  const unavailable = mode === "strict" && !STRICT_AVAILABLE;
  const disabled = busy || unavailable;
  const Box = only ? "div" : "button";

  return (
    <Box
      {...(only
        ? {}
        : {
            type: "button" as const,
            onClick: onSelect,
            disabled,
            "aria-pressed": selected,
          })}
      style={{
        textAlign: "left",
        background: selected ? "var(--accent-tint)" : "var(--paper)",
        border: `2px solid ${selected ? "var(--accent)" : "var(--border)"}`,
        borderRadius: 13,
        padding: 16,
        cursor: only || disabled ? "default" : "pointer",
        opacity: unavailable ? 0.62 : 1,
      }}
    >
      <span
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          marginBottom: 12,
        }}
      >
        <span style={{ font: "700 14.5px var(--font-sans)" }}>
          {info.promise}
        </span>
        <span
          style={{
            font: "600 9.5px var(--font-mono)",
            color: info.colour,
            background: info.tint,
            padding: "3px 7px",
            borderRadius: 5,
          }}
        >
          {mode === "standard" ? "DEFAULT" : "END-TO-END"}
        </span>
        {unavailable && <Soon>NOT YET</Soon>}
        {/* No radio when this is the only mode — see `only` above. */}
        {!only && (
          <span
            aria-hidden="true"
            style={{
              marginLeft: "auto",
              width: 17,
              height: 17,
              flex: "none",
              borderRadius: "50%",
              border: `2px solid ${selected ? "var(--accent)" : "var(--border2)"}`,
              background: selected
                ? "radial-gradient(circle, var(--accent) 0 4px, transparent 4px)"
                : "none",
            }}
          />
        )}
      </span>

      <span style={{ display: "flex", flexDirection: "column", gap: 9 }}>
        {info.points.map((point) => (
          <span
            key={point.text}
            style={{ display: "flex", alignItems: "flex-start", gap: 9 }}
          >
            <PointIcon warn={point.warn} soon={point.soon} />
            <span
              style={{
                font: "500 12.5px/1.4 var(--font-sans)",
                color: point.warn || point.soon ? "var(--muted)" : "var(--ink2)",
              }}
            >
              {point.bold && (
                <b style={{ fontWeight: 600 }}>{point.bold} — </b>
              )}
              {point.text}
              {point.soon && " (not built yet)"}
            </span>
          </span>
        ))}
      </span>

      {unavailable && (
        <span
          style={{
            display: "block",
            marginTop: 12,
            font: "400 11.5px/1.45 var(--font-sans)",
            color: "var(--muted)",
          }}
        >
          The edge terminates TLS today, so there is no passthrough to choose
          yet. Kitchen Table will not quietly serve Standard in its place.
        </span>
      )}
    </Box>
  );
}

function PointIcon({ warn, soon }: { warn?: boolean; soon?: boolean }) {
  const stroke = warn ? "var(--danger)" : soon ? "var(--gold)" : "var(--green)";
  return (
    <svg
      viewBox="0 0 24 24"
      width="15"
      height="15"
      aria-hidden="true"
      style={{ flex: "none", marginTop: 1 }}
      fill="none"
      stroke={stroke}
      strokeWidth="2.6"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {warn ? (
        <>
          <line x1="6" y1="6" x2="18" y2="18" />
          <line x1="18" y1="6" x2="6" y2="18" />
        </>
      ) : soon ? (
        <>
          <circle cx="12" cy="12" r="9" />
          <polyline points="12 7 12 12 15.5 14" />
        </>
      ) : (
        <polyline points="4 12 10 18 20 6" />
      )}
    </svg>
  );
}

/**
 * The suffix every public address on this machine ends with.
 *
 * `-adarsh.kitchentable.cloud`. Install-level, because one handle covers every
 * app here - that is what makes a single entry in the edge enough. `null` until
 * an account claims a handle, which is every install today.
 */
function useRelaySuffix(): string | null {
  const status = useStatus(true);
  const handle = status.data?.relay_handle;
  const domain = status.data?.relay_domain;
  return handle && domain ? `-${handle}.${domain}` : null;
}

/**
 * The relay, on a machine that has no handle to publish under - and the way
 * to get one.
 *
 * Drawn instead of the Turn-on card, and the reason is the rule this whole tab
 * exists for. A public hostname is `<label>-<handle>.<domain>`, so with no
 * handle there is no name for any app here - pressing the button wrote
 * `relay: standard` into the manifest, changed nothing about what anybody could
 * reach, and then explained itself afterwards with "this app is published, but
 * there is no public address". A switch that can only do nothing is the same
 * class of lie as a blurb promising reach, and it is worse for arriving after
 * the press rather than before it.
 *
 * So the card offers the one action that would make the switch real: the
 * upgrade. The whole ceremony happens in a browser - see `useBeginUpgrade` -
 * and while it is in flight the card says so, because "not linked" beside a
 * checkout somebody is mid-way through reads as the button having failed.
 *
 * Stated as a fact about the machine rather than about the app, because it is
 * one: every app here is in the same position, and none of them is broken.
 */
function RelayHasNowhereToGo({ app }: { app: App }) {
  const account = useAccount(true);
  const upgrade = useBeginUpgrade();
  const waiting = account.data?.link?.state === "waiting";

  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 13,
        padding: 18,
        display: "flex",
        alignItems: "center",
        gap: 16,
      }}
    >
      <div style={{ flex: 1 }}>
        <div style={{ font: "700 14px var(--font-sans)", marginBottom: 4 }}>
          {waiting ? "Finish in your browser" : "Not available yet"}
        </div>
        <p
          style={{
            margin: 0,
            font: "400 12px/1.55 var(--font-sans)",
            color: "var(--ink3)",
          }}
        >
          {waiting ? (
            <>
              The upgrade page is open in your browser. The moment it is done,
              this machine gets its handle and this switch comes alive — this
              window updates by itself, nothing to come back and press.
            </>
          ) : (
            <>
              Publishing needs a handle — the part after the hyphen in a public
              address — and this machine does not have one until you link an
              account. There is nowhere to publish to yet, so there is nothing
              to switch on. <b style={{ fontWeight: 600 }}>{app.name}</b> is
              reachable on your network at{" "}
              <b style={{ fontWeight: 600 }}>{app.url}</b>, as it was before.
            </>
          )}
          {upgrade.isError && (
            <>
              {" "}
              <b style={{ color: "var(--gold)" }}>
                {(upgrade.error as { message?: string }).message ??
                  "Could not start the upgrade."}
              </b>
            </>
          )}
        </p>
      </div>
      <button
        type="button"
        onClick={() => upgrade.mutate()}
        disabled={upgrade.isPending}
        style={{
          background: waiting ? "var(--paper)" : "var(--accent)",
          color: waiting ? "var(--ink2)" : "#fff",
          border: waiting ? "1px solid var(--border)" : "none",
          padding: "10px 16px",
          borderRadius: 10,
          font: "600 13px var(--font-sans)",
          whiteSpace: "nowrap",
          cursor: upgrade.isPending ? "default" : "pointer",
        }}
      >
        {waiting ? "Reopen the page" : "Link an account"}
      </button>
    </div>
  );
}

/**
 * The address this app answers to away from home, and the field to change it.
 *
 * Editable because the slug comes from a folder name, and a folder name is a
 * thing you chose for yourself, not a thing you wanted to send to somebody.
 * `trip-briefing-html-template` is a fine folder and an embarrassing URL.
 *
 * Only the label is editable. The handle belongs to the machine and the domain
 * to us, so both are shown as fixed text rather than a field somebody can put
 * a hyphen in - a handle containing one would make every URL already issued
 * ambiguous.
 */
function PublicUrl({ app }: { app: App }) {
  const suffix = useRelaySuffix();
  const [copied, setCopied] = useState(false);
  const [editing, setEditing] = useState(false);

  async function copy() {
    if (!app.public_url) return;
    await navigator.clipboard.writeText(app.public_url);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  }

  if (!suffix) {
    return (
      <p
        style={{
          margin: "12px 0 0",
          background: "var(--gold-tint)",
          border: "1px solid var(--gold-bd)",
          borderRadius: 12,
          padding: "13px 16px",
          font: "400 12px/1.55 var(--font-sans)",
          color: "var(--ink3)",
        }}
      >
        <b style={{ color: "var(--gold)" }}>No public address yet.</b> The relay
        is switched on for this app, but this machine has no handle to hang a
        name off — that arrives when you link an account — so nothing is
        published anywhere. It is reachable on your network only, at{" "}
        <b style={{ fontWeight: 600 }}>{app.url}</b>, exactly as it was before.
      </p>
    );
  }

  if (editing) {
    return (
      <AddressEditor
        app={app}
        suffix={suffix}
        onDone={() => setEditing(false)}
      />
    );
  }

  return (
    <div
      style={{
        marginTop: 12,
        display: "flex",
        alignItems: "center",
        gap: 12,
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: "13px 16px",
      }}
    >
      <LinkGlyph />
      <span style={{ flex: 1, minWidth: 0 }}>
        <span
          style={{
            display: "block",
            font: "600 10px var(--font-mono)",
            color: "var(--muted)",
            letterSpacing: "0.05em",
            marginBottom: 2,
          }}
        >
          PUBLIC ADDRESS
        </span>
        <span
          title={app.public_url ?? undefined}
          style={{
            display: "block",
            font: "400 13px var(--font-mono)",
            color: "var(--accent)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {app.public_label}
          {suffix}
        </span>
      </span>
      <button
        type="button"
        onClick={() => setEditing(true)}
        style={{
          flex: "none",
          padding: "7px 13px",
          borderRadius: 9,
          border: "1px solid var(--border2)",
          background: "var(--paper)",
          cursor: "pointer",
          font: "600 12px var(--font-sans)",
          color: "var(--ink2)",
        }}
      >
        Edit
      </button>
      <button
        type="button"
        onClick={copy}
        style={{
          flex: "none",
          border: "none",
          background: "none",
          cursor: "pointer",
          font: "600 12px var(--font-sans)",
          color: "var(--accent)",
        }}
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

function LinkGlyph() {
  return (
    <svg
      viewBox="0 0 24 24"
      width="18"
      height="18"
      aria-hidden="true"
      style={{ flex: "none" }}
      fill="none"
      stroke="var(--accent)"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.5 1.5" />
      <path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.5-1.5" />
    </svg>
  );
}

/**
 * Renaming an address in place.
 *
 * The suffix is shown but not editable, so what is being typed is never in
 * doubt: people are used to a domain field meaning the whole domain, and this
 * one is a single label with the machine's handle already spent.
 *
 * The daemon is the authority on what is allowed - it holds the length limit,
 * which depends on the handle, and the only list of what other apps answer to.
 * So its refusal is shown verbatim rather than reimplementing the rules here
 * and having two answers to the same question.
 */
function AddressEditor({
  app,
  suffix,
  onDone,
}: {
  app: App;
  suffix: string;
  onDone: () => void;
}) {
  const queryClient = useQueryClient();
  const [label, setLabel] = useState(app.public_label);
  const [error, setError] = useState<string | null>(null);

  const save = useMutation({
    mutationFn: (next: string) =>
      call("share.set_public_label", { slug: app.slug, label: next }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["apps"] });
      queryClient.invalidateQueries({ queryKey: ["invites", app.slug] });
      onDone();
    },
    onError: (e: unknown) => {
      const message =
        typeof e === "object" && e !== null && "message" in e
          ? String((e as { message: unknown }).message)
          : "That address could not be saved.";
      setError(message);
    },
  });

  const unchanged = label.trim().toLowerCase() === app.public_label;

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        if (unchanged) return onDone();
        setError(null);
        save.mutate(label);
      }}
      style={{
        marginTop: 12,
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: "13px 16px",
      }}
    >
      <label
        htmlFor={`address-${app.slug}`}
        style={{
          display: "block",
          font: "600 10px var(--font-mono)",
          color: "var(--muted)",
          letterSpacing: "0.05em",
          marginBottom: 7,
        }}
      >
        PUBLIC ADDRESS
      </label>

      <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span
          style={{
            flex: 1,
            display: "flex",
            alignItems: "center",
            border: `1px solid ${error ? "var(--danger)" : "var(--border2)"}`,
            borderRadius: 9,
            overflow: "hidden",
            background: "var(--desk)",
          }}
        >
          <input
            id={`address-${app.slug}`}
            value={label}
            autoFocus
            spellCheck={false}
            autoCapitalize="none"
            onChange={(event) => {
              setLabel(event.target.value);
              setError(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") onDone();
            }}
            style={{
              flex: 1,
              minWidth: 0,
              border: "none",
              outline: "none",
              background: "transparent",
              padding: "9px 10px",
              font: "400 13px var(--font-mono)",
              color: "var(--ink)",
            }}
          />
          <span
            style={{
              flex: "none",
              padding: "9px 10px 9px 0",
              font: "400 13px var(--font-mono)",
              color: "var(--muted)",
              whiteSpace: "nowrap",
            }}
          >
            {suffix}
          </span>
        </span>

        <button
          type="submit"
          disabled={save.isPending}
          style={{
            flex: "none",
            padding: "9px 15px",
            borderRadius: 9,
            border: "none",
            background: "var(--accent)",
            color: "#fff",
            cursor: save.isPending ? "default" : "pointer",
            font: "600 12.5px var(--font-sans)",
          }}
        >
          {save.isPending ? "Saving" : "Save"}
        </button>
        <button
          type="button"
          onClick={onDone}
          style={{
            flex: "none",
            border: "none",
            background: "none",
            cursor: "pointer",
            font: "600 12.5px var(--font-sans)",
            color: "var(--ink3)",
          }}
        >
          Cancel
        </button>
      </div>

      {error ? (
        <p
          role="alert"
          style={{
            margin: "8px 0 0",
            font: "500 11.5px/1.45 var(--font-sans)",
            color: "var(--danger)",
          }}
        >
          {error}
        </p>
      ) : (
        <p
          style={{
            margin: "8px 0 0",
            font: "400 11.5px/1.45 var(--font-sans)",
            color: "var(--muted)",
          }}
        >
          Anyone holding the old address will stop being able to open this app.
        </p>
      )}
    </form>
  );
}

/** What the chosen mode means, spelled out under the choice. */
function RelayNote({ mode }: { mode: Exclude<RelayMode, "off"> }) {
  const info = RELAY[mode];
  return (
    <div
      style={{
        marginTop: 12,
        display: "flex",
        gap: 12,
        background: info.tint,
        border: `1px solid ${info.border}`,
        borderRadius: 12,
        padding: "14px 16px",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 6,
          flex: "none",
          borderRadius: 3,
          background: info.colour,
        }}
      />
      <div>
        <div
          style={{
            font: "700 12.5px var(--font-sans)",
            color: info.colour,
            marginBottom: 3,
          }}
        >
          {info.note.head}
        </div>
        <div
          style={{
            font: "400 12px/1.55 var(--font-sans)",
            color: "var(--ink3)",
          }}
        >
          {info.note.body}
        </div>
      </div>
    </div>
  );
}

/**
 * The choice, at the moment the relay is switched on.
 *
 * Never defaulted silently. The two modes trade privacy against staying
 * reachable, and which of those matters is the owner's to decide - so the
 * switch does not turn on until one of them is picked.
 */
function RelayGate({
  app,
  onChoose,
  onClose,
}: {
  app: App;
  onChoose: (mode: RelayMode) => void;
  onClose: () => void;
}) {
  const suggested = suggestedFor(app.visibility);

  return (
    <Modal title={`Turn on the relay for ${app.name}`} width={540} onClose={onClose}>
      <div
        style={{
          font: "600 10.5px var(--font-mono)",
          color: "var(--muted)",
          letterSpacing: "0.05em",
          textTransform: "uppercase",
          marginBottom: 10,
        }}
      >
        Turn on the relay · {app.name}
      </div>
      <h2
        style={{
          font: "800 21px/1.32 var(--font-sans)",
          letterSpacing: "-0.01em",
          margin: "0 0 9px",
        }}
      >
        This app gets a public address.
      </h2>
      <p
        style={{
          font: "400 13.5px/1.6 var(--font-sans)",
          color: "var(--ink3)",
          margin: "0 0 20px",
        }}
      >
        Anyone you have shared it with can open it from anywhere, and Kitchen
        Table's servers carry it — which means they can technically read it.
        Nothing else in your workspace is published by this.
      </p>

      <GateAddress app={app} />

      {/*
       * One option, and it is the confirm. This used to open on "Should this
       * app stay reachable when this machine is asleep?" with Standard and
       * Strict beneath it, which was the right question while both modes were
       * coming. With Strict deferred it is a fork with one road, and the honest
       * shape is a confirmation of what publishing does rather than a choice
       * between two things one of which is disabled.
       *
       * Restoring the pair is this block plus the heading and lede above.
       */}
      <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
        <GateOption mode="standard" onChoose={onChoose} />
      </div>

      <div
        style={{
          marginTop: 15,
          display: "flex",
          alignItems: "center",
          gap: 9,
        }}
      >
        <span
          style={{
            flex: 1,
            font: "400 11.5px/1.5 var(--font-sans)",
            color: "var(--muted)",
          }}
        >
          {VISIBILITY[app.visibility].label} apps use{" "}
          <b style={{ color: "var(--ink2)" }}>{RELAY[suggested].promise}</b>. A
          full-privacy mode, where our servers could not read this app, is
          designed and not built.
        </span>
        <button
          type="button"
          onClick={onClose}
          style={{
            padding: "9px 15px",
            borderRadius: 9,
            font: "600 12.5px var(--font-sans)",
            color: "var(--ink3)",
            background: "none",
            border: "1px solid var(--border2)",
            cursor: "pointer",
          }}
        >
          Cancel
        </button>
      </div>
    </Modal>
  );
}

/**
 * The address, shown while deciding whether to publish.
 *
 * Here rather than only afterwards because this is the moment somebody first
 * sees what they are about to hand out, and `trip-briefing-html-template` is
 * the moment they will want to change it. Saved on its own, before a mode is
 * picked, so cancelling the publish still keeps a name they corrected - and so
 * that neither decision is quietly bundled into the other.
 */
function GateAddress({ app }: { app: App }) {
  const suffix = useRelaySuffix();
  const [editing, setEditing] = useState(false);

  if (!suffix) return null;

  if (editing) {
    return (
      <div style={{ marginBottom: 18 }}>
        <AddressEditor
          app={app}
          suffix={suffix}
          onDone={() => setEditing(false)}
        />
      </div>
    );
  }

  return (
    <div style={{ marginBottom: 18 }}>
      <div
        style={{
          font: "600 10px var(--font-mono)",
          color: "var(--muted)",
          letterSpacing: "0.05em",
          marginBottom: 7,
        }}
      >
        PUBLIC ADDRESS
      </div>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 10,
          border: "1px solid var(--border2)",
          borderRadius: 9,
          padding: "10px 12px",
        }}
      >
        <span
          style={{
            flex: 1,
            minWidth: 0,
            font: "400 13px var(--font-mono)",
            color: "var(--ink2)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {app.public_label}
          <span style={{ color: "var(--muted)" }}>{suffix}</span>
        </span>
        <button
          type="button"
          onClick={() => setEditing(true)}
          style={{
            flex: "none",
            border: "none",
            background: "none",
            cursor: "pointer",
            font: "600 12px var(--font-sans)",
            color: "var(--accent)",
          }}
        >
          Edit
        </button>
      </div>
    </div>
  );
}

function GateOption({
  mode,
  onChoose,
}: {
  mode: Exclude<RelayMode, "off">;
  onChoose: (mode: RelayMode) => void;
}) {
  const info = RELAY[mode];
  const unavailable = mode === "strict" && !STRICT_AVAILABLE;

  return (
    <button
      type="button"
      onClick={() => onChoose(mode)}
      disabled={unavailable}
      style={{
        textAlign: "left",
        border: "1px solid var(--border2)",
        borderRadius: 13,
        padding: 16,
        cursor: unavailable ? "default" : "pointer",
        background: "none",
        display: "flex",
        gap: 14,
        opacity: unavailable ? 0.62 : 1,
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 40,
          height: 40,
          borderRadius: 11,
          flex: "none",
          background: info.tint,
        }}
      />
      <span style={{ flex: 1 }}>
        <span
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginBottom: 3,
          }}
        >
          <span style={{ font: "700 14.5px var(--font-sans)" }}>
            {info.label}
          </span>
          <span
            style={{
              font: "600 9.5px var(--font-mono)",
              color: info.colour,
              background: info.tint,
              padding: "2px 6px",
              borderRadius: 5,
            }}
          >
            {info.badge}
          </span>
          {unavailable && <Soon>NOT YET</Soon>}
        </span>
        <span
          style={{
            display: "block",
            font: "400 12px/1.45 var(--font-sans)",
            color: "var(--ink3)",
          }}
        >
          {unavailable
            ? "Strict has no transport yet — the edge still terminates TLS. It will not be offered until it is real."
            : info.pitch}
        </span>
      </span>
    </button>
  );
}

/** A promise that is real but not yet implemented, marked rather than hidden. */
function Soon({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        font: "600 9.5px var(--font-mono)",
        color: "var(--gold)",
        background: "var(--gold-tint)",
        padding: "2px 6px",
        borderRadius: 5,
      }}
    >
      {children}
    </span>
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
        <span
          style={{
            display: "flex",
            alignItems: "center",
            gap: 7,
            font: "600 13.5px var(--font-sans)",
          }}
        >
          {info.label}
          {/* Public reaches "anywhere" only once the relay is on, and the
              relay is off until the owner says otherwise. Without this the
              picker's most consequential row promises something it cannot do. */}
          {level === "public" && (
            <span
              style={{
                font: "500 10px var(--font-mono)",
                color: "var(--muted)",
                background: "var(--chip)",
                padding: "2px 6px",
                borderRadius: 5,
              }}
            >
              needs relay
            </span>
          )}
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
          {invite.redemptions} {invite.redemptions === 1 ? "open" : "opens"}
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
        <Tag>{expiry(invite.expires_at)}</Tag>
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
