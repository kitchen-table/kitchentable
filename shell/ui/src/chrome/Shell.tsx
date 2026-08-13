import { useMemo, useState } from "react";
import type { App, SysStatus, Visibility } from "../generated";
import { Library } from "../Library";
import { StatusBar } from "../StatusBar";
import { quit } from "../daemon";
import { LIBRARY, type Surface } from "../navigation";
import { useAppearance } from "../theme";
import { NewAppModal } from "./NewAppModal";
import { Sidebar, type McpStatus } from "./Sidebar";
import { TitleBar } from "./TitleBar";

/**
 * The window frame every surface lives in: title bar, rail, content, status.
 *
 * It owns exactly three pieces of state - where you are, what you searched for,
 * and how it looks - and nothing that the daemon owns. Everything else is a
 * query.
 */
export function Shell({
  apps,
  status,
  mcp,
  pending,
}: {
  apps: App[];
  status?: SysStatus;
  mcp?: McpStatus;
  /** Devices waiting on a decision, for the title bar badge. */
  pending: number;
}) {
  const [surface, setSurface] = useState<Surface>(LIBRARY);
  const [query, setQuery] = useState("");
  const [newApp, setNewApp] = useState(false);
  const { dark, toggle } = useAppearance();

  const counts = useMemo(() => {
    const out: Partial<Record<Visibility, number>> = {};
    for (const app of apps) {
      out[app.visibility] = (out[app.visibility] ?? 0) + 1;
    }
    return out;
  }, [apps]);

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <TitleBar
        query={query}
        onQuery={setQuery}
        pending={pending}
        onPending={() => setSurface({ kind: "activity" })}
        dark={dark}
        onToggleTheme={toggle}
        onNewApp={() => setNewApp(true)}
      />

      <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
        <Sidebar
          surface={surface}
          onNavigate={setSurface}
          total={apps.length}
          counts={counts}
          mcp={mcp}
          onQuit={() => void quit()}
        />
        <main style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
          <Content
            surface={surface}
            apps={apps}
            status={status}
            query={query}
            onNavigate={setSurface}
            onNewApp={() => setNewApp(true)}
          />
        </main>
      </div>

      <StatusBar status={status} />

      {newApp && <NewAppModal workspace={status?.workspace} onClose={() => setNewApp(false)} />}
    </div>
  );
}

function Content({
  surface,
  apps,
  status,
  query,
  onNavigate,
  onNewApp,
}: {
  surface: Surface;
  apps: App[];
  status?: SysStatus;
  query: string;
  onNavigate: (surface: Surface) => void;
  onNewApp: () => void;
}) {
  switch (surface.kind) {
    case "library":
      return (
        <Library
          apps={apps}
          filter={surface.filter}
          query={query}
          workspace={status?.workspace ?? "…"}
          onOpen={(app) => onNavigate({ kind: "app", slug: app.slug, tab: "overview" })}
          onNewApp={onNewApp}
        />
      );
    default:
      // Filled in as each surface lands; the rail is already routing to them.
      return <NotBuiltYet surface={surface} />;
  }
}

function NotBuiltYet({ surface }: { surface: Surface }) {
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        font: "400 14px var(--font-sans)",
        color: "var(--faint)",
      }}
    >
      {surface.kind} is not built yet.
    </div>
  );
}
