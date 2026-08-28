import type { TestTemplate } from "$lib/api";

/** Auto-detect `{name}` placeholders in a command template. */
export function extractTemplatePlaceholders(commandTemplate: string) {
  const names = new Set<string>();
  const bytes = commandTemplate;
  let i = 0;
  while (i < bytes.length) {
    if (bytes[i] === "{") {
      if (i + 1 < bytes.length && bytes[i + 1] === "{") {
        i += 2;
        continue;
      }
      let j = i + 1;
      while (j < bytes.length && bytes[j] !== "}") j += 1;
      if (j < bytes.length) {
        const token = bytes.slice(i + 1, j).trim();
        if (/^[A-Za-z0-9_]{1,64}$/.test(token)) {
          names.add(token);
        }
        i = j + 1;
        continue;
      }
    }
    i += 1;
  }
  return Array.from(names).map((name) => ({
    name,
    description: `Auto-detected placeholder '${name}'`,
    required: false,
    secret: false,
  }));
}

/// Build a PointsReq from the four `points_*` form fields. Empty inputs
/// serialize as `null` (inherit on create, preserve on PATCH). Returns
/// `undefined` when no field was provided so the body omits `points` entirely.
export function buildPointsFromForm(data: FormData) {
  const pv = String(data.get("points_value") ?? "").trim();
  const pf = String(data.get("points_fail") ?? "").trim();
  const pnr = String(data.get("points_no_response") ?? "").trim();
  const pcb = String(data.get("points_completion_bonus") ?? "").trim();
  if (!pv && !pf && !pnr && !pcb) return undefined;
  return {
    value: pv ? Number(pv) : null,
    fail: pf ? Number(pf) : null,
    no_response: pnr ? Number(pnr) : null,
    completion_bonus: pcb ? Number(pcb) : null,
  };
}

/// Build an IntervalsReq from the four `intervals_*` form fields. Empty inputs
/// serialize as `null` (inherit on create, preserve on PATCH). Returns
/// `undefined` when no field was provided so the body omits `intervals` entirely.
export function buildIntervalsFromForm(data: FormData) {
  const idl = String(data.get("intervals_deadline_secs") ?? "").trim();
  const imin = String(data.get("intervals_min_interval_secs") ?? "").trim();
  const iinc = String(data.get("intervals_interval_increment_secs") ?? "").trim();
  const imax = String(data.get("intervals_max_interval_secs") ?? "").trim();
  if (!idl && !imin && !iinc && !imax) return undefined;
  return {
    deadline_secs: idl ? Number(idl) : null,
    min_interval_secs: imin ? Number(imin) : null,
    interval_increment_secs: iinc ? Number(iinc) : null,
    max_interval_secs: imax ? Number(imax) : null,
  };
}

/// Parse the `session_duration_secs` form field. Empty input returns
/// `undefined` so the body omits the field entirely (server default on
/// create, preserve on PATCH).
export function buildSessionDurationFromForm(data: FormData) {
  const sd = String(data.get("session_duration_secs") ?? "").trim();
  return sd ? Number(sd) : undefined;
}

export function buildIdleTimeoutFromForm(data: FormData) {
  const it = String(data.get("idle_timeout_secs") ?? "").trim();
  return it ? Number(it) : undefined;
}

/// Build a `memory_schema` object from the `memory_schema_json` form field
/// (a JSON array of `{key, value}` rows). Returns `undefined` when the field
/// is absent (omit → no change), `null` when no key survives (clear the
/// schema), else the flat object the API expects. Values that read as a
/// number or as `true`/`false` are sent as such; everything else as a string.
export function buildMemorySchemaFromForm(
  data: FormData,
): Record<string, string | number | boolean> | null | undefined {
  const raw = data.get("memory_schema_json");
  if (typeof raw !== "string") return undefined;
  let rows: unknown;
  try {
    rows = JSON.parse(raw);
  } catch {
    return undefined;
  }
  if (!Array.isArray(rows)) return undefined;
  const schema: Record<string, string | number | boolean> = {};
  for (const row of rows) {
    if (typeof row !== "object" || row === null) continue;
    const key = String((row as { key?: unknown }).key ?? "").trim();
    if (!key) continue;
    schema[key] = coerceMemoryValue(String((row as { value?: unknown }).value ?? ""));
  }
  return Object.keys(schema).length > 0 ? schema : null;
}

function coerceMemoryValue(value: string): string | number | boolean {
  const trimmed = value.trim();
  if (trimmed === "true") return true;
  if (trimmed === "false") return false;
  if (trimmed !== "" && Number.isFinite(Number(trimmed))) return Number(trimmed);
  return value;
}

export function defaultBackoff() {
  return {
    initial_ms: 1000,
    multiplier: 2.0,
    max_ms: 30000,
    max_attempts: 5,
  };
}

/** Build a shell TestTemplate from form fields. */
export function buildTestTemplate(
  commandTemplate: string,
  answerTemplate: string,
  fixtures: unknown[],
): TestTemplate {
  return {
    kind: "shell",
    command_template: commandTemplate,
    ...(answerTemplate ? { answer_template: answerTemplate } : {}),
    fixtures,
    placeholders: extractTemplatePlaceholders(commandTemplate),
    backoff: defaultBackoff(),
  };
}

/** Parse a `fixtures` JSON form value into an array (empty on failure). */
export function parseFixtures(raw: FormDataEntryValue | null): unknown[] {
  if (typeof raw !== "string") return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}
