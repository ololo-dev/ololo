import { redirect, fail } from "@sveltejs/kit";
import type { Actions, PageServerLoad } from "./$types";
import { handleAuthAction } from "$lib/actions/handleAuthAction";

export const load: PageServerLoad = async ({ locals, url }) => {
  const cliToken = url.searchParams.get("cli_token");

  if (!cliToken) {
    throw redirect(303, "/?auth_error=invalid_state");
  }

  return {
    cliToken,
    authenticated: locals.isAuthenticated ?? false,
  };
};

export const actions: Actions = {
  confirm: async ({ locals, fetch, url }) => {
    if (!locals.isAuthenticated) {
      return fail(401, { error: "not_authenticated" });
    }

    const cliToken = url.searchParams.get("cli_token") ?? "";

    const upstream = await fetch("/auth/cli/confirm", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ cli_token: cliToken }),
    });

    if (upstream.status === 410) {
      return fail(410, { error: "session_expired" });
    }
    if (!upstream.ok) {
      return fail(500, { error: "confirm_failed" });
    }

    return { confirmed: true };
  },

  login: async ({ request, fetch, cookies, url }) => {
    const cliToken = url.searchParams.get("cli_token") ?? "";
    const data = await request.formData();
    const email = String(data.get("email") ?? "").trim();
    const password = String(data.get("password") ?? "");

    if (!email || !password) {
      return fail(422, { error: "missing_fields", email });
    }

    return handleAuthAction({
      fetch,
      cookies,
      endpoint: "/auth/login",
      body: { email, password },
      forwardCookies: true,
      failFields: { email },
      defaultError: "login_failed",
      parseErrorFromJson: false,
      next: `/cli-auth?cli_token=${encodeURIComponent(cliToken)}`,
    });
  },
};
