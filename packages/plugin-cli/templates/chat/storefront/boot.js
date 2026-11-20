// Chat bubble. Shadow DOM keeps theme styles out of the host page.
const placement = "position:fixed;right:20px;bottom:20px;z-index:2147483000";

class ChatBubble extends HTMLElement {
  connectedCallback() {
    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `
      <style>
        :host { all: initial; font: 14px/1.4 system-ui, sans-serif; }
        button {
          width: 56px; height: 56px; border-radius: 999px; border: 0;
          background: var(--fc-chat-accent, #2563eb); color: #fff;
          font-size: 22px; cursor: pointer; box-shadow: 0 8px 24px rgba(0,0,0,.25);
        }
        button:focus-visible { outline: 3px solid #93c5fd; outline-offset: 2px; }
      </style>
      <button type="button" aria-label="Open live chat">💬</button>
    `;
    root.querySelector("button").addEventListener("click", () => this.open());
  }

  async open() {
    const response = await window.FastCartExtensions.api("/apps/__PLUGIN_ID__/api/hello", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ source: "bubble" }),
    });
    console.info("[__PLUGIN_ID__]", await response.json());
  }
}

customElements.define("fc-chat-bubble", ChatBubble);
const element = document.createElement("fc-chat-bubble");
element.style.cssText = placement;
document.body.appendChild(element);
