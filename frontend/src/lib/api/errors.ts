export class ApiError extends Error {
  status: number;
  code: string | null;
  body: unknown;

  constructor(status: number, code: string | null, body: unknown, message?: string) {
    super(message ?? `api ${status}${code ? ` (${code})` : ""}`);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.body = body;
  }
}
