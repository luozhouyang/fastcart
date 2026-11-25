// Live chat button component.
//
// Served from the plugin archive by the digest-scoped storefront extension
// route and mounted by a theme wherever a page author placed the
// `plugin:chat-widget:livechat-button` block. The module is self-contained:
// importing it registers <fc-livechat-button>, and the element reads its
// attributes (`label`, `position`) plus the store's capability binding.
//
// When the `livechat` capability is unbound or disabled the button still
// renders and explains that chat is unavailable, so the page never breaks.

const CAPABILITY_ENDPOINT = "/store/capabilities/livechat";
const DEFAULT_LABEL = "Chat with us";
const DEFAULT_POSITION = "bottom-right";

function api(path, init) {
  const request = { credentials: "same-origin", ...init };
  if (window.FastCartExtensions && typeof window.FastCartExtensions.api === "function") {
    return window.FastCartExtensions.api(path, request);
  }
  return fetch(path, request);
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

class LivechatButton extends HTMLElement {
  static get observedAttributes() {
    return ["label", "position"];
  }

  connectedCallback() {
    if (this.shadowRoot) return;
    this.available = null;
    this.greeting = "";

    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `
      <style>
        :host { all: initial; font: 14px/1.4 system-ui, sans-serif; display: inline-block; }
        .button {
          display: inline-flex; align-items: center; gap: 8px;
          padding: 10px 14px; border: 0; border-radius: 999px;
          background: var(--fc-chat-accent, #2563eb); color: #fff;
          font: inherit; font-weight: 600; cursor: pointer;
          box-shadow: 0 8px 24px rgba(0, 0, 0, .25);
        }
        .button[aria-disabled="true"] { opacity: .6; cursor: default; }
        .button:focus-visible { outline: 3px solid #93c5fd; outline-offset: 2px; }
        .panel {
          box-sizing: border-box; width: 288px; max-width: calc(100vw - 40px);
          margin-top: 12px; padding: 14px; border-radius: 16px;
          background: #fff; color: #111827;
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
        .status { min-height: 1.4em; margin: 8px 0 0; color: #4b5563; }
      </style>
      <button class="button" type="button" aria-expanded="false">
        <span class="label"></span>
      </button>
      <section class="panel" hidden aria-label="Live chat">
        <p class="greeting"></p>
        <form>
          <input type="text" name="message" autocomplete="off" aria-label="Message" />
          <button class="send" type="submit">Send</button>
        </form>
        <p class="status" role="status"></p>
      </section>
    `;

    this.button = root.querySelector(".button");
    this.labelNode = root.querySelector(".label");
    this.panel = root.querySelector(".panel");
    this.greetingNode = root.querySelector(".greeting");
    this.statusNode = root.querySelector(".status");
    this.input = root.querySelector("input");
    this.sendButton = root.querySelector(".send");

    this.button.addEventListener("click", () => this.toggle());
    root.querySelector("form").addEventListener("submit", (event) => {
      event.preventDefault();
      void this.send();
    });

    this.applyLabel();
    this.applyPosition();
    this.ready = this.loadSession();
  }

  attributeChangedCallback(name) {
    if (name === "label") this.applyLabel();
    if (name === "position") this.applyPosition();
  }

  applyLabel() {
    if (!this.labelNode) return;
    this.labelNode.textContent = nonEmptyString(this.getAttribute("label")) ?? DEFAULT_LABEL;
    this.button.setAttribute("aria-label", `${this.labelNode.textContent}: open live chat`);
  }

  applyPosition() {
    const position = this.getAttribute("position");
    this.style.display = "inline-block";
    this.dataset.position = position === "bottom-left" ? "bottom-left" : DEFAULT_POSITION;
  }

  async loadSession() {
    try {
      const response = await api(`${CAPABILITY_ENDPOINT}/session`, {
        headers: { accept: "application/json" },
      });
      if (!response.ok) {
        this.available = false;
        this.button.setAttribute("aria-disabled", "true");
        this.statusNode.textContent = "Live chat is not available for this store.";
        return;
      }
      const session = await response.json().catch(() => null);
      if (!session || session.capability !== "livechat") {
        this.available = false;
        this.button.setAttribute("aria-disabled", "true");
        return;
      }
      this.available = true;
      this.greeting = nonEmptyString(session.greeting) ?? "";
      this.greetingNode.textContent = this.greeting;
      const accent = nonEmptyString(session.accent);
      if (accent) this.style.setProperty("--fc-chat-accent", accent);
    } catch {
      this.available = false;
      this.button.setAttribute("aria-disabled", "true");
      this.statusNode.textContent = "Live chat is not available for this store.";
    }
  }

  async toggle() {
    if (this.available === false) return;
    await this.ready;
    if (this.available === false) return;
    const open = this.panel.hidden;
    this.panel.hidden = !open;
    this.button.setAttribute("aria-expanded", String(open));
    if (open) this.input.focus();
  }

  async send() {
    const text = this.input.value.trim();
    if (!text || this.available === false) return;
    this.sendButton.disabled = true;
    this.statusNode.textContent = "Sending…";
    try {
      const response = await api(`${CAPABILITY_ENDPOINT}/messages`, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify({ message: text }),
      });
      const payload = await response.json().catch(() => null);
      if (response.status === 404) {
        this.available = false;
        this.button.setAttribute("aria-disabled", "true");
        this.statusNode.textContent = "Live chat is not available for this store.";
        return;
      }
      if (!response.ok) {
        const message = payload && typeof payload.error === "string" ? payload.error : null;
        this.statusNode.textContent = message ?? "Message could not be sent.";
        return;
      }
      this.input.value = "";
      this.statusNode.textContent =
        payload && typeof payload.echo === "string" ? `Received: ${payload.echo}` : "Message sent.";
    } catch {
      this.statusNode.textContent = "Message could not be sent.";
    } finally {
      this.sendButton.disabled = false;
    }
  }
}

if (!customElements.get("fc-livechat-button")) {
  customElements.define("fc-livechat-button", LivechatButton);
}

export { LivechatButton as default, LivechatButton };
