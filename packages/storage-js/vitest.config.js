import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // The client is a browser thing: it reads `location`, `localStorage` and
    // `window`, and the fallback path is most of what these tests exercise.
    environment: "jsdom",
    // A page inside an app, so `location.pathname` has the shape the local
    // namespace is derived from.
    environmentOptions: { jsdom: { url: "http://trip-planner.local/" } },
  },
});
