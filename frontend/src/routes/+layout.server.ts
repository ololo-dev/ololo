import { getMe } from "$lib/api";
import type { LayoutServerLoad } from "./$types";

export const load: LayoutServerLoad = async ({ locals, fetch, depends }) => {
  depends("app:user");
  if (!locals.isAuthenticated) {
    return {
      isAuthenticated: false,
      isAdmin: false,
      user: null,
      allowProjectCreation: false,
      replayEnabled: false,
    };
  }
  try {
    const me = await getMe({ fetch });
    const nameParts = me.display_name.trim().split(/\s+/);
    const initials =
      nameParts.length >= 2
        ? (nameParts[0][0] + nameParts[nameParts.length - 1][0]).toUpperCase()
        : me.display_name.slice(0, 2).toUpperCase();
    return {
      isAuthenticated: true,
      isAdmin: me.is_admin,
      allowProjectCreation: me.allow_project_creation ?? false,
      // Absent on an older server: treat as on, which is what it was.
      replayEnabled: me.session_replay_enabled ?? true,
      user: {
        id: me.id,
        name: me.display_name,
        initials,
        avatarUrl: me.avatar_url ?? undefined,
        username: me.username ?? undefined,
      },
    };
  } catch {
    // DB failure or 401 degrades gracefully — isAdmin defaults false.
    return {
      isAuthenticated: locals.isAuthenticated,
      isAdmin: false,
      user: null,
      allowProjectCreation: false,
      replayEnabled: false,
    };
  }
};
