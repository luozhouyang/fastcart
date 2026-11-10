#!/usr/bin/env node
/** `fastcart-plugin` command line interface. */

import { parseArgs } from "node:util";
import { resolve } from "node:path";

import { devPlugin } from "./dev.ts";
import { doctorPlugin } from "./doctor.ts";
import { initPlugin, type TemplateName } from "./init.ts";
import { packPlugin } from "./pack.ts";

const HELP = `fastcart-plugin — build and scaffold FastCart plugins

Usage:
  fastcart-plugin init <dir> [--template minimal|admin|chat] [--force]
  fastcart-plugin pack <dir> [--out <dir|file>] [--no-bundle] [--keep-staging]
  fastcart-plugin dev <dir> [--out <dir>] [--interval <ms>] [--no-serve] [--port <port>] [--workerd <path>]
  fastcart-plugin doctor <dir>

Examples:
  fastcart-plugin init chat-widget --template chat
  fastcart-plugin pack chat-widget
  fastcart-plugin dev chat-widget --out .fastcart
`;

async function main(argv: string[]): Promise<void> {
  const { positionals, values } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      template: { type: "string", default: "minimal" },
      force: { type: "boolean", default: false },
      out: { type: "string" },
      "no-bundle": { type: "boolean", default: false },
      "keep-staging": { type: "boolean", default: false },
      interval: { type: "string" },
      port: { type: "string" },
      workerd: { type: "string" },
      "no-serve": { type: "boolean", default: false },
      help: { type: "boolean", short: "h", default: false },
      version: { type: "boolean", short: "v", default: false },
    },
  });

  if (values.version) {
    process.stdout.write("0.1.0\n");
    return;
  }
  if (values.help || positionals.length === 0) {
    process.stdout.write(HELP);
    return;
  }
  const [command, dir] = positionals;
  if (!command || !dir) {
    throw new Error("expected a command and a plugin directory");
  }
  switch (command) {
    case "init": {
      const template = values.template as TemplateName;
      if (!["minimal", "admin", "chat"].includes(template)) {
        throw new Error(`unknown template: ${template}`);
      }
      const result = await initPlugin({ dir, template, force: Boolean(values.force) });
      process.stdout.write(`Created ${result.template} plugin in ${result.dir}\n`);
      process.stdout.write("Next: fastcart-plugin pack " + dir + "\n");
      return;
    }
    case "pack": {
      const result = await packPlugin({
        root: dir,
        ...(values.out ? { out: values.out } : {}),
        bundle: !values["no-bundle"],
        keepStaging: Boolean(values["keep-staging"]),
      });
      process.stdout.write(
        `Packed ${result.manifest.id}@${result.manifest.version}: ${result.files} files, ${result.bytes} bytes\n` +
          `Archive: ${resolve(result.archive)}\nsha256: ${result.digest}\n`,
      );
      return;
    }
    case "dev": {
      const out = values.out ?? ".fastcart";
      const interval = values.interval ? Number.parseInt(values.interval, 10) : 500;
      const port = values.port ? Number.parseInt(values.port, 10) : 8787;
      const session = await devPlugin({
        root: dir,
        out,
        intervalMs: Number.isFinite(interval) && interval > 0 ? interval : 500,
        serve: !values["no-serve"],
        port: Number.isFinite(port) && port > 0 ? port : 8787,
        ...(values.workerd ? { workerd: values.workerd } : {}),
      });
      process.stdout.write(
        session.url
          ? `Preview ${session.url}\nWatching ${resolve(dir)}\n`
          : `Watching ${resolve(dir)}; writing ${session.out}\n`,
      );
      const shutdown = () => {
        void session.stop().finally(() => process.exit(0));
      };
      process.once("SIGINT", shutdown);
      process.once("SIGTERM", shutdown);
      return;
    }
    case "doctor": {
      const result = await doctorPlugin(dir);
      process.stdout.write(
        `Plugin ${result.manifest.id}@${result.manifest.version}\n` +
          `Entry: ${result.manifest.entry.path}\n` +
          `Files: ${result.files}\n` +
          (result.missing.length === 0 ? "OK\n" : `Missing: ${result.missing.join(", ")}\n`),
      );
      if (result.missing.length > 0) process.exitCode = 1;
      return;
    }
    default:
      throw new Error(`unknown command: ${command}`);
  }
}

main(process.argv.slice(2)).catch((error: unknown) => {
  process.stderr.write(
    `fastcart-plugin: ${error instanceof Error ? error.message : String(error)}\n`,
  );
  process.exitCode = 1;
});
