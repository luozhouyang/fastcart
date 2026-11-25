// Chat bubble. Shadow DOM keeps theme styles out of the host page.
//
// The widget first probes the storefront capability route. When the store has
// bound and enabled livechat@1, it uses that same-origin endpoint and the
// greeting/accent from the plugin's settings. Otherwise it keeps the built-in
// defaults and falls back to the plugin's legacy /apps route, so the embed
// still mounts on stores that have not bound the capability.
const CAPABILITY_ENDPOINT = "/store/capabilities/livechat";
const LEGACY_ENDPOINT = "/apps/chat-widget/api/hello";
const DEFAULT_GREETING = "Hi! How can we help?";
const DEFAULT_ACCENT = "#2563eb";
const DEFAULT_PLACEMENT = "position:fixed;right:20px;bottom:20px;z-index:2147483000";

function api(path, init) {
  if (window.FastCartExtensions && typeof window.FastCartExtensions.api === "function") {
    return window.FastCartExtensions.api(path, init);
  }
  return fetch(path, { credentials: "same-origin", ...init });
}

class ChatBubble extends HTMLElement {
  connectedCallback() {
    if (this.shadowRoot) return;
    this.mode = "loading";
    this.greeting = DEFAULT_GREETING;
    this.accent = DEFAULT_ACCENT;

    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `
      <style>
        :host { all: initial; font: 14px/1.4 system-ui, sans-serif; }
        .root { display: flex; flex-direction: column; align-items: flex-end; gap: 12px; }
        .panel {
          box-sizing: border-box; width: 288px; max-width: calc(100vw - 40px);
          padding: 14px; border-radius: 16px; background: #fff; color: #111827;
          box-shadow: 0 12px 32px rgba(0, 0, 0, .22);
        }
        .panel[hidden] { display: none; }
        .greeting { margin: 0 0 10px; font-weight: 600; }
        form { display: flex; gap: 8px; }
        input {
          box-sizing: border-box; flex: 1; min-width: 0; padding: 8px 10px;
          border: 1px solid #d1d5db; border-radius: 10px; font: inherit;
        }
        input:focus-visible { outline: 3px solid #93c5fd; outline-offset: 1px; }
        .send {
          padding: 8px 12px; border: 0; border-radius: 10px; color: #fff;
          background: var(--fc-chat-accent, #2563eb); font: inherit; cursor: pointer;
        }
        .send:disabled { opacity: .6; cursor: default; }
        .send:focus-visible { outline: 3px solid #93c5fd; outline-offset: 2px; }
        .status { min-height: 1.4em; margin: 8px 0 0; color: #4b5563; }
        .bubble {
          width: 56px; height: 56px; border-radius: 999px; border: 0;
          background: var(--fc-chat-accent, #2563eb); color: #fff;
          font-size: 22px; cursor: pointer; box-shadow: 0 8px 24px rgba(0, 0, 0, .25);
        }
        .bubble:focus-visible { outline: 3px solid #93c5fd; outline-offset: 2px; }
      </style>
      <div class="root">
        <section class="panel" hidden aria-label="Live chat">
          <p class="greeting"></p>
          <form>
            <input type="text" name="message" autocomplete="off" aria-label="Message" />
            <button class="send" type="submit">Send</button>
          </form>
          <p class="status" role="status"></p>
        </section>
        <button class="bubble" type="button" aria-label="Open live chat" aria-expanded="false">
          💬
        </button>
      </div>
    `;

    this.panel = root.querySelector(".panel");
    this.greetingNode = root.querySelector(".greeting");
    this.statusNode = root.querySelector(".status");
    this.form = root.querySelector("form");
    this.input = root.querySelector("input");
    this.sendButton = root.querySelector(".send");
    this.bubbleButton = root.querySelector(".bubble");

    this.applySettings();
    this.bubbleButton.addEventListener("click", () => {
      void this.toggle();
    });
    this.form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.send();
    });
    this.ready = this.loadCapability();
  }

  applySettings() {
    this.greetingNode.textContent = this.greeting;
    this.style.setProperty("--fc-chat-accent", this.accent);
    this.bubbleButton.style.background = this.accent;
  }

  async loadCapability() {
    let session = null;
    try {
      const response = await fetch(`${CAPABILITY_ENDPOINT}/session`, {
        credentials: "same-origin",
        headers: { accept: "application/json" },
      });
      if (response.ok) {
        const payload = await response.json();
        if (payload && payload.capability === "livechat") session = payload;
      } else if (response.status !== 404) {
        console.warn("[chat-widget] capability session failed", response.status);
      }
    } catch (error) {
      console.warn("[chat-widget] capability session unavailable", error);
    }
    if (!session) {
      this.mode = "legacy";
      return;
    }
    this.mode = "capability";
    if (typeof session.greeting === "string" && session.greeting.trim().length > 0) {
      this.greeting = session.greeting;
    }
    if (typeof session.accent === "string" && session.accent.trim().length > 0) {
      this.accent = session.accent;
    }
    this.applySettings();
  }

  async toggle() {
    await this.ready;
    if (this.mode !== "capability") {
      await this.legacyPing();
      return;
    }
    const open = this.panel.hidden;
    this.panel.hidden = !open;
    this.bubbleButton.setAttribute("aria-expanded", String(open));
    if (open) this.input.focus();
  }

  async send() {
    const text = this.input.value.trim();
    if (!text) return;
    this.sendButton.disabled = true;
    this.statusNode.textContent = "Sending…";
    try {
      const response = await fetch(`${CAPABILITY_ENDPOINT}/messages`, {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ message: text }),
      });
      const payload = await response.json().catch(() => null);
      if (response.status === 404) {
        this.mode = "legacy";
        this.statusNode.textContent = "Live chat is not available for this store.";
        return;
      }
      if (!response.ok) {
        this.statusNode.textContent =
          (payload && typeof payload.error === "string" && payload.error) ||
          "Message could not be sent.";
        return;
      }
      this.input.value = "";
      this.statusNode.textContent = `Received: ${payload && payload.echo ? payload.echo : text}`;
    } catch {
      this.statusNode.textContent = "Message could not be sent.";
    } finally {
      this.sendButton.disabled = false;
    }
  }

  async legacyPing() {
    try {
      const response = await api(LEGACY_ENDPOINT, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ source: "bubble" }),
      });
      console.info("[chat-widget]", await response.json());
    } catch (error) {
      console.warn("[chat-widget]", error);
    }
  }
}

if (!customElements.get("fc-chat-bubble")) {
  customElements.define("fc-chat-bubble", ChatBubble);
}

if (!document.querySelector("fc-chat-bubble")) {
  const element = document.createElement("fc-chat-bubble");
  element.style.cssText = DEFAULT_PLACEMENT;
  document.body.appendChild(element);
}
