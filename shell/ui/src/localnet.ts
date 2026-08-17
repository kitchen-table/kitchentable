/**
 * The macOS Local Network permission, as onboarding uses it.
 *
 * There is one thing worth understanding here, and it decides the shape of the
 * whole screen: **checking is asking**. macOS has no call that reports this
 * permission and no call that requests it. The prompt appears when an app first
 * touches the local network, so any check we make is itself the request.
 *
 * That is why this never runs on its own. A query that fired on mount would
 * throw the dialog at somebody mid-sentence, before the screen that explains
 * what it is for had been read - which is exactly the reflexive "Don't Allow"
 * that onboarding exists to prevent. It runs when the button is pressed.
 *
 * It also means "we have not asked" is a real state and cannot be skipped past.
 * Arriving on the step, we genuinely do not know, and the screen says so by
 * offering the button rather than claiming an answer.
 */
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";

export interface LocalNetwork {
  /**
   * `allowed` - a device answered us, so packets really are crossing.
   * `denied` - macOS refused the send outright.
   * `unknown` - nothing answered and nothing refused. Common on a quiet
   * network, and deliberately not read as a refusal.
   * `not_applicable` - not macOS; there is no such permission to hold.
   */
  state: "allowed" | "denied" | "unknown" | "not_applicable";
}

export function useLocalNetwork() {
  return useQuery({
    queryKey: ["local-network"],
    queryFn: () => invoke<LocalNetwork>("kt_local_network"),
    // Never on its own; see above.
    enabled: false,
    // Asking twice is not free here - the first send is what raises a system
    // dialog - so nothing about focus or staleness may trigger it.
    refetchOnWindowFocus: false,
    refetchOnMount: false,
    refetchOnReconnect: false,
    staleTime: Infinity,
    retry: false,
  });
}
