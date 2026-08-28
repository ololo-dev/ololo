import type { PageServerLoad } from "./$types";
import { redirect } from "@sveltejs/kit";

// The dedicated invite page was removed: the public session dashboard
// (/s/<code>) already shows the project, status, roster and the
// `ololo join <code>` command an invitee needs, so a separate page added
// nothing. This route now just forwards any old invite link there.
export const load: PageServerLoad = ({ params }) => {
  throw redirect(308, `/s/${params.code}`);
};
