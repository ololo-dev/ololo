import type { PageServerLoad, Actions } from "./$types";
import { redirect, error, fail } from "@sveltejs/kit";
import {
  getPersonalProject,
  getPersonalProjectOptions,
  updatePersonalProject,
  patchProject,
  ApiError,
} from "$lib/api";
import { parsePersonalForm, personalFailure } from "$lib/server/personal-project-form";

// Edit a personal project — rebuild its tasks from new words — until its
// first session freezes them; after that the page offers a duplicate.
export const load: PageServerLoad = async ({ params, locals, fetch }) => {
  if (!locals.isAuthenticated) {
    throw redirect(303, `/login?next=/projects/${params.id}/personal`);
  }
  try {
    const [detail, options] = await Promise.all([
      getPersonalProject(params.id, { fetch }),
      getPersonalProjectOptions({ fetch }),
    ]);
    if (detail.project.owner_user_id !== locals.userId) {
      throw error(403, "Only the owner edits a personal project.");
    }
    return { detail, options };
  } catch (err) {
    if (err instanceof ApiError && err.status === 404) throw error(404, "Project not found");
    throw err;
  }
};

export const actions: Actions = {
  // Who sees it is not part of the tasks: it moves after the first session
  // too, when the rest of the project is frozen.
  visibility: async ({ params, request, fetch }) => {
    const data = await request.formData();
    const isPublic = String(data.get("public") ?? "true") !== "false";
    try {
      await patchProject(params.id, { public: isPublic }, { fetch });
      return { visibility: isPublic ? "public" : "private" };
    } catch (err) {
      if (err instanceof ApiError) {
        return fail(err.status, { visibilityError: err.code ?? "error" });
      }
      throw err;
    }
  },
  default: async ({ params, request, fetch }) => {
    const { request: body, values } = parsePersonalForm(await request.formData());
    try {
      await updatePersonalProject(params.id, body, { fetch });
      throw redirect(303, `/projects/${params.id}?message=updated`);
    } catch (err) {
      if (err instanceof ApiError) return personalFailure(err, values);
      throw err;
    }
  },
};
