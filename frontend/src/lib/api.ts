// Typed REST client for the Arena server.
//
// Auth strategy (dual-mode):
//   SSR  — SvelteKit server-side loads pass their own `fetch` via `opts.fetch`;
//          the HttpOnly access cookie is forwarded automatically.
//   Browser — no `opts.fetch` is provided; `auth.ts` supplies the
//             localStorage token as an `Authorization: Bearer` header.
//             On a 401 the client performs one silent refresh then retries.
//
// All non-2xx responses raise `ApiError` carrying the upstream HTTP
// status and the parsed JSON body (when present). Callers should pattern
// match on `status` rather than inspect message text.
//
// This file is a re-export barrel; the implementation lives in ./api/*.

export { ApiError } from "./api/errors";
export * from "./api/types";
export * from "./api/accounts";
export * from "./api/sessions";
export * from "./api/projects";
export * from "./api/users";
export * from "./api/email";
export * from "./api/judges";
export * from "./api/llm";
export * from "./api/analytics";
