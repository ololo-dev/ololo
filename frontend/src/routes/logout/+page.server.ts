import type { Actions } from "./$types";
import { handleAuthAction } from "$lib/actions/handleAuthAction";

export const actions: Actions = {
  default: async ({ fetch, cookies }) => {
    return handleAuthAction({
      fetch,
      cookies,
      endpoint: "/auth/logout",
      method: "POST",
      forwardCookies: true,
      next: "/",
      redirectOnFailure: true,
      beforeRedirect: () => {
        // Defensive: ensure the gate cookie is removed even if the upstream
        // failed (network blip, expired session). Other cookies the upstream
        // would have cleared are left to it.
        cookies.delete("arena_access", { path: "/" });
        cookies.delete("arena_refresh", { path: "/" });
      },
    });
  },
};
