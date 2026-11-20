import { spawn } from "node:child_process";
import { once } from "node:events";
import { EXPERIMENTAL, WORKERD } from "./env.mjs";

export async function startWorkerd({
  configPath,
  cwd,
  port,
  experimental = EXPERIMENTAL,
  timeoutMs = 20_000,
}) {
  const args = ["serve", ...(experimental ? ["--experimental"] : []), configPath];
  const child = spawn(WORKERD, args, { cwd, stdio: ["ignore", "pipe", "pipe"] });
  const logs = [];
  const collect = (chunk) => {
    for (const line of chunk.toString().split("\n")) {
      if (line.trim().length > 0) logs.push(line);
    }
  };
  child.stdout.on("data", collect);
  child.stderr.on("data", collect);
  const exited = once(child, "exit").then(([code, signal]) => ({ code, signal }));

  const base = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`workerd exited early (${child.exitCode})\n${logs.join("\n")}`);
    }
    try {
      const response = await fetch(`${base}/__fastcart/health`);
      if (response.ok) {
        return { child, logs, base, exited, stop: () => stopWorkerd(child) };
      }
    } catch {
      // not ready yet
    }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  await stopWorkerd(child);
  throw new Error(`workerd did not become ready in ${timeoutMs}ms\n${logs.join("\n")}`);
}

export async function stopWorkerd(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  child.kill("SIGTERM");
  const timer = new Promise((resolve) => setTimeout(resolve, 3_000));
  await Promise.race([once(child, "exit"), timer]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}

export async function runWorkerdOnce({ configPath, cwd, timeoutMs = 5_000 }) {
  const child = spawn(WORKERD, ["serve", configPath], { cwd, stdio: ["ignore", "pipe", "pipe"] });
  const logs = [];
  child.stdout.on("data", (chunk) => logs.push(chunk.toString()));
  child.stderr.on("data", (chunk) => logs.push(chunk.toString()));
  const timer = setTimeout(() => child.kill("SIGKILL"), timeoutMs);
  const [code, signal] = await once(child, "exit");
  clearTimeout(timer);
  return { code, signal, output: logs.join("") };
}
