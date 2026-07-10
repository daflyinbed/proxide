import { execFile, spawn } from "node:child_process";
import { mkdir, writeFile, readFile, stat } from "node:fs/promises";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { login as httpLogin, uniqueName, uniqueScopedName, BASE_URL } from "./helpers.js";

const execFileAsync = promisify(execFile);

const E2E_RUN_DIR = resolve("/tmp", "proxide-e2e-npm-cli");
export const NPM_REGISTRY = `${BASE_URL}/npm/`;

export interface NpmResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

function buildNpmEnv(extra?: Record<string, string>): Record<string, string> {
  const env: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env as Record<string, string>)) {
    if (!k.startsWith("npm_config_")) {
      env[k] = v;
    }
  }
  if (extra) {
    Object.assign(env, extra);
  }
  return env;
}

export async function execNpm(
  args: string[],
  opts: { cwd: string; env?: Record<string, string> },
): Promise<NpmResult> {
  const env = buildNpmEnv(opts.env);
  try {
    const { stdout, stderr } = await execFileAsync("npm", args, {
      cwd: opts.cwd,
      env,
      timeout: 60_000,
    });
    return { stdout: stdout ?? "", stderr: stderr ?? "", exitCode: 0 };
  } catch (err: any) {
    return {
      stdout: err.stdout ?? "",
      stderr: err.stderr ?? "",
      exitCode: typeof err.status === "number" ? err.status : 1,
    };
  }
}

export async function execNpmStdin(
  args: string[],
  opts: { cwd: string; env?: Record<string, string>; input: string },
): Promise<NpmResult> {
  const env = buildNpmEnv(opts.env);
  return new Promise((resolve) => {
    const child = spawn("npm", args, {
      cwd: opts.cwd,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stdin.write(opts.input);
    child.stdin.end();
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d.toString()));
    child.stderr.on("data", (d) => (stderr += d.toString()));
    child.on("close", (code) =>
      resolve({ stdout, stderr, exitCode: code ?? 1 }),
    );
  });
}

export async function createTempDir(prefix: string): Promise<string> {
  const dir = join(E2E_RUN_DIR, `${prefix}-${Date.now().toString(36)}`);
  await mkdir(dir, { recursive: true });
  await writeFile(join(dir, "package.json"), '{"name":"e2e-install-dir","version":"0.0.0"}');
  return dir;
}

export async function createTempPackageDir(
  name: string,
  version: string,
  extra?: Record<string, any>,
): Promise<string> {
  const dir = await createTempDir(`pkg-${name.replace(/[\/@]/g, "_")}`);
  const pkgJson: Record<string, any> = {
    name,
    version,
    description: `e2e npm cli test package ${name}`,
    main: "index.js",
    ...extra,
  };
  await writeFile(join(dir, "package.json"), JSON.stringify(pkgJson, null, 2));
  await writeFile(join(dir, "index.js"), 'module.exports = "e2e";\n');
  return dir;
}

export async function writeNpmrc(dir: string, token: string): Promise<void> {
  const content = [
    `registry=${NPM_REGISTRY}`,
    `//localhost:14873/npm/:_authToken=${token}`,
    `//localhost:14873/npm/:always-auth=true`,
  ].join("\n");
  await writeFile(join(dir, ".npmrc"), content);
}

export async function npmLogin(username: string, password: string): Promise<string> {
  return httpLogin(username, password);
}

export function makeUnscopedPkg(): { name: string; version: string } {
  return { name: uniqueName("e2e-npm-cli"), version: "1.0.0" };
}

export function makeScopedPkg(): { name: string; version: string } {
  return { name: uniqueScopedName("e2e-scope", "e2e-npm-cli"), version: "1.0.0" };
}

export async function fileExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

export async function readJsonFile(path: string): Promise<any> {
  const content = await readFile(path, "utf-8");
  return JSON.parse(content);
}

let authDirCounter = 0;

export async function createAuthDir(): Promise<string> {
  const dir = join(
    E2E_RUN_DIR,
    `auth-${authDirCounter++}-${Date.now().toString(36)}`,
  );
  await mkdir(dir, { recursive: true });
  return dir;
}

export function npmrcEnv(authDir: string): Record<string, string> {
  return { NPM_CONFIG_USERCONFIG: join(authDir, ".npmrc") };
}

export async function npmAdduser(
  dir: string,
  username: string,
  password: string,
): Promise<NpmResult> {
  try {
    const token = await httpLogin(username, password);
    const url = new URL(NPM_REGISTRY);
    const authKey = `//${url.host}${url.pathname}:_authToken`;
    await writeFile(
      join(dir, ".npmrc"),
      [
        `registry=${NPM_REGISTRY}`,
        `${authKey}=${token}`,
        `//${url.host}${url.pathname}:always-auth=true`,
      ].join("\n") + "\n",
    );
    return {
      stdout: `Logged in as ${username}\n`,
      stderr: "",
      exitCode: 0,
    };
  } catch (err: any) {
    return { stdout: "", stderr: String(err.message ?? err), exitCode: 1 };
  }
}

export async function setupCliAuth(): Promise<{
  username: string;
  authDir: string;
}> {
  const username = uniqueName("e2e-cli");
  const authDir = await createAuthDir();
  const res = await npmAdduser(authDir, username, "pass1234");
  if (res.exitCode !== 0) {
    throw new Error(`npmAdduser failed: ${res.stderr}`);
  }
  return { username, authDir };
}
