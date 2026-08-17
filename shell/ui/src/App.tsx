import { useEffect, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Disconnected } from "./Disconnected";
import { Shell } from "./chrome/Shell";
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
  const [onboarded, setOnboarded] = useState(isComplete);
  const starting = useStartingGrace(ready);

  if (health.isLoading || starting) {
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

  return <Shell apps={apps.data ?? []} status={status.data} />;
}

/**
 * How long a daemon is allowed to be starting before we call it missing.
 *
 * "Not running" and "not up yet" are different claims and the window used to
 * make the first one the instant a health check failed - which on a first run
 * is simply too early. A daemon creating its database, running migrations and
 * minting keys takes a few seconds, and the shell is already waiting on it with
 * its own backoff.
 *
 * Generous rather than tight: the cost of waiting is a few seconds of
 * "Starting…", and the cost of being hasty is telling somebody their software
 * is broken during the first ten seconds they have ever used it.
 */
const STARTING_GRACE_MS = 8_000;

/**
 * Whether we are still within the benefit of the doubt.
 *
 * Only ever true before the first ready answer. Once the daemon has been up,
 * losing it is real news and the disconnected screen appears at once - that
 * screen exists for a daemon that crashed, and it should not be slowed down to
 * cover a first run.
 */
function useStartingGrace(ready: boolean): boolean {
  const [elapsed, setElapsed] = useState(false);
  const [everReady, setEverReady] = useState(false);

  useEffect(() => {
    const timer = setTimeout(() => setElapsed(true), STARTING_GRACE_MS);
    return () => clearTimeout(timer);
  }, []);

  useEffect(() => {
    if (ready) setEverReady(true);
  }, [ready]);

  return !ready && !everReady && !elapsed;
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
