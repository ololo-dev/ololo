import { listProjects } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const projects = await listProjects(true, { fetch });
  return { projects };
};
