import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { buildConfig } from "./config.mjs";
import { startCommerce } from "./commerce.mjs";
import { freePort, HARNESS_DIR } from "./env.mjs";
import { startWorkerd } from "./workerd.mjs";

const gatewaySource = await readFile(path.join(HARNESS_DIR, "gateway.js"), "utf8");
const bridgeSource = await readFile(path.join(HARNESS_DIR, "bridge.js"), "utf8");

export async function startSuite({ fixtures, experimental = true }) {
  const commerce = await startCommerce(fixtures);
  const port = await freePort();
  const dir = await mkdtemp(path.join(os.tmpdir(), "fc-plugin-p0-"));
  await writeFile(path.join(dir, "gateway.js"), gatewaySource);
  await writeFile(path.join(dir, "bridge.js"), bridgeSource);
  await writeFile(
    path.join(dir, "config.capnp"),
    buildConfig({ port, commercePort: commerce.port }),
  );
  const startedAt = performance.now();
  const runtime = await startWorkerd({
    configPath: path.join(dir, "config.capnp"),
    cwd: dir,
    port,
    experimental,
  });
  const bootMs = performance.now() - startedAt;
  return {
    bootMs,
    base: runtime.base,
    port,
    child: runtime.child,
    pid: runtime.child.pid,
    logs: runtime.logs,
    commerce,
    request: (pathAndQuery) => fetch(`${runtime.base}${pathAndQuery}`),
    async stop() {
      await runtime.stop();
      await commerce.stop();
      await rm(dir, { recursive: true, force: true });
    },
  };
}
