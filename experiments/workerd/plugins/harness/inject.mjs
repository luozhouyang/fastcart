// Reference HTML injector used by P0. ASCII corpus; the Rust port must scan bytes.
export const HEAD_MARKER = "<!--fastcart:extensions-head-->";
export const BODY_MARKER = "<!--fastcart:extensions-body-->";
export const HEAD_SCRIPT =
  '<script type="application/fastcart-extensions" data-slot="head"></script>';
export const BODY_SCRIPT =
  '<script type="application/fastcart-extensions" data-slot="body"></script>';
export const SENTINEL = "<!--fastcart:extensions-injected-->";

const MARKERS = [
  { slot: "head", token: HEAD_MARKER },
  { slot: "body", token: BODY_MARKER },
  { slot: "head", token: HEAD_SCRIPT },
  { slot: "body", token: BODY_SCRIPT },
];

export function shouldInject({
  status = 200,
  contentType = "text/html",
  contentEncoding,
  body = "",
} = {}) {
  if (status !== 200) return false;
  if (!/text\/html/i.test(contentType)) return false;
  if (contentEncoding && contentEncoding.toLowerCase() !== "identity") return false;
  const head = body.slice(0, 1024).toLowerCase();
  return head.includes("<!doctype") || head.includes("<html");
}

export class Injector {
  constructor({ headTags = "", bodyTags = "", strict = false, fallback = true } = {}) {
    this.headTags = headTags;
    this.bodyTags = bodyTags;
    this.strict = strict;
    this.fallback = fallback;
    this.buffer = "";
    this.document = "unknown";
    this.resolved = { head: false, body: false };
    this.sentinel = false;
    this.emitted = false;
  }

  write(chunk) {
    if (this.document === "no" || this.sentinel) return chunk;
    this.buffer += chunk;
    if (this.buffer.includes(SENTINEL)) {
      this.sentinel = true;
      const out = this.buffer;
      this.buffer = "";
      return out;
    }
    if (this.document === "unknown") {
      if (this.buffer.length < 1024) return "";
      const head = this.buffer.slice(0, 1024).toLowerCase();
      this.document = head.includes("<!doctype") || head.includes("<html") ? "yes" : "no";
      if (this.document === "no") {
        const out = this.buffer;
        this.buffer = "";
        return out;
      }
    }
    return this.drain(false);
  }

  end() {
    if (this.document === "no" || this.sentinel) return this.buffer;
    if (this.document === "unknown") {
      const head = this.buffer.slice(0, 1024).toLowerCase();
      this.document = head.includes("<!doctype") || head.includes("<html") ? "yes" : "no";
    }
    return this.drain(true);
  }

  prefix() {
    if (this.emitted) return "";
    this.emitted = true;
    return SENTINEL;
  }

  drain(final) {
    let output = "";
    // Replace the first occurrence of each marker form.
    while (true) {
      let earliest = null;
      for (const marker of MARKERS) {
        if (this.resolved[marker.slot]) continue;
        const index = this.buffer.indexOf(marker.token);
        if (index === -1) continue;
        if (!earliest || index < earliest.index) earliest = { ...marker, index };
      }
      if (!earliest) break;
      const tags = earliest.slot === "head" ? this.headTags : this.bodyTags;
      output += this.buffer.slice(0, earliest.index) + this.prefix() + tags;
      this.buffer = this.buffer.slice(earliest.index + earliest.token.length);
      this.resolved[earliest.slot] = true;
    }

    // Head fallback can be decided as soon as </head> is seen.
    if (!this.resolved.head && this.fallback && !this.strict) {
      const headClose = this.buffer.toLowerCase().indexOf("</head>");
      if (headClose !== -1) {
        output += this.buffer.slice(0, headClose) + this.prefix() + this.headTags;
        this.buffer = this.buffer.slice(headClose);
        this.resolved.head = true;
      }
    }

    if (final) {
      if (!this.resolved.body && this.fallback && !this.strict) {
        const index = this.buffer.toLowerCase().lastIndexOf("</body>");
        if (index !== -1) {
          output +=
            this.buffer.slice(0, index) + this.prefix() + this.bodyTags + this.buffer.slice(index);
          this.buffer = "";
          this.resolved.body = true;
          return output;
        }
      }
      output += this.buffer;
      this.buffer = "";
      return output;
    }

    // Keep a tail only while the body slot may still need the fallback.
    const keep = !this.resolved.body && this.fallback && !this.strict ? 32768 : 128;
    if (this.buffer.length > keep) {
      output += this.buffer.slice(0, this.buffer.length - keep);
      this.buffer = this.buffer.slice(-keep);
    }
    return output;
  }
}

export function inject(html, options = {}) {
  const injector = new Injector(options);
  return injector.write(html) + injector.end();
}

export function injectChunked(chunks, options = {}) {
  const injector = new Injector(options);
  let output = "";
  for (const chunk of chunks) output += injector.write(chunk);
  return output + injector.end();
}
