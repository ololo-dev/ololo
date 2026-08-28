// Current user, admin users/settings/categories/game-servers, email templates,
// and public profile/session lookups.

import { request, type FetchLike } from "./core";
import { ApiError } from "./errors";
import type {
  Me,
  PatDto,
  SessionReportResponse,
  SessionPlayerStatsResponse,
  ActivityEventDto,
  AvatarAuthResponse,
  AppSettings,
  CategoryDto,
  AdminUserDto,
  CreateAdminUserBody,
  UpdateAdminUserBody,
  GameServerDto,
  EmailTemplate,
  PublicUserProfile,
  PublicSessionsResponse,
} from "./types";

export function getMe(opts: { fetch?: FetchLike } = {}) {
  return request<Me>("/api/users/me", { fetch: opts.fetch });
}

export function getSessionReport(sessionId: string, opts: { fetch?: FetchLike } = {}) {
  return request<SessionReportResponse>(`/api/sessions/${encodeURIComponent(sessionId)}/report`, {
    fetch: opts.fetch,
  });
}

export function getSessionPlayerStats(sessionId: string, opts: { fetch?: FetchLike } = {}) {
  return request<SessionPlayerStatsResponse>(
    `/api/sessions/${encodeURIComponent(sessionId)}/player-stats`,
    { fetch: opts.fetch },
  );
}

export function getSessionActivity(sessionId: string, opts: { fetch?: FetchLike } = {}) {
  return request<{ events: ActivityEventDto[] }>(
    `/api/sessions/${encodeURIComponent(sessionId)}/activity`,
    { fetch: opts.fetch },
  );
}

export function patchMe(
  body: { display_name?: string; avatar_url?: string; username?: string },
  opts: { fetch?: FetchLike } = {},
) {
  return request<Me>("/api/users/me", {
    method: "PATCH",
    body,
    fetch: opts.fetch,
  });
}

export function getAvatarAuth(opts: { fetch?: FetchLike } = {}) {
  return request<AvatarAuthResponse>("/api/users/me/avatar-auth", {
    fetch: opts.fetch,
  });
}

export function changePassword(
  body: { current_password: string; new_password: string },
  opts: { fetch?: FetchLike } = {},
) {
  return request<{ ok: boolean }>("/api/users/me/password", {
    method: "POST",
    body,
    fetch: opts.fetch,
  });
}

export async function listUserPats(opts: { fetch?: FetchLike } = {}): Promise<PatDto[]> {
  const data = await request<{ pats: PatDto[] }>("/api/users/me/pats", {
    fetch: opts.fetch,
  });
  return data.pats;
}

export function revokeUserPat(id: string, opts: { fetch?: FetchLike } = {}) {
  return request<void>(`/api/users/me/pats/${encodeURIComponent(id)}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

export function getSettings(opts: { fetch?: FetchLike } = {}) {
  return request<AppSettings>("/api/admin/settings", { fetch: opts.fetch });
}

export function updateSetting(key: string, value: string, opts: { fetch?: FetchLike } = {}) {
  return request<{ ok: boolean }>("/api/admin/settings", {
    method: "PUT",
    body: { key, value },
    fetch: opts.fetch,
  });
}

export function getCategories(opts: { fetch?: FetchLike } = {}): Promise<CategoryDto[]> {
  return request<{ categories: CategoryDto[] }>("/api/categories", {
    fetch: opts.fetch,
  }).then((d) => d.categories);
}

export function createCategory(name: string, opts: { fetch?: FetchLike } = {}) {
  return request<{ category: CategoryDto }>("/api/admin/categories", {
    method: "POST",
    body: { name },
    fetch: opts.fetch,
  });
}

export function renameCategory(id: number, name: string, opts: { fetch?: FetchLike } = {}) {
  return request<{ category: CategoryDto; projects_updated: number }>(
    `/api/admin/categories/${id}`,
    { method: "PATCH", body: { name }, fetch: opts.fetch },
  );
}

export function deleteCategory(id: number, opts: { fetch?: FetchLike } = {}) {
  return request<{ ok: boolean }>(`/api/admin/categories/${id}`, {
    method: "DELETE",
    fetch: opts.fetch,
  });
}

export function reorderCategories(ids: number[], opts: { fetch?: FetchLike } = {}) {
  return request<{ ok: boolean }>("/api/admin/categories/reorder", {
    method: "PATCH",
    body: { ids },
    fetch: opts.fetch,
  });
}

export async function getAdminUsers(opts: { fetch?: FetchLike } = {}): Promise<AdminUserDto[]> {
  const data = await request<{ users: AdminUserDto[] }>("/api/admin/users", {
    fetch: opts.fetch,
  });
  return data.users;
}

export function createAdminUser(body: CreateAdminUserBody) {
  return request<{ user: AdminUserDto }>("/api/admin/users", {
    method: "POST",
    body,
  });
}

export function updateAdminUser(id: string, body: UpdateAdminUserBody) {
  return request<{ user: AdminUserDto }>(`/api/admin/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body,
  });
}

export function deleteAdminUser(id: string) {
  return request<{ ok: boolean }>(`/api/admin/users/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}

export async function getGameServers(opts: { fetch?: FetchLike } = {}): Promise<GameServerDto[]> {
  const data = await request<{ servers: GameServerDto[] }>("/api/admin/game-servers", {
    fetch: opts.fetch,
  });
  return data.servers;
}

export function patchGameServer(
  id: string,
  body: {
    display_name?: string;
    status?: string;
    clear_display_name?: boolean;
  },
) {
  return request<{ server: GameServerDto }>(`/api/admin/game-servers/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body,
  });
}

export async function getEmailTemplates(
  opts: { fetch?: FetchLike } = {},
): Promise<EmailTemplate[]> {
  return request<EmailTemplate[]>("/api/admin/email-templates", {
    fetch: opts.fetch,
  });
}

export function updateEmailTemplate(
  type: string,
  data: { subject: string; body_html: string; body_text: string },
  opts: { fetch?: FetchLike } = {},
) {
  return request<void>(`/api/admin/email-templates/${encodeURIComponent(type)}`, {
    method: "PUT",
    body: data,
    fetch: opts.fetch,
  });
}

export function updateEmailSetting(key: string, value: string, opts: { fetch?: FetchLike } = {}) {
  return request<void>("/api/admin/settings", {
    method: "PUT",
    body: { key, value },
    fetch: opts.fetch,
  });
}

/// Anonymous (credentials-omitted) GET that maps error bodies to `ApiError`.
async function publicGet<T>(path: string): Promise<T> {
  const resp = await fetch(path, { credentials: "omit" });
  if (!resp.ok) {
    const text = await resp.text();
    let parsed: unknown = null;
    try {
      parsed = JSON.parse(text);
    } catch {
      parsed = text;
    }
    const code =
      parsed && typeof parsed === "object" && parsed !== null && "error" in parsed
        ? String((parsed as { error: unknown }).error)
        : null;
    throw new ApiError(resp.status, code, parsed);
  }
  return resp.json();
}

export function getPublicProfile(username: string): Promise<PublicUserProfile> {
  return publicGet(`/api/users/by-username/${encodeURIComponent(username)}`);
}

export function getPublicSessions(
  username: string,
  page = 1,
  perPage = 20,
): Promise<PublicSessionsResponse> {
  const params = new URLSearchParams({ page: String(page), per_page: String(perPage) });
  return publicGet(`/api/users/by-username/${encodeURIComponent(username)}/sessions?${params}`);
}
