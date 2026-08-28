// SvelteKit route parameter matcher for UUIDs
// Must export a `match` function per SvelteKit conventions

export function match(param: string): boolean {
  // UUID v4 regex: 8-4-4-4-12 hex characters, case insensitive
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(param);
}
