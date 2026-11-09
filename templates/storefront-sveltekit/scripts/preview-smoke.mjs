import { spawn } from "node:child_process";

const command = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
const child = spawn(command, ["run", "test:artifact"], {
  stdio: "inherit",
  env: {
    ...process.env,
    FASTCART_EXPECT_ASSET_PREFIX: "/_fastcart/preview-assets/",
  },
});
child.on("error", (error) => {
  console.error(error);
  process.exit(1);
});
child.on("exit", (code) => process.exit(code ?? 1));
