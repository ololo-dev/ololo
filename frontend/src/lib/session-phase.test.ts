import { describe, expect, it } from "vitest";
import { backendPhaseToUiPhase, sessionStateToUiPhase } from "./session-phase";

describe("session phase mapping", () => {
  it("maps backend running phase to active ui phase", () => {
    expect(backendPhaseToUiPhase("running")).toBe("active");
  });

  it("maps terminal backend phases to finished ui phase", () => {
    expect(backendPhaseToUiPhase("finished")).toBe("finished");
    expect(backendPhaseToUiPhase("cancelled")).toBe("finished");
  });

  it("maps paused backend phase to paused ui phase", () => {
    expect(backendPhaseToUiPhase("paused")).toBe("paused");
  });

  it("maps lobby-like values to lobby", () => {
    expect(backendPhaseToUiPhase("lobby")).toBe("lobby");
    expect(backendPhaseToUiPhase("unknown")).toBe("lobby");
  });

  it("maps session state values consistently", () => {
    expect(sessionStateToUiPhase("running")).toBe("active");
    expect(sessionStateToUiPhase("finished")).toBe("finished");
    expect(sessionStateToUiPhase("lobby")).toBe("lobby");
    expect(sessionStateToUiPhase("paused")).toBe("paused");
  });
});
