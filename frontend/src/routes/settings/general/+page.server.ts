import { getSettings, updateSetting, ApiError } from "$lib/api";
import type { PageServerLoad, Actions } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const settings = await getSettings({ fetch });
  const allowUserProjectCreation = settings.allow_user_project_creation === "true";
  // Never configured means on: the switch exists to take the replay away,
  // not to have to hand it out.
  const sessionReplayEnabled = settings.session_replay_enabled !== "false";
  return { settings, allowUserProjectCreation, sessionReplayEnabled };
};

export const actions: Actions = {
  updateSessionReplay: async ({ request, fetch }) => {
    const data = await request.formData();
    const value = String(data.get("replay_enabled") ?? "true");
    if (value !== "true" && value !== "false") {
      return { success: false, error: "invalid_value" };
    }
    try {
      await updateSetting("session_replay_enabled", value, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return { success: false, error: err.code ?? "error" };
      }
      return { success: false, error: "unknown" };
    }
  },

  updateProjectCreation: async ({ request, fetch }) => {
    const data = await request.formData();
    const value = String(data.get("allow_creation") ?? "false");
    if (value !== "true" && value !== "false") {
      return { success: false, error: "invalid_value" };
    }
    try {
      await updateSetting("allow_user_project_creation", value, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return { success: false, error: err.code ?? "error" };
      }
      return { success: false, error: "unknown" };
    }
  },
};
