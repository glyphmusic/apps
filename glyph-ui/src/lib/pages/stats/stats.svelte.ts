import type { StatsRange } from "./types";

/** Shared UI state for the Stats page (kept across navigation and URL sync). */
export const statsState = $state<{ range: StatsRange; recap: string | null }>({
  range: "month",
  recap: null,
});
