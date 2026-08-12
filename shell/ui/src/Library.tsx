import { useMemo, useState } from "react";
import type { App, Visibility } from "./generated";
import { VISIBILITY, VISIBILITY_ORDER, tileColour } from "./visibility";

/**
 * The app library: the main window's home, per the desktop mockup.
 *
 * A sidebar of filters, a grid of app cards, and a drop target. Everything
 * here renders from `app.list`; the component holds no state the daemon owns.
 */
export function Library({ apps, workspace }: { apps: App[]; workspace: string }) {
  const [filter, setFilter] = useState<Visibility | null>(null);

  const counts = useMemo(() => {
    const out: Partial<Record<Visibility, number>> = {};
    for (const app of apps) {
      out[app.visibility] = (out[app.visibility] ?? 0) + 1;
    }
    return out;
  }, [apps]);

  const shown = filter ? apps.filter((a) => a.visibility === filter) : apps;

  return (
    <div style={{ display: "flex", height: "100%", minHeight: 0 }}>
      <Sidebar
        total={apps.length}
        counts={counts}
        filter={filter}
        onFilter={setFilter}
        workspace={workspace}
      />
      <section
        style={{
          flex: 1,
          minWidth: 0,
          overflowY: "auto",
          padding: "26px 30px",
        }}
      >
        <header style={{ marginBottom: 22 }}>
          <h1
            style={{
              margin: 0,
              font: "800 26px/1.1 var(--font-sans)",
              letterSpacing: "-0.02em",
            }}
          >
            {filter ? VISIBILITY[filter].label : "Library"}
          </h1>
          <p
            style={{
              margin: "3px 0 0",
              font: "400 13px var(--font-mono)",
              color: "var(--muted)",
            }}
          >
            {workspace} · watching · {apps.length}{" "}
            {apps.length === 1 ? "app" : "apps"} live
          </p>
        </header>

        {apps.length === 0 ? <EmptyWorkspace /> : <Grid apps={shown} />}
      </section>
    </div>
  );
}

function Sidebar({
  total,
  counts,
  filter,
  onFilter,
  workspace,
}: {
  total: number;
  counts: Partial<Record<Visibility, number>>;
  filter: Visibility | null;
  onFilter: (v: Visibility | null) => void;
  workspace: string;
}) {
  return (
    <nav
      style={{
        width: 228,
        flex: "none",
        background: "var(--raised)",
        borderRight: "1px solid var(--border)",
        padding: "16px 12px",
        display: "flex",
        flexDirection: "column",
        gap: 2,
      }}
    >
      <Heading>Workspace</Heading>
      <Row
        label="Library"
        count={total}
        active={filter === null}
        onClick={() => onFilter(null)}
      />

      <Heading style={{ marginTop: 18 }}>Filters</Heading>
      {VISIBILITY_ORDER.map((v) => (
        <Row
          key={v}
          label={VISIBILITY[v].label}
          count={counts[v] ?? 0}
          dot={VISIBILITY[v].colour}
          active={filter === v}
          onClick={() => onFilter(filter === v ? null : v)}
        />
      ))}

      <div style={{ marginTop: "auto", padding: "10px 10px 4px" }}>
        <div
          style={{
            font: "400 10.5px var(--font-mono)",
            color: "var(--faint)",
            wordBreak: "break-all",
          }}
          title={workspace}
        >
          {workspace}
        </div>
      </div>
    </nav>
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
    <div
      style={{
        font: "600 10.5px var(--font-mono)",
        color: "var(--faint)",
        letterSpacing: "0.06em",
        textTransform: "uppercase",
        padding: "4px 10px 8px",
        ...style,
      }}
    >
      {children}
    </div>
  );
}

function Row({
  label,
  count,
  dot,
  active,
  onClick,
}: {
  label: string;
  count: number;
  dot?: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      style={{
        display: "flex",
        alignItems: "center",
        gap: 10,
        padding: "8px 11px",
        borderRadius: 9,
        border: "none",
        cursor: "pointer",
        textAlign: "left",
        font: "600 13px var(--font-sans)",
        background: active ? "var(--accent-tint)" : "transparent",
        color: active ? "var(--accent)" : "var(--ink2)",
      }}
    >
      {dot && (
        <span
          style={{
            width: 9,
            height: 9,
            borderRadius: 3,
            background: dot,
            flex: "none",
          }}
        />
      )}
      {label}
      <span
        style={{
          marginLeft: "auto",
          font: "500 11px var(--font-mono)",
          color: active ? "var(--accent)" : "var(--faint)",
        }}
      >
        {count}
      </span>
    </button>
  );
}

function Grid({ apps }: { apps: App[] }) {
  if (apps.length === 0) {
    return (
      <p style={{ font: "400 14px var(--font-sans)", color: "var(--ink3)" }}>
        Nothing at this level yet.
      </p>
    );
  }

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))",
        gap: 16,
      }}
    >
      {apps.map((app) => (
        <Card key={app.slug} app={app} />
      ))}
    </div>
  );
}

function Card({ app }: { app: App }) {
  const vis = VISIBILITY[app.visibility];
  // Anything other than exactly `<slug>.local` means the network already had
  // that name and we were suffixed. Say so on the card rather than letting the
  // URL look like a bug. Note this is an equality check, not a prefix one:
  // `notes-2.local` does start with `notes`.
  const renamed = app.hostname !== undefined && app.hostname !== `${app.slug}.local`;

  return (
    <article
      style={{
        background: "var(--paper)",
        border: "1px solid var(--border)",
        borderRadius: 14,
        padding: 18,
        display: "flex",
        flexDirection: "column",
        gap: 14,
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 12 }}>
        <span
          aria-hidden="true"
          style={{
            width: 44,
            height: 44,
            borderRadius: 11,
            flex: "none",
            background: tileColour(app.slug),
            color: "#fff",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            font: "700 18px var(--font-sans)",
          }}
        >
          {app.name.charAt(0).toUpperCase()}
        </span>
        <div style={{ minWidth: 0, flex: 1 }}>
          <h2 style={{ margin: 0, font: "700 15px var(--font-sans)" }}>
            {app.name}
          </h2>
          <a
            href={app.url}
            target="_blank"
            rel="noreferrer"
            style={{
              display: "block",
              font: "400 11.5px var(--font-mono)",
              color: "var(--accent)",
              textDecoration: "none",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
            title={app.url}
          >
            {app.url.replace(/^https?:\/\//, "")}
          </a>
        </div>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <span
          style={{
            font: "600 10.5px var(--font-mono)",
            color: vis.colour,
            background: vis.tint,
            padding: "4px 9px",
            borderRadius: 6,
          }}
        >
          {vis.label}
        </span>
        <span style={{ font: "500 10.5px var(--font-mono)", color: "var(--muted)" }}>
          v{app.version}
        </span>
        {renamed && (
          <span
            style={{
              marginLeft: "auto",
              font: "500 10px var(--font-mono)",
              color: "var(--gold)",
            }}
            title={`The name ${app.slug}.local was taken on this network`}
          >
            renamed
          </span>
        )}
      </div>
    </article>
  );
}

function EmptyWorkspace() {
  return (
    <div
      style={{
        border: "2px dashed var(--border2)",
        borderRadius: 14,
        padding: "48px 24px",
        textAlign: "center",
        color: "var(--ink3)",
      }}
    >
      <p style={{ margin: "0 0 6px", font: "600 15px var(--font-sans)" }}>
        Drop a folder into your workspace
      </p>
      <p style={{ margin: 0, font: "400 13px var(--font-sans)" }}>
        Anything with an <code style={{ fontFamily: "var(--font-mono)" }}>index.html</code>{" "}
        becomes an app within seconds. No config needed.
      </p>
    </div>
  );
}
