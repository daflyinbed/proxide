export const BASE_URL = "http://localhost:14873";

export async function api(
  path: string,
  opts: RequestInit = {},
): Promise<Response> {
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
): Promise<{ res: Response; body: any }> {
  const { buildPublishPayload } = await import("./fixtures/tarball.js");
  const payload = buildPublishPayload(name, version);
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
