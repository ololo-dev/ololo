import type { Actions } from "./$types";
import { fail } from "@sveltejs/kit";
import { handleAuthAction, loadTurnstile } from "$lib/actions/handleAuthAction";

export const actions: Actions = {
  default: async ({ request, fetch }) => {
    const data = await request.formData();
    const token = String(data.get("token") ?? "");
    const newPassword = String(data.get("new_password") ?? "");
    const turnstileToken = String(data.get("turnstile_token") ?? "");
    if (!token || !newPassword) {
      return fail(422, { error: "missing_fields" });
    }
    const body: Record<string, string> = { token, new_password: newPassword };
    if (turnstileToken) body.turnstile_token = turnstileToken;
    return handleAuthAction({
      fetch,
      endpoint: "/auth/reset-password",
      body,
      defaultError: "reset_failed",
      returnOnSuccess: { success: true },
    });
  },
};

export const load = loadTurnstile;
