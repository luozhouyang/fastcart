//! Parsing for the trusted storefront capability header injected by the Rust
//! gateway.
//!
//! The gateway sets `x-fastcart-capabilities` on a workerd request only when a
//! store has at least one enabled capability binding. Its value is `1.`
//! followed by the unpadded base64url encoding of a UTF-8 JSON envelope:
//!
//! ```text
//! {"v":1,"grants":{"<id>":{"version":1,"endpoint":"/store/capabilities/<id>","config":{}}}}
//! ```
//!
//! Base64 keeps non-ASCII configuration valid as an HTTP header value. The
//! parser deliberately rejects anything unexpected instead of returning a
//! partially valid grant map.

/** Trusted request header carrying the serialized capability grants. */
export const CAPABILITY_HEADER = "x-fastcart-capabilities";

/** One capability bound to the store for the current storefront render. */
export interface CapabilityGrant {
  /** Positive version of the capability the plugin currently provides. */
  version: number;
  /** Site-relative endpoint the theme calls through the store origin. */
  endpoint: string;
  /** Store-level configuration for the capability. */
  config: unknown;
}

/** Capability grants keyed by capability id. */
export type CapabilityGrants = Record<string, CapabilityGrant>;

/** The only envelope version this parser understands. */
const ENVELOPE_VERSION = 1;

/** Upper bound on the encoded header value, enforced before decoding. */
const MAX_ENCODED_LENGTH = 64 * 1024;

/**
 * Capability ids follow the store binding rules: lowercase ASCII letters,
 * digits, `-`, `.` and `_`, at most 120 characters, with no leading or
 * trailing `-`.
 */
const CAPABILITY_ID_PATTERN = /^[a-z0-9._-]{1,120}$/;

/** Base64url alphabet without padding. */
const BASE64URL_PATTERN = /^[A-Za-z0-9_-]*$/;

/**
 * Parses the trusted `x-fastcart-capabilities` header.
 *
 * Returns an empty object when the header is absent or empty. Throws an
 * `Error` when the header is present but malformed: an unsupported version
 * prefix, invalid base64url, invalid UTF-8 or JSON, an unsupported envelope,
 * or an invalid capability grant.
 */
export function parseCapabilityGrants(value: string | null | undefined): CapabilityGrants {
  if (value === null || value === undefined || value.length === 0) {
    return {};
  }
  if (value.length > MAX_ENCODED_LENGTH) {
    throw new Error(
      `invalid ${CAPABILITY_HEADER}: encoded value exceeds ${MAX_ENCODED_LENGTH} characters`,
    );
  }

  const separator = value.indexOf(".");
  if (separator === -1 || value.slice(0, separator) !== "1") {
    throw new Error(`unsupported ${CAPABILITY_HEADER} version prefix: expected "1."`);
  }
  const payload = decodeBase64Url(value.slice(separator + 1));

  let envelope: unknown;
  try {
    envelope = JSON.parse(payload);
  } catch {
    throw new Error(`invalid ${CAPABILITY_HEADER}: payload is not valid JSON`);
  }
  return parseEnvelope(envelope);
}

function parseEnvelope(envelope: unknown): CapabilityGrants {
  if (!isRecord(envelope)) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: envelope must be a JSON object`);
  }
  if (envelope.v !== ENVELOPE_VERSION) {
    throw new Error(
      `unsupported ${CAPABILITY_HEADER} envelope version: expected ${ENVELOPE_VERSION}`,
    );
  }
  if (!isRecord(envelope.grants)) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: "grants" must be a JSON object`);
  }

  const entries: Array<[string, CapabilityGrant]> = [];
  for (const [id, value] of Object.entries(envelope.grants)) {
    entries.push([id, parseGrant(id, value)]);
  }
  // Build a fresh object so callers never share the parsed envelope. Defining
  // entries also keeps ids such as "__proto__" as own data properties.
  return Object.fromEntries(entries);
}

function parseGrant(id: string, value: unknown): CapabilityGrant {
  if (!isValidCapabilityId(id)) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: invalid capability id "${id}"`);
  }
  if (!isRecord(value)) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: grant "${id}" must be a JSON object`);
  }
  if (typeof value.version !== "number" || !Number.isInteger(value.version) || value.version <= 0) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: grant "${id}" has an invalid version`);
  }
  if (typeof value.endpoint !== "string" || value.endpoint.length === 0) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: grant "${id}" has an invalid endpoint`);
  }
  if (!Object.prototype.hasOwnProperty.call(value, "config")) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: grant "${id}" is missing config`);
  }
  return {
    version: value.version,
    endpoint: value.endpoint,
    config: value.config,
  };
}

function decodeBase64Url(encoded: string): string {
  if (!BASE64URL_PATTERN.test(encoded) || encoded.length % 4 === 1) {
    throw new Error(`invalid ${CAPABILITY_HEADER}: payload is not valid base64url`);
  }

  const base64 = encoded.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64.padEnd(Math.ceil(base64.length / 4) * 4, "=");

  let bytes: Uint8Array;
  try {
    const binary = atob(padded);
    bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
  } catch {
    throw new Error(`invalid ${CAPABILITY_HEADER}: payload is not valid base64url`);
  }

  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`invalid ${CAPABILITY_HEADER}: payload is not valid UTF-8`);
  }
}

function isValidCapabilityId(value: string): boolean {
  return CAPABILITY_ID_PATTERN.test(value) && !value.startsWith("-") && !value.endsWith("-");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
