export type UiSessionPhase = "lobby" | "active" | "paused" | "finished";

export function backendPhaseToUiPhase(phase: string): UiSessionPhase {
  if (phase === "running" || phase === "active") return "active";
  if (phase === "paused") return "paused";
  if (phase === "finished" || phase === "cancelled") return "finished";
  return "lobby";
}

export function sessionStateToUiPhase(state: string): UiSessionPhase {
  return backendPhaseToUiPhase(state);
}
