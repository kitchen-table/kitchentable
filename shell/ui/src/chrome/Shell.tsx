import { useCallback, useMemo, useState } from "react";
import type { App, SysStatus, Visibility } from "../generated";
import { Library } from "../Library";
import { StatusBar } from "../StatusBar";
import { AppDetail } from "../surfaces/AppDetail";
import { quit } from "../daemon";
import { useAddApp, useFolderDrop } from "../addApp";
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
  const [dropError, setDropError] = useState<string | null>(null);
  const { dark, toggle } = useAppearance();
  const { create, importFolder, pick } = useAddApp();

  /**
   * A folder dropped anywhere on the window is an app, whichever surface is
   * showing. Several at once are several apps - dragging a selection is a
   * normal thing to do, and taking only the first would silently lose the rest.
   */
  const onDrop = useCallback(
    async (paths: string[]) => {
      setDropError(null);
      const failures: string[] = [];
      for (const path of paths) {
        try {
          await importFolder(path);
        } catch (raw) {
          failures.push(
            typeof raw === "object" && raw !== null && "message" in raw
              ? String((raw as { message: unknown }).message)
              : String(raw),
          );
        }
      }
      if (failures.length > 0) setDropError(failures.join(" · "));
    },
    [importFolder],
  );

  const dropping = useFolderDrop(onDrop) === "working";

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

      {dropping && !newApp && <DropOverlay />}
      {dropError && (
        <Toast onDismiss={() => setDropError(null)}>{dropError}</Toast>
      )}

      {newApp && (
        <NewAppModal
          workspace={status?.workspace}
          dropping={dropping}
          onCreate={create}
          onPick={pick}
          onClose={() => setNewApp(false)}
        />
      )}
    </div>
  );
}

/** Shown while a dropped folder is being copied in, if no dialog is up. */
function DropOverlay() {
  return (
    <div
      role="status"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 90,
        background: "rgba(28,25,21,.32)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        animation: "kt-fade .16s ease-out",
      }}
    >
      <div
        style={{
          background: "var(--paper)",
          border: "1px solid var(--border2)",
          borderRadius: 14,
          padding: "18px 26px",
          font: "600 14px var(--font-sans)",
          boxShadow: "var(--shadow-lift)",
        }}
      >
        Copying it in…
      </div>
    </div>
  );
}

function Toast({
  children,
  onDismiss,
}: {
  children: React.ReactNode;
  onDismiss: () => void;
}) {
  return (
    <div
      role="alert"
      style={{
        position: "fixed",
        left: "50%",
        bottom: 54,
        transform: "translateX(-50%)",
        zIndex: 95,
        maxWidth: "min(560px, calc(100% - 48px))",
        display: "flex",
        alignItems: "center",
        gap: 14,
        background: "var(--paper)",
        border: "1px solid var(--danger)",
        borderRadius: 11,
        padding: "12px 14px",
        boxShadow: "var(--shadow-lift)",
        animation: "kt-fade .16s ease-out",
      }}
    >
      <span style={{ font: "500 12.5px/1.45 var(--font-sans)", color: "var(--ink2)" }}>
        {children}
      </span>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss"
        style={{
          marginLeft: "auto",
          border: "none",
          background: "none",
          cursor: "pointer",
          font: "600 12.5px var(--font-sans)",
          color: "var(--accent)",
        }}
      >
        Dismiss
      </button>
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
  const library = (filter: Visibility | null) => (
    <Library
      apps={apps}
      filter={filter}
      query={query}
      workspace={status?.workspace ?? "…"}
      onOpen={(app) => onNavigate({ kind: "app", slug: app.slug, tab: "overview" })}
      onNewApp={onNewApp}
    />
  );

  switch (surface.kind) {
    case "library":
      return library(surface.filter);

    case "app": {
      const app = apps.find((candidate) => candidate.slug === surface.slug);
      // The folder was deleted while its detail view was open. Dropping back to
      // the library beats an error page about an app that no longer exists.
      if (!app) return library(null);
      return (
        <AppDetail
          app={app}
          tab={surface.tab}
          onTab={(tab) => onNavigate({ ...surface, tab })}
          onNavigate={onNavigate}
        />
      );
    }

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
