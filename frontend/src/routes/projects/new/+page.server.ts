import type { PageServerLoad, Actions } from "./$types";
import { redirect, error } from "@sveltejs/kit";
import {
  createPersonalProject,
  getPersonalProject,
  getPersonalProjectOptions,
  ApiError,
} from "$lib/api";
import {
  parsePersonalForm,
  personalFailure,
  type PersonalFormValues,
} from "$lib/server/personal-project-form";

// A new personal project: the user's own work, played in their own
// repository. `?from=<project id>` starts from one of theirs (duplicate).
export const load: PageServerLoad = async ({ locals, fetch, parent, url }) => {
  if (!locals.isAuthenticated) {
    throw redirect(303, `/login?next=${encodeURIComponent(url.pathname + url.search)}`);
  }
  const { isAdmin, allowProjectCreation } = await parent();
  if (!isAdmin && !allowProjectCreation) {
    throw error(403, "Project creation is not enabled for your account.");
  }
  const options = await getPersonalProjectOptions({ fetch });
  let initial: PersonalFormValues | null = null;
  const from = url.searchParams.get("from");
  if (from) {
    try {
      const { spec, project } = await getPersonalProject(from, { fetch });
      initial = {
        name: `${spec.name} (copy)`.slice(0, options.limits.max_name_chars),
        description: spec.description,
        tasks: spec.tasks,
        judges: spec.judges,
        session_duration_secs: spec.session_duration_secs,
        // A copy is a new project: private only for whoever may make one.
        public: project.public || !options.private_allowed,
      };
    } catch {
      // Not theirs, or gone: start from an empty form.
    }
  }
  return { options, initial };
};

export const actions: Actions = {
  default: async ({ request, fetch }) => {
    const { request: body, values } = parsePersonalForm(await request.formData());
    try {
      const project = await createPersonalProject(body, { fetch });
      throw redirect(303, `/projects/${project.id}?message=created`);
    } catch (err) {
      if (err instanceof ApiError) return personalFailure(err, values);
      throw err;
    }
  },
};
