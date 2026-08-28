import type { Actions } from "./$types";
import { emailRequestAction, loadTurnstile } from "$lib/actions/handleAuthAction";

export const actions: Actions = {
  default: emailRequestAction("/auth/magic-link/request"),
};

export const load = loadTurnstile;
