import { fileURLToPath } from "node:url";

import viteReact from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const clientOutDir = fileURLToPath(new URL("./dist/client", import.meta.url));
const serverOutDir = fileURLToPath(new URL("./dist/server", import.meta.url));

/**
 * One `vite build` produces the theme bundle:
 *
 * - `dist/client/client.js` is the browser hydration entry.
 * - `dist/client/editor.js` is the executable registry the admin editor loads.
 * - `dist/server/server.js` is the Node entry imported by the sidecar.
 *
 * Entry names are deliberately unhashed so the server and admin routes can
 * reference them without reading the build manifest. Chunks and assets keep a
 * content hash so the sidecar can serve them with an HTTP cache.
 */
export default defineConfig({
  root,
  plugins: [viteReact()],
  environments: {
    client: {
      build: {
        outDir: clientOutDir,
        emptyOutDir: true,
        sourcemap: false,
        rollupOptions: {
          input: {
            client: fileURLToPath(new URL("./src/client.tsx", import.meta.url)),
          },
          output: {
            entryFileNames: "[name].js",
            chunkFileNames: "chunks/[name]-[hash].js",
            assetFileNames: "assets/[name]-[hash][extname]",
          },
        },
      },
    },
    // The editor registry is loaded by the admin as a single ESM module. It is
    // built separately with inlined dynamic imports so the admin does not need
    // a route for shared chunks next to `editor.js`.
    editor: {
      // Treat the registry as a browser bundle: React and workspace packages
      // are inlined so the admin can load `editor.js` with a plain script tag.
      consumer: "client",
      build: {
        outDir: clientOutDir,
        emptyOutDir: false,
        sourcemap: false,
        rollupOptions: {
          input: fileURLToPath(new URL("./src/editor.ts", import.meta.url)),
          // The admin imports this entry directly and needs its exports, so
          // Rollup must not treat the registry as a tree-shaken app entry.
          preserveEntrySignatures: "strict",
          output: {
            entryFileNames: "editor.js",
            codeSplitting: false,
            assetFileNames: "assets/[name]-[hash][extname]",
          },
        },
      },
    },
    ssr: {
      resolve: {
        // Workspace packages ship TypeScript sources, so they must be bundled
        // into the server output instead of being externalized.
        noExternal: [/^@fastcart\//],
      },
      build: {
        outDir: serverOutDir,
        emptyOutDir: true,
        ssr: true,
        target: "node22",
        rollupOptions: {
          input: fileURLToPath(new URL("./src/server.tsx", import.meta.url)),
          output: {
            format: "es",
            entryFileNames: "server.js",
            chunkFileNames: "chunks/[name]-[hash].js",
            assetFileNames: "assets/[name]-[hash][extname]",
          },
        },
      },
    },
  },
  builder: {
    async buildApp(builder) {
      await builder.build(builder.environments.client!);
      await builder.build(builder.environments.editor!);
      await builder.build(builder.environments.ssr!);
    },
  },
});
