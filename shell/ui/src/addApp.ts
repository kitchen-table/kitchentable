import { useCallback, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { App } from "./generated";
import { call } from "./daemon";

/**
 * The three ways a folder becomes an app.
 *
 * All of them end in a socket call, because the daemon owns the workspace: the
 * shell picks the folder and hands over a path, and nothing about what happens
 * to it is decided here (CLAUDE.md rule 2).
 *
 * Dropping a folder into the workspace in Finder still works and always will.
 * These exist so someone who has not learned that yet is not stuck.
 */
export function useAddApp() {
  const queryClient = useQueryClient();

  const refresh = useCallback(
    (created: App) => {
      // The list is polled, but waiting up to two seconds to see the app you
      // just made reads as a failure. Invalidate so it appears at once.
      void queryClient.invalidateQueries({ queryKey: ["apps"] });
      return created;
    },
    [queryClient],
  );

  /** An empty app with a starter page, named by the person making it. */
  const create = useCallback(
    async (name: string) => refresh(await call<App>("app.create", { name })),
    [refresh],
  );

  /** An existing folder, copied in. The original is left where it is. */
  const importFolder = useCallback(
    async (path: string) => refresh(await call<App>("app.import", { path })),
    [refresh],
  );

  /** The native folder picker. Resolves to null when it is cancelled. */
  const pick = useCallback(async (): Promise<App | null> => {
    const chosen = await open({ directory: true, multiple: false });
    if (typeof chosen !== "string") return null;
    return importFolder(chosen);
  }, [importFolder]);

  return { create, importFolder, pick };
}

/** What a drop is doing right now, for the drop target's own styling. */
export type DropState = "idle" | "over" | "working";

/**
 * Folders dragged onto the window from Finder.
 *
 * Uses Tauri's drag-drop event rather than the HTML5 one on purpose: the
 * browser event hands over a `File` with no path, and a folder full of files
 * cannot be reassembled from that. Tauri reports real paths, which is exactly
 * what `app.import` needs.
 */
export function useFolderDrop(onDrop: (paths: string[]) => Promise<unknown>) {
  const [state, setState] = useState<DropState>("idle");

  useEffect(() => {
    let cancelled = false;
    // `onDragDropEvent` resolves to the unlisten function, so unsubscribing has
    // to wait for it - and has to cope with unmounting before it arrives.
    let unlisten: (() => void) | undefined;

    let webview;
    try {
      // Throws synchronously outside a Tauri window - the vitest environment,
      // or a browser during `pnpm dev` - because there are no internals to read
      // off `window`. Dropping is simply absent there.
      webview = getCurrentWebview();
    } catch {
      return;
    }

    void webview
      .onDragDropEvent(async (event) => {
        if (cancelled) return;

        if (event.payload.type === "over") {
          setState((current) => (current === "working" ? current : "over"));
          return;
        }

        if (event.payload.type === "leave") {
          setState((current) => (current === "working" ? current : "idle"));
          return;
        }

        if (event.payload.type === "drop") {
          setState("working");
          try {
            await onDrop(event.payload.paths);
          } finally {
            if (!cancelled) setState("idle");
          }
        }
      })
      .then((off) => {
        if (cancelled) off();
        else unlisten = off;
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [onDrop]);

  return state;
}
