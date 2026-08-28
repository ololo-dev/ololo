import type { Actions } from "./$types";
import { fail } from "@sveltejs/kit";
import { handleAuthAction } from "$lib/actions/handleAuthAction";

export const actions: Actions = {
  default: async ({ request, fetch, cookies, url }) => {
    const data = await request.formData();
    const email = String(data.get("email") ?? "").trim();
    const password = String(data.get("password") ?? "");
    const turnstileToken = String(data.get("turnstile_token") ?? "");
    if (!email || !password) {
      return fail(422, { error: "missing_fields", email });
    }
    const body: Record<string, string> = { email, password };
    if (turnstileToken) body.turnstile_token = turnstileToken;
    const next = url.searchParams.get("next") ?? "/";
    return handleAuthAction({
      fetch,
      cookies,
      endpoint: "/auth/login",
      body,
      forwardCookies: true,
      failFields: { email },
      defaultError: "login_failed",
      next: next.startsWith("/") ? next : "/",
    });
  },
};
