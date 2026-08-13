import { useMemo, useState } from "react";
import type { App, Visibility } from "./generated";
import { Qr } from "./Qr";
import { VISIBILITY, tileColour } from "./visibility";

export type Sort = "recent" | "name" | "visibility";

const SORT_LABELS: Record<Sort, string> = {
  recent: "Recent",
  name: "Name",
  visibility: "Visibility",
};

/**
 * The app library: the window's home, per the desktop mockup.
 *
 * The rail and its filters live in the shell now; this renders the content
 * column only. Everything here comes from `app.list`, so the component holds no
 * state the daemon owns - only how the list is sorted, which is a view concern.
 */
export function Library({
  apps,
  filter,
  query,
  workspace,
  onOpen,
  onNewApp,
}: {
  apps: App[];
  filter: Visibility | null;
  /** The title bar's search box. Matches name, slug and URL. */
  query: string;
  workspace: string;
  onOpen: (app: App) => void;
  onNewApp?: () => void;
}) {
  const [sort, setSort] = useState<Sort>("recent");

  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const matching = apps.filter((app) => {
      if (filter && app.visibility !== filter) return false;
      if (!needle) return true;
      return (
        app.name.toLowerCase().includes(needle) ||
        app.slug.toLowerCase().includes(needle) ||
        app.url.toLowerCase().includes(needle)
      );
    });
    return sorted(matching, sort);
  }, [apps, filter, query, sort]);

  return (
    <section
      className="kt-scroll"
      style={{ flex: 1, minWidth: 0, overflowY: "auto", padding: "26px 30px" }}
    >
      <header
        style={{
          display: "flex",
          alignItems: "flex-end",
          gap: 16,
          marginBottom: 22,
        }}
      >
        <div style={{ flex: 1, minWidth: 0 }}>
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
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
            title={workspace}
          >
            {workspace} · watching · {apps.length}{" "}
            {apps.length === 1 ? "app" : "apps"} live
          </p>
        </div>

        {apps.length > 0 && <SortControl sort={sort} onSort={setSort} />}
      </header>

      {apps.length === 0 ? (
        <EmptyWorkspace workspace={workspace} onNewApp={onNewApp} />
      ) : shown.length === 0 ? (
        <p style={{ font: "400 14px var(--font-sans)", color: "var(--ink3)" }}>
          {query.trim()
            ? `Nothing matches “${query.trim()}”.`
            : "Nothing at this level yet."}
        </p>
      ) : (
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))",
            gap: 16,
          }}
        >
          {shown.map((app) => (
            <Card key={app.slug} app={app} onOpen={() => onOpen(app)} />
          ))}
          <NewAppTile onClick={onNewApp} />
        </div>
      )}
    </section>
  );
}

function sorted(apps: App[], sort: Sort): App[] {
  const out = [...apps];
  switch (sort) {
    case "name":
      return out.sort((a, b) => a.name.localeCompare(b.name));
    case "visibility":
      // Most private first: the levels are ordered by how much they expose.
      return out.sort(
        (a, b) =>
          ORDER.indexOf(a.visibility) - ORDER.indexOf(b.visibility) ||
          a.name.localeCompare(b.name),
      );
    case "recent":
      // Newest deploy first. `version` counts deploys, so it is a usable proxy
      // until the manifest carries a timestamp.
      return out.sort((a, b) => b.version - a.version || a.name.localeCompare(b.name));
  }
}

const ORDER: Visibility[] = ["private", "network", "invited", "public"];

function SortControl({ sort, onSort }: { sort: Sort; onSort: (sort: Sort) => void }) {
  return (
    <label
      style={{
        display: "flex",
        alignItems: "center",
        gap: 6,
        font: "500 12px var(--font-sans)",
        color: "var(--muted)",
        flex: "none",
      }}
    >
      Sort:
      <select
        value={sort}
        onChange={(event) => onSort(event.target.value as Sort)}
        style={{
          border: "1px solid var(--border)",
          background: "var(--paper)",
          color: "var(--ink2)",
          borderRadius: 8,
          padding: "5px 8px",
          font: "600 12px var(--font-sans)",
          cursor: "pointer",
        }}
      >
        {(Object.keys(SORT_LABELS) as Sort[]).map((value) => (
          <option key={value} value={value}>
            {SORT_LABELS[value]}
          </option>
        ))}
      </select>
    </label>
  );
}

function Card({ app, onOpen }: { app: App; onOpen: () => void }) {
  const [lifted, setLifted] = useState(false);
  const vis = VISIBILITY[app.visibility];
  // Anything other than exactly `<slug>.local` means the network already had
  // that name and we were suffixed. Say so on the card rather than letting the
  // URL look like a bug. Note this is an equality check, not a prefix one:
  // `notes-2.local` does start with `notes`.
  const renamed = app.hostname !== undefined && app.hostname !== `${app.slug}.local`;
  const insecure = app.url.startsWith("http://");

  return (
    <article
      onMouseEnter={() => setLifted(true)}
      onMouseLeave={() => setLifted(false)}
      onFocus={() => setLifted(true)}
      onBlur={() => setLifted(false)}
      style={{
        position: "relative",
        background: "var(--paper)",
        border: `1px solid ${lifted ? "rgba(47,109,240,.45)" : "var(--border)"}`,
        borderRadius: 14,
        padding: 18,
        cursor: "pointer",
        boxShadow: lifted ? "var(--shadow-lift)" : "var(--shadow-card)",
        transform: lifted ? "translateY(-2px)" : "none",
        transition: "box-shadow .18s, transform .18s, border-color .18s",
      }}
    >
      <div style={{ display: "flex", alignItems: "flex-start", gap: 12, marginBottom: 16 }}>
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
          {/* The name is the card's control. An overlay stretches its hit area
              over the whole card, so the mockup's "click anywhere" behaviour
              costs no semantics: this stays a heading with a real button in it
              rather than a div with a click handler. */}
          <h2 style={{ margin: 0, font: "700 15px var(--font-sans)" }}>
            <button
              type="button"
              onClick={onOpen}
              style={{
                border: "none",
                background: "none",
                padding: 0,
                font: "inherit",
                color: "var(--ink)",
                textAlign: "left",
                cursor: "pointer",
              }}
            >
              {app.name}
              <span
                aria-hidden="true"
                style={{ position: "absolute", inset: 0, borderRadius: 14 }}
              />
            </button>
          </h2>
          <div
            style={{
              font: "400 11.5px var(--font-mono)",
              color: "var(--muted)",
              whiteSpace: "nowrap",
              overflow: "hidden",
              textOverflow: "ellipsis",
            }}
            title={app.url}
          >
            {app.url.replace(/^https?:\/\//, "")}
          </div>
        </div>
        <span
          style={{
            width: 38,
            height: 38,
            flex: "none",
            borderRadius: 8,
            border: "1px solid var(--border)",
            overflow: "hidden",
            background: "#fff",
          }}
          title={`Scan to open ${app.name}`}
        >
          <Qr value={app.url} size={36} quiet={1} title={`Scan to open ${app.name}`} />
        </span>
      </div>

      {/* Above the stretched overlay, so the badges keep their tooltips - each
          one explains a state someone would otherwise read as a bug. */}
      <div
        style={{
          position: "relative",
          zIndex: 1,
          display: "flex",
          alignItems: "center",
          gap: 8,
          flexWrap: "wrap",
        }}
      >
        <Badge colour={vis.colour} background={vis.tint}>
          {vis.label}
        </Badge>
        <span style={{ font: "500 10.5px var(--font-mono)", color: "var(--muted)" }}>
          v{app.version}
        </span>
        {/* Loudest badge on the card, and the only one that means the app does
            not work. Everything else about it is healthy, so nothing else here
            would ever reveal that its URL is a 404. */}
        {!app.entry_exists && (
          <Badge
            colour="var(--danger)"
            background="var(--paper)"
            border="var(--danger)"
            title={`There is no ${app.entry} in this folder, so the app's address will not open`}
          >
            won&rsquo;t open
          </Badge>
        )}
        {insecure && (
          <Badge
            colour="var(--danger)"
            background="var(--paper)"
            border="var(--danger)"
            title="Local certificate not trusted on this device — served over HTTP"
          >
            Not encrypted
          </Badge>
        )}
        {app.hostname === undefined && (
          <Badge
            colour="var(--muted)"
            background="var(--chip)"
            title="No .local name was announced — use the IP fallback URL"
          >
            IP fallback
          </Badge>
        )}
        {renamed && (
          <Badge
            colour="var(--gold)"
            background="var(--gold-tint)"
            title={`Another app claimed ${app.slug}.local — served as ${app.hostname}`}
          >
            name taken
          </Badge>
        )}
      </div>
    </article>
  );
}

function Badge({
  children,
  colour,
  background,
  border,
  title,
}: {
  children: React.ReactNode;
  colour: string;
  background: string;
  border?: string;
  title?: string;
}) {
  return (
    <span
      title={title}
      style={{
        font: "600 10.5px var(--font-mono)",
        color: colour,
        background,
        border: border ? `1px solid ${border}` : undefined,
        padding: border ? "3px 7px" : "4px 9px",
        borderRadius: 6,
      }}
    >
      {children}
    </span>
  );
}

function NewAppTile({ onClick }: { onClick?: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        border: "2px dashed var(--dash)",
        borderRadius: 14,
        padding: 18,
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 8,
        cursor: "pointer",
        background: "transparent",
        color: "var(--muted)",
        minHeight: 118,
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 34,
          height: 34,
          borderRadius: 10,
          background: "var(--raised)",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          fontSize: 18,
        }}
      >
        ＋
      </span>
      <span style={{ font: "600 13px var(--font-sans)" }}>
        Drop a folder to add an app
      </span>
    </button>
  );
}

/**
 * The zero state.
 *
 * The mockup makes this a real screen rather than a shrug, because it is what
 * someone sees for the whole gap between installing and their first folder.
 */
function EmptyWorkspace({
  workspace,
  onNewApp,
}: {
  workspace: string;
  onNewApp?: () => void;
}) {
  return (
    <div
      style={{
        border: "2px dashed var(--dash)",
        borderRadius: 16,
        padding: "48px 28px",
        textAlign: "center",
        color: "var(--ink3)",
      }}
    >
      <h2
        style={{
          margin: "0 0 8px",
          font: "800 20px var(--font-sans)",
          color: "var(--ink)",
          letterSpacing: "-0.02em",
        }}
      >
        Set the table
      </h2>
      <p
        style={{
          margin: "0 auto 20px",
          maxWidth: 420,
          font: "400 13.5px/1.55 var(--font-sans)",
        }}
      >
        Your workspace is watching{" "}
        <code style={{ fontFamily: "var(--font-mono)" }}>{workspace}</code>. Drop
        any folder in and it becomes a live app in seconds — no config, no build
        step.
      </p>
      <button
        type="button"
        onClick={onNewApp}
        style={{
          border: "none",
          borderRadius: 10,
          padding: "11px 22px",
          background: "var(--accent)",
          color: "#fff",
          font: "600 13.5px var(--font-sans)",
          cursor: "pointer",
        }}
      >
        Add your first app
      </button>
      <p style={{ margin: "16px 0 0", font: "400 12px var(--font-sans)", color: "var(--faint)" }}>
        or run <code style={{ fontFamily: "var(--font-mono)" }}>kt deploy .</code>{" "}
        from any folder
      </p>
    </div>
  );
}
