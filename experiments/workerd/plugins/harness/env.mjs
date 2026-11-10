import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const PLUGIN_ROOT = fileURLToPath(new URL("..", import.meta.url));
export const HARNESS_DIR = path.join(PLUGIN_ROOT, "harness");
export const WORKERD =
  process.env.FASTCART_WORKERD_BIN ?? path.resolve(PLUGIN_ROOT, "../bin/workerd");
export const EXPERIMENTAL = process.env.FASTCART_WORKERD_EXPERIMENTAL !== "0";
export const REPORT_DIR = path.join(PLUGIN_ROOT, "report");
export const RESULTS_FILE =
  process.env.FASTCART_P0_RESULTS ?? path.join(REPORT_DIR, "results.jsonl");

export async function freePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

export async function delay(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}
