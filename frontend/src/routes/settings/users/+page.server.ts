import { getAdminUsers } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const users = await getAdminUsers({ fetch });
  return { users };
};
