import type { PageServerLoad, Actions } from "./$types";
import { fail, redirect, error } from "@sveltejs/kit";
import { createProject, getProjectCategories, ApiError } from "$lib/api";
import { buildSessionDurationFromForm } from "$lib/server/project-form";

export const load: PageServerLoad = async ({ locals, fetch, parent }) => {
  if (!locals.isAuthenticated) {
    throw redirect(303, "/login?next=/projects/new");
  }
  const { isAdmin, allowProjectCreation } = await parent();
  if (!isAdmin && !allowProjectCreation) {
    throw error(403, "Project creation is not enabled for your account.");
  }
  let categories: string[] = [];
  try {
    categories = await getProjectCategories({ fetch });
  } catch {
    // graceful degradation — categories optional
  }
  return { categories };
};

export const actions: Actions = {
  default: async ({ request, fetch }) => {
    const data = await request.formData();
    const name = String(data.get("name") ?? "").trim();
    const description = String(data.get("description") ?? "").trim();
    const isPublic = data.get("public") === "true";
    const category = String(data.get("category") ?? "").trim() || undefined;
    const tags_json = String(data.get("tags_json") ?? "[]");
    let tags: string[] | undefined;
    try {
      tags = JSON.parse(tags_json) as string[];
    } catch {
      tags = [];
    }
    const cover_image_url = String(data.get("cover_image_url") ?? "").trim() || undefined;

    // Points defaults (admin only). Empty inputs serialize as null.
    const pv = String(data.get("points_value") ?? "").trim();
    const pf = String(data.get("points_fail") ?? "").trim();
    const pnr = String(data.get("points_no_response") ?? "").trim();
    const pcb = String(data.get("points_completion_bonus") ?? "").trim();
    const points =
      pv || pf || pnr || pcb
        ? {
            value: pv ? Number(pv) : null,
            fail: pf ? Number(pf) : null,
            no_response: pnr ? Number(pnr) : null,
            completion_bonus: pcb ? Number(pcb) : null,
          }
        : undefined;

    // Intervals defaults (admin only). Empty inputs serialize as null.
    const idl = String(data.get("intervals_deadline_secs") ?? "").trim();
    const imin = String(data.get("intervals_min_interval_secs") ?? "").trim();
    const iinc = String(data.get("intervals_interval_increment_secs") ?? "").trim();
    const imax = String(data.get("intervals_max_interval_secs") ?? "").trim();
    const intervals =
      idl || imin || iinc || imax
        ? {
            deadline_secs: idl ? Number(idl) : null,
            min_interval_secs: imin ? Number(imin) : null,
            interval_increment_secs: iinc ? Number(iinc) : null,
            max_interval_secs: imax ? Number(imax) : null,
          }
        : undefined;

    // Session duration default (admin only). Empty input → omit (server default).
    const session_duration_secs = buildSessionDurationFromForm(data);

    if (name.length < 1 || name.length > 200) {
      return fail(422, {
        error: "invalid_name",
        name,
        description,
        public: String(isPublic),
      });
    }

    try {
      const project = await createProject(
        {
          name,
          description,
          public: isPublic,
          category,
          tags,
          cover_image_url,
          points,
          intervals,
          session_duration_secs,
        },
        { fetch },
      );
      throw redirect(303, `/projects/${project.id}`);
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, {
          error: err.code ?? "error",
          name,
          description,
          public: String(isPublic),
        });
      }
      throw err;
    }
  },
};
