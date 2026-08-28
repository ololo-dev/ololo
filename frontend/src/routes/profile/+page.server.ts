import {
  listSessions,
  listUserPats,
  revokeUserPat,
  patchMe,
  changePassword,
  ApiError,
} from "$lib/api";
import type { PageServerLoad, Actions } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const [sessions, pats] = await Promise.all([listSessions({ fetch }), listUserPats({ fetch })]);
  return { sessions, pats };
};

export const actions: Actions = {
  revokePat: async ({ request, fetch }) => {
    const data = await request.formData();
    const id = String(data.get("id") ?? "").trim();
    if (!id) return { success: false, error: "invalid_id" };
    try {
      await revokeUserPat(id, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) return { success: false, error: err.code ?? "error" };
      return { success: false, error: "unknown" };
    }
  },

  updateProfile: async ({ request, fetch }) => {
    const data = await request.formData();
    const display_name = String(data.get("display_name") ?? "").trim();
    if (!display_name) return { success: false, error: "invalid_name" };
    try {
      await patchMe({ display_name }, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) return { success: false, error: err.code ?? "error" };
      return { success: false, error: "unknown" };
    }
  },

  updateUsername: async ({ request, fetch }) => {
    const data = await request.formData();
    const username = String(data.get("username") ?? "")
      .trim()
      .toLowerCase();
    if (!username) return { success: false, error: "invalid_username" };
    try {
      await patchMe({ username }, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) return { success: false, error: err.code ?? "error" };
      return { success: false, error: "unknown" };
    }
  },

  changePassword: async ({ request, fetch }) => {
    const data = await request.formData();
    const current_password = String(data.get("current_password") ?? "");
    const new_password = String(data.get("new_password") ?? "");
    if (!current_password || new_password.length < 8) {
      return { success: false, error: "invalid_input" };
    }
    try {
      await changePassword({ current_password, new_password }, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) return { success: false, error: err.code ?? "error" };
      return { success: false, error: "unknown" };
    }
  },
};
