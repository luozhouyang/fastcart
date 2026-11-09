import adapter from "@sveltejs/adapter-cloudflare";

const previewAppDir = process.env.FASTCART_PREVIEW_APP_DIR;

export default {
  kit: {
    appDir: previewAppDir ?? "_app",
    adapter: adapter(),
    csrf: {
      checkOrigin: false,
    },
  },
};
