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
  const SAFE_METHODS = new Set(["GET", "HEAD", "OPTIONS", "TRACE"]);

  function readCookie(name) {
    const prefix = name + "=";
    for (const part of document.cookie.split("; ")) {
      if (part.startsWith(prefix)) return decodeURIComponent(part.slice(prefix.length));
    }
    return null;
  }

  window.FastCartExtensions = {
    register(tag, define) {
      if (!customElements.get(tag)) customElements.define(tag, define);
    },
    config(id) {
      return config.items.find((item) => item.id === id);
    },
    api(path, init = {}) {
      const headers = new Headers(init.headers);
      const method = (init.method ?? "GET").toUpperCase();
      if (!SAFE_METHODS.has(method)) {
        const csrf = readCookie("fc_customer_csrf");
        if (csrf) headers.set("x-csrf-token", csrf);
      }
      return fetch(path, { ...init, credentials: "same-origin", headers });
    },
  };
})();
