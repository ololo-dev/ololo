import type { Actions } from "./$types";
import { fail } from "@sveltejs/kit";
import { handleAuthAction } from "$lib/actions/handleAuthAction";

export const actions: Actions = {
  default: async ({ request, fetch, cookies }) => {
    const data = await request.formData();
    const email = String(data.get("email") ?? "").trim();
    const password = String(data.get("password") ?? "");
    const display_name = String(data.get("display_name") ?? "").trim();
    const turnstileToken = String(data.get("turnstile_token") ?? "");
    if (!email || !password || !display_name) {
      return fail(422, { error: "missing_fields", email, display_name });
    }
    const body: Record<string, string | boolean> = { email, password, display_name };
    if (turnstileToken) body.turnstile_token = turnstileToken;
    return handleAuthAction({
      fetch,
      cookies,
      endpoint: "/auth/register",
      body,
      forwardCookies: true,
      failFields: { email, display_name },
      defaultError: "register_failed",
      next: "/",
    });
  },
};
