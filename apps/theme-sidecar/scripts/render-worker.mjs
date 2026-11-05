// Isolated render worker.
//
// Each request imports the theme entry and calls its render export inside a
// short-lived worker thread. The main sidecar terminates the worker when the
// request deadline passes, so a runaway theme cannot block other renders or
// the health endpoint. Resource limits cap the heap the theme can allocate.

import { parentPort, workerData } from "node:worker_threads";
import { pathToFileURL } from "node:url";

const { entryFile, version, payload } = workerData;

async function main() {
  const url = `${pathToFileURL(entryFile).href}?v=${encodeURIComponent(String(version))}`;
  const module = await import(url);
  const render = renderFunction(module);
  if (!render) {
    throw new Error("theme entry does not export a render function");
  }
  const result = await render(payload);
  parentPort.postMessage({ ok: true, result });
}

function renderFunction(module) {
  if (typeof module?.render === "function") {
    return module.render;
  }
  if (typeof module?.default?.render === "function") {
    return module.default.render;
  }
  if (typeof module?.default === "function") {
    return module.default;
  }
  return null;
}

main().catch((error) => {
  parentPort.postMessage({
    ok: false,
    message: error instanceof Error ? error.message : String(error),
  });
});
