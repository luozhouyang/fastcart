// FastCart storefront extension runtime (v1).
(async () => {
  let config = window.__FASTCART_EXTENSIONS__;
  if (!config) {
    try {
      const response = await fetch("/_fastcart/extensions/manifest.json", {
        credentials: "same-origin",
      });
      if (!response.ok) return;
      config = await response.json();
    } catch {
      return;
    }
  }
  if (!config || !Array.isArray(config.items)) return;
  for (const item of config.items) {
    import(item.entry).catch((error) => {
      console.error("[fastcart] plugin boot failed", item.id, error);
    });
  }
  window.FastCartExtensions = {
    register(tag, define) {
      if (!customElements.get(tag)) customElements.define(tag, define);
    },
    config(id) {
      return config.items.find((item) => item.id === id);
    },
    api(path, init) {
      return fetch(path, { credentials: "same-origin", ...init });
    },
  };
})();
