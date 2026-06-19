import { buildPublishPayload } from "./fixtures/tarball.js";

export const BASE_URL = "http://localhost:14873";

const MEILI_URL = "http://127.0.0.1:7700";
const MEILI_KEY = "proxide";
const MEILI_INDEX = "proxide-e2e";

export async function api(
  path: string,
  opts: RequestInit = {},
): Promise<Response> {
  if (path.startsWith("http://") || path.startsWith("https://")) {
    return fetch(path, opts);
  }
  return fetch(`${BASE_URL}${path}`, opts);
}

export async function apiJson<T = any>(
  path: string,
  opts: RequestInit = {},
): Promise<{ res: Response; body: T }> {
  const res = await api(path, opts);
  const body = (await res.json()) as T;
  return { res, body };
}

export async function login(
  username: string,
  password: string,
): Promise<string> {
  const { res, body } = await apiJson<{ token: string }>(
    `/npm/-/user/org.couchdb.user:${encodeURIComponent(username)}`,
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        name: username,
        password,
        email: `${username}@e2e.test`,
      }),
    },
  );
  if (res.status !== 201) {
    throw new Error(`login failed: ${res.status} ${JSON.stringify(body)}`);
  }
  return body.token;
}

export function packagePath(name: string): string {
  return `/npm/${name}`;
}

export async function publishPackage(
  token: string,
  name: string,
  version: string,
  opts?: { description?: string; keywords?: string[]; author?: string },
): Promise<{ res: Response; body: any }> {
  const payload = buildPublishPayload(name, version, opts);
  const { res, body } = await apiJson(packagePath(name), {
    method: "PUT",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${token}`,
    },
    body: JSON.stringify(payload),
  });
  return { res, body };
}

export function uniqueName(prefix: string): string {
  return `${prefix}-${process.pid}-${Date.now().toString(36)}`;
}

export function uniqueScopedName(scope: string, prefix: string): string {
  return `@${scope}/${uniqueName(prefix)}`;
}

export async function searchPackages(
  text: string,
  params?: { from?: number; size?: number },
): Promise<{ res: Response; body: any }> {
  const q = new URLSearchParams({ text });
  if (params?.from !== undefined) q.set("from", String(params.from));
  if (params?.size !== undefined) q.set("size", String(params.size));
  return apiJson(`/npm/-/v1/search?${q}`);
}

export async function waitForSearch(
  text: string,
  predicate: (body: any) => boolean,
  timeoutMs = 10_000,
): Promise<any> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const { body } = await searchPackages(text);
      if (predicate(body)) {
        return body;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`timed out waiting for search: text="${text}"`);
}

function meiliHeaders(): Record<string, string> {
  return {
    authorization: `Bearer ${MEILI_KEY}`,
    "content-type": "application/json",
  };
}

export async function waitForMeiliTask(taskUid: number, timeoutMs = 10_000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const res = await fetch(`${MEILI_URL}/tasks/${taskUid}`, { headers: meiliHeaders() });
      const body = await res.json();
      if (body.status === "succeeded") return;
      if (body.status === "failed") throw new Error(`meili task ${taskUid} failed: ${JSON.stringify(body)}`);
    } catch (e) {
      if (e instanceof Error && e.message.includes("failed")) throw e;
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`timed out waiting for meili task ${taskUid}`);
}

export async function clearSearchIndex(): Promise<void> {
  const res = await fetch(`${MEILI_URL}/indexes/${MEILI_INDEX}/documents`, {
    method: "DELETE",
    headers: meiliHeaders(),
  });
  const body = await res.json();
  await waitForMeiliTask(body.taskUid);
}

function sanitizeSearchId(name: string): string {
  return name.replace(/@/g, "").replace(/\//g, "__");
}

export async function deleteSearchDoc(id: string): Promise<void> {
  const res = await fetch(`${MEILI_URL}/indexes/${MEILI_INDEX}/documents/${encodeURIComponent(sanitizeSearchId(id))}`, {
    method: "DELETE",
    headers: meiliHeaders(),
  });
  const body = await res.json();
  await waitForMeiliTask(body.taskUid);
}
