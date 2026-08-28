import { getCategories, createCategory, deleteCategory, ApiError } from "$lib/api";
import type { PageServerLoad, Actions } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const categories = await getCategories({ fetch });
  return { categories };
};

export const actions: Actions = {
  addCategory: async ({ request, fetch }) => {
    const data = await request.formData();
    const name = String(data.get("name") ?? "").trim();
    if (!name) {
      return { success: false, error: "invalid_name" };
    }
    try {
      await createCategory(name, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return { success: false, error: err.code ?? "error" };
      }
      return { success: false, error: "unknown" };
    }
  },

  deleteCategory: async ({ request, fetch }) => {
    const data = await request.formData();
    const id = Number(data.get("id"));
    if (!id) {
      return { success: false, error: "invalid_id" };
    }
    try {
      await deleteCategory(id, { fetch });
      return { success: true };
    } catch (err) {
      if (err instanceof ApiError) {
        return { success: false, error: err.code ?? "error" };
      }
      return { success: false, error: "unknown" };
    }
  },
};
