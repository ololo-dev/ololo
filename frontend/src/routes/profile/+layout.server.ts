import { redirect } from "@sveltejs/kit";
import { getMe } from "$lib/api";
import type { LayoutServerLoad } from "./$types";

export const load: LayoutServerLoad = async (event) => {
  const parent_data = await event.parent();
  if (!parent_data.isAuthenticated) {
    throw redirect(307, `/login?next=${event.url.pathname}`);
  }
  const me = await getMe({ fetch: event.fetch });
  return { profile: me };
};
