import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { App } from "../generated";
import {
  ACTIVITY_FILTERS,
  type AccessEvent,
  type ActivityFilter,
  FILTER_LABELS,
  describe,
  filterAvailable,
  matches,
  meta,
  sentence,
  when,
} from "../activity";
import { call } from "../daemon";
import { type Device, describeAgent, useDevices } from "../devices";
import { type Surface, appSurface } from "../navigation";
import { tileColour } from "../visibility";

/**
 * The access log, as two surfaces: one app's history, and the whole workspace.
 *
 * Both are the same list of rows over the same socket method, which is why they
 * live in one file: `log.query` takes an optional slug and nothing else
 * changes. What differs is the frame - one app gets filter chips, the workspace
 * gets an app name on every row and a way into it.
 *
 * Nothing here writes. The access log is append-only by design (the owner has
 * to be able to answer "who has seen this"), so there is no action on a row and
 * no way to clear the list.
 */

/** The most rows either view asks for. The daemon caps at 1000 regardless. */
const LIMIT = 500;

function useLog(slug: string | null) {
  return useQuery({
    // Sharing the ["log"] prefix with Overview's panel, so one `access` event
    // makes every view of the log stale at once.
    queryKey: ["log", slug],
    queryFn: () =>
      call<AccessEvent[]>("log.query", { slug: slug ?? undefined, limit: LIMIT }),
    retry: false,
  });
}

/**
 * What a row needs to know about the device that caused it.
 *
 * Built once per list rather than per row: a query per row would be five
 * hundred subscriptions to the same `device.list`.
 */
function deviceIndex(devices: Device[]): Map<string, Device> {
  return new Map(devices.map((device) => [device.id, device]));
}

/** One app's history, with the mockup's filter chips above it. */
export function AppActivity({ app }: { app: App }) {
  const [filter, setFilter] = useState<ActivityFilter>("all");
  const log = useLog(app.slug);
  const devices = deviceIndex(useDevices().data ?? []);

  const events = (log.data ?? []).filter((event) => matches(event, filter));

  return (
    <div style={{ maxWidth: 760 }}>
      <div style={{ display: "flex", gap: 8, marginBottom: 16 }}>
        {ACTIVITY_FILTERS.map((value) => (
          <Chip
            key={value}
            label={FILTER_LABELS[value]}
            active={value === filter}
            available={filterAvailable(value)}
            onClick={() => setFilter(value)}
          />
        ))}
      </div>

      <Feed
        events={events}
        devices={devices}
        pending={log.isPending}
        failed={log.isError}
        empty={
          filter === "all"
            ? "Nothing yet. Opens, pairings and refusals all land here."
            : `No ${FILTER_LABELS[filter].toLowerCase()} in this app's history.`
        }
      />
    </div>
  );
}

/**
 * Everything across the workspace, newest first.
 *
 * No filter chips: the useful cut here is by app, and every row already names
 * one and opens it. Adding the per-app chips as well would be two ways to
 * narrow the same list.
 */
export function WorkspaceActivity({
  apps,
  onNavigate,
}: {
  apps: App[];
  onNavigate: (surface: Surface) => void;
}) {
  const log = useLog(null);
  const devices = deviceIndex(useDevices().data ?? []);
  const named = new Map(apps.map((app) => [app.slug, app.name]));

  return (
    <div className="kt-scroll" style={{ flex: 1, overflowY: "auto" }}>
      <div style={{ padding: "26px 30px", maxWidth: 820 }}>
        <h1
          style={{
            margin: 0,
            font: "800 26px var(--font-sans)",
            letterSpacing: "-0.02em",
          }}
        >
          Activity
        </h1>
        <p
          style={{
            margin: "4px 0 22px",
            font: "400 13px var(--font-mono)",
            color: "var(--muted)",
          }}
        >
          everything across your workspace
        </p>

        <Feed
          events={log.data ?? []}
          devices={devices}
          pending={log.isPending}
          failed={log.isError}
          empty="Nothing has happened yet. Opens, pairings and refusals all land here."
          // A forgotten app keeps its history - the log is append-only and the
          // folder is untouched - so a row can name an app the library no
          // longer lists. The slug is the honest fallback, and it is not
          // clickable, because there is nothing to open.
          appOf={(slug) => ({
            name: named.get(slug) ?? slug,
            onOpen: named.has(slug)
              ? () => onNavigate(appSurface(slug, "activity"))
              : undefined,
          })}
        />
      </div>
    </div>
  );
}

/** The card both views put their rows in. */
function Feed({
  events,
  devices,
  pending,
  failed,
  empty,
  appOf,
}: {
  events: AccessEvent[];
  devices: Map<string, Device>;
  pending: boolean;
  failed: boolean;
  empty: string;
  appOf?: (slug: string) => { name: string; onOpen?: () => void };
}) {
  return (
    <div
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 12,
        padding: "6px 18px",
      }}
    >
      {failed ? (
        <Note>The daemon did not answer. This list will fill in when it does.</Note>
      ) : pending ? (
        // "Nothing has happened" and "we have not asked yet" are different
        // claims, and this list is the one an owner checks to be sure nobody
        // opened something. It must never say the first while meaning the second.
        <Note>Reading the log…</Note>
      ) : events.length === 0 ? (
        <Note>{empty}</Note>
      ) : (
        events.map((event, index) => (
          <ActivityRow
            key={`${event.at}-${event.action}-${index}`}
            event={event}
            device={event.device_id ? devices.get(event.device_id) : undefined}
            app={event.app_slug && appOf ? appOf(event.app_slug) : undefined}
            slug={event.app_slug ?? undefined}
          />
        ))
      )}
    </div>
  );
}

/**
 * One row: what happened, the detail under it, and when.
 *
 * The glyph tile carries the app's colour when the list spans apps and the
 * action's colour when it does not - which is the mockup's rule, and a sound
 * one: in a mixed list the first thing you scan for is which app, and in a
 * single-app list that question is already answered by the page you are on.
 */
function ActivityRow({
  event,
  device,
  app,
  slug,
}: {
  event: AccessEvent;
  device?: Device;
  app?: { name: string; onOpen?: () => void };
  slug?: string;
}) {
  const kind = describe(event.action);
  const mixed = app !== undefined;

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 14,
        padding: mixed ? "14px 0" : "13px 0",
        borderBottom: "1px solid var(--divider)",
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: mixed ? 32 : 30,
          height: mixed ? 32 : 30,
          borderRadius: 8,
          flex: "none",
          background: mixed && slug ? tileColour(slug) : kind.tint,
          color: mixed ? "#fff" : kind.colour,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          font: "600 12px var(--font-mono)",
        }}
      >
        {kind.glyph}
      </span>

      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            font: "500 13.5px var(--font-sans)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {app &&
            (app.onOpen ? (
              <button
                type="button"
                onClick={app.onOpen}
                style={{
                  border: "none",
                  background: "none",
                  padding: 0,
                  font: "700 13.5px var(--font-sans)",
                  color: "var(--ink)",
                  cursor: "pointer",
                }}
              >
                {app.name}
              </button>
            ) : (
              <b style={{ fontWeight: 700 }}>{app.name}</b>
            ))}
          {app && " · "}
          {sentence(event, device?.name)}
        </div>
        <div
          style={{
            font: "400 11.5px var(--font-mono)",
            color: "var(--muted)",
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {meta(event, {
            fingerprint: device?.fingerprint,
            agent: device ? describeAgent(device.user_agent) : undefined,
          })}
        </div>
      </div>

      <span
        style={{
          flex: "none",
          font: "500 11.5px var(--font-mono)",
          color: "var(--muted)",
          whiteSpace: "nowrap",
        }}
      >
        {when(event.at)}
      </span>
    </div>
  );
}

function Note({ children }: { children: React.ReactNode }) {
  return (
    <p
      style={{
        margin: 0,
        padding: "16px 0",
        font: "400 13px var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      {children}
    </p>
  );
}

/** A filter chip. Disabled ones say why rather than quietly emptying the list. */
function Chip({
  label,
  active,
  available,
  onClick,
}: {
  label: string;
  active: boolean;
  available: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={!available}
      aria-pressed={active}
      title={
        available ? undefined : "Nothing records a deploy until versions are built"
      }
      style={{
        border: active ? "none" : "1px solid var(--border2)",
        borderRadius: 20,
        padding: "6px 12px",
        // The inverted chip, which is what the mockup selects a filter with.
        background: active ? "var(--chip-ink-bg)" : "transparent",
        color: !available
          ? "var(--faint)"
          : active
            ? "var(--chip-ink-fg)"
            : "var(--ink2)",
        font: `${active ? 600 : 500} 12px var(--font-sans)`,
        cursor: available ? "pointer" : "not-allowed",
      }}
    >
      {label}
    </button>
  );
}
