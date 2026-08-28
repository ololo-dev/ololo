/** Top-level sections of the player session page. */
export type SectionTabId = "tasks" | "changes" | "judges" | "stats" | "memory";

export type SectionTab = {
  id: SectionTabId;
  label: string;
  /** Badge count; `null` hides the badge. */
  count: number | null;
};
