import type { Actions } from "./$types";
import { emailRequestAction, loadTurnstile } from "$lib/actions/handleAuthAction";

export const actions: Actions = {
  default: emailRequestAction("/auth/forgot-password"),
};

export const load = loadTurnstile;
