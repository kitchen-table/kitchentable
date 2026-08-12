import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Disconnected } from "./Disconnected";
import { Library } from "./Library";
import { Sharing } from "./Sharing";
import { StatusBar } from "./StatusBar";
import { Onboarding } from "./onboarding/Onboarding";
import { isComplete } from "./onboarding/steps";
import { useApps, useHealth, useStatus } from "./daemon";

const client = new QueryClient();

export function App() {
  return (
    <QueryClientProvider client={client}>
      <Window />
    </QueryClientProvider>
  );
}

/**
 * The window has exactly two shapes: connected to a daemon, or not.
 *
 * "Not" is a real, designed state rather than a spinner, because it is what
 * someone sees if the daemon crashes or the app was just updated - and it is
 * the only screen that can explain what to do about it.
 */
function Window() {
  const health = useHealth();
  const ready = health.data?.state === "ready";

  const apps = useApps(ready);
  const status = useStatus(ready);
  const [sharing, setSharing] = useState<string | null>(null);
  const [onboarded, setOnboarded] = useState(isComplete);

  // Resolve from the live list rather than holding a copy, so the panel keeps
  // showing the current visibility after a change.
  const shared = (apps.data ?? []).find((a) => a.slug === sharing);

  if (health.isLoading) {
    return <Centred>Starting…</Centred>;
  }

  if (!ready) {
    return <Disconnected health={health.data} onRetry={() => health.refetch()} />;
  }

  // First run, once there is a daemon to show real state from. Running it
  // before the daemon is up would mean an onboarding flow full of blanks.
  if (!onboarded) {
    return (
      <Onboarding
        apps={apps.data ?? []}
        status={status.data}
        onDone={() => setOnboarded(true)}
      />
    );
  }

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <main style={{ flex: 1, minHeight: 0 }}>
        {shared ? (
          <Sharing app={shared} onClose={() => setSharing(null)} />
        ) : (
          <Library
            apps={apps.data ?? []}
            workspace={status.data?.workspace ?? "…"}
            onShare={(app) => setSharing(app.slug)}
          />
        )}
      </main>
      <StatusBar status={status.data} />
    </div>
  );
}

function Centred({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        font: "400 14px var(--font-sans)",
        color: "var(--ink3)",
      }}
    >
      {children}
    </div>
  );
}
