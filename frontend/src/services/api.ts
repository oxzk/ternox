const JSON_HEADERS = { "Content-Type": "application/json" };

/** 请求超时：防止后端/代理 hang 导致页面假死。 */
const REQUEST_TIMEOUT_MS = 10_000;

/** 终端信息：前端只消费 id；后端还返回 status/cols/rows。 */
export interface TerminalInfo {
  id: string;
}

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }

  /** 会话过期：应回到登录页。 */
  get authExpired(): boolean {
    return this.status === 401;
  }
}

async function parseError(res: Response): Promise<ApiError> {
  let message = `请求失败 (${res.status})`;
  try {
    const body = await res.json();
    if (typeof body?.error === "string") {
      message = body.error;
    }
  } catch {
    // 非 JSON 响应体，保留默认信息
  }
  return new ApiError(res.status, message);
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  let res: Response;
  try {
    // credentials 显式声明：跨域名部署时同源默认不会携带 Cookie
    res = await fetch(url, {
      ...init,
      credentials: "same-origin",
      headers: { ...JSON_HEADERS, ...init?.headers },
      signal: controller.signal,
    });
  } catch (err) {
    const aborted = err instanceof DOMException && err.name === "AbortError";
    throw new ApiError(0, aborted ? "请求超时" : "网络错误，请重试");
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) {
    throw await parseError(res);
  }
  try {
    return (await res.json()) as T;
  } catch {
    // 空响应体兜底，避免裸 SyntaxError 逃过 instanceof ApiError 分支
    return null as T;
  }
}

export function checkSession(): Promise<{ authenticated: boolean }> {
  return request("/api/session");
}

export function login(
  username: string,
  password: string,
): Promise<{ authenticated: boolean }> {
  return request("/api/login", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });
}

export function logout(): Promise<{ authenticated: boolean }> {
  return request("/api/logout", { method: "POST" });
}

/** 获取或初始化唯一终端（Shell 已退出时会在服务端重建）。 */
export function initTerminal(): Promise<TerminalInfo> {
  return request("/api/terminal", { method: "POST" });
}
