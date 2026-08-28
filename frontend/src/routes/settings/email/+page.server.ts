import { getSettings, getEmailTemplates, ApiError, type EmailTemplate } from "$lib/api";
import type { PageServerLoad } from "./$types";

export const load: PageServerLoad = async ({ fetch }) => {
  const settings = await getSettings({ fetch });

  const emailSettings: Record<string, string> = {};
  for (const [k, v] of Object.entries(settings)) {
    if (k.startsWith("email.")) {
      emailSettings[k] = v;
    }
  }

  let emailTemplates: EmailTemplate[] = [];
  try {
    emailTemplates = await getEmailTemplates({ fetch });
  } catch {
    // ignore — endpoint may not exist yet
  }

  return { emailSettings, emailTemplates };
};
