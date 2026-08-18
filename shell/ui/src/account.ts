/**
 * The install's link to an account, and the one way to start one.
 *
 * The whole ceremony happens in a browser: `account.begin_upgrade` answers
 * with the cloud's upgrade page carrying this install's key, Stripe collects
 * the email and the card there, and the daemon notices the link landing by
 * itself. This module is only the window's view of that - a query for where
 * things stand, and a mutation that opens the page.
 */
import { useMutation, useQuery } from "@tanstack/react-query";
import { call } from "./daemon";
import { hasTauri, openExternal } from "./external";
import type { AccountStatus } from "./generated";

/**
 * Slow, because `account_changed` events carry the transitions; quick while an
 * upgrade is in flight, because the events need a Tauri IPC and the plain-
 * browser dev build has none - there the poll is what notices the link land.
 */
export function useAccount(enabled: boolean) {
  return useQuery({
    queryKey: ["account"],
    queryFn: () => call<AccountStatus>("account.status"),
    refetchInterval: (query) =>
      query.state.data?.link?.state === "waiting" ? 3_000 : 60_000,
    retry: false,
    enabled,
  });
}

/**
 * Start the upgrade and put the page in front of the person.
 *
 * One mutation for both steps because neither is useful alone: a URL nobody
 * opens links nothing, and a browser opened with no URL is not a thing. Safe
 * to fire twice - the daemon reuses the same URL rather than stacking a
 * second checkout.
 */
export function useBeginUpgrade() {
  return useMutation({
    mutationFn: async () => {
      const started = await call<{ url: string }>("account.begin_upgrade");
      // The desktop app hands the URL to the OS; the plain-browser dev build
      // has no IPC to hand it to, and there a tab is the right shape anyway.
      if (hasTauri()) await openExternal(started.url);
      else window.open(started.url, "_blank", "noopener");
      return started;
    },
  });
}
