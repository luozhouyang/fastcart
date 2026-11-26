#!/usr/bin/env node
// OpenAPI -> admin SDK parity check.
//
// Compares the Rust API's OpenAPI document with the hand-written admin SDK so
// that request paths and DTO shapes cannot drift silently. The document comes
// from a checked-in snapshot or a running server; the script has no package
// dependencies, so CI can run it without installing the workspace.
//
// Usage:
//   node scripts/check-openapi-parity.mjs --file openapi.snapshot.json
//   OPENAPI_URL=http://127.0.0.1:8080/api/v1/openapi.json \
//     node scripts/check-openapi-parity.mjs
//   node scripts/check-openapi-parity.mjs --allow-missing /api/v1/admin/plugins
//
// Hard errors (non-zero exit):
//   - an SDK request path/method the OpenAPI document does not describe
//   - a request call whose path cannot be resolved statically
//
// Warnings (exit zero):
//   - admin operations documented in OpenAPI without an SDK method
//   - component schemas used by the operations the SDK calls with no matching
//     SDK type, by name, documented alias, or structural shape
//
// Exceptions belong in openapi-sdk-gaps.json next to this package's manifest:
// allowedMissingPaths and allowedMissingSchemas suppress findings, schemaAliases
// maps a renamed DTO, and ignoredAdminPaths silences operation families the SDK
// deliberately does not implement yet. Every entry carries a reason so stale
// entries are visible in review.
//
// Refresh the checked-in snapshot after API changes with:
//   FASTCART_WRITE_OPENAPI_SNAPSHOT=1 \
//     cargo test -p fastcart-api --test openapi writes_openapi_snapshot
//
// The pnpm script reads the snapshot by default:
//   pnpm --filter @fastcart/admin-sdk check:openapi --file openapi.snapshot.json

import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const PACKAGE_DIR = resolve(SCRIPT_DIR, "..");
const SDK_ENTRY = resolve(PACKAGE_DIR, "src/index.ts");
const SHARED_TYPES_ENTRY = resolve(PACKAGE_DIR, "../shared-types/src/index.ts");
const GAPS_FILE = resolve(PACKAGE_DIR, "openapi-sdk-gaps.json");
const DEFAULT_URL = "http://127.0.0.1:8080/api/v1/openapi.json";
const ADMIN_PREFIXES = ["/api/v1/admin", "/api/v1/platform"];
const HTTP_METHODS = new Set(["get", "put", "post", "delete", "patch", "head", "options", "trace"]);
const CLIENT_PLUMBING_TYPES = new Set(["AdminClient", "AdminClientOptions", "AdminRequestOptions"]);
const STRUCTURAL_MIN_PROPERTIES = 3;
const DEFAULT_LIST_LIMIT = 30;

// ---------------------------------------------------------------------------
// CLI and configuration
// ---------------------------------------------------------------------------

function parseArguments(argv) {
  const options = {
    file: null,
    url: process.env.OPENAPI_URL || DEFAULT_URL,
    allowMissing: [],
    verbose: false,
    help: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      options.help = true;
    } else if (argument === "--verbose") {
      options.verbose = true;
    } else if (argument === "--file") {
      options.file = requiredValue(argv, ++index, "--file");
    } else if (argument.startsWith("--file=")) {
      options.file = argument.slice("--file=".length);
    } else if (argument === "--allow-missing") {
      options.allowMissing.push(requiredValue(argv, ++index, "--allow-missing"));
    } else if (argument.startsWith("--allow-missing=")) {
      options.allowMissing.push(argument.slice("--allow-missing=".length));
    } else if (!argument.startsWith("-") && options.file === null) {
      // Positional form: check-openapi-parity.mjs <snapshot.json>
      options.file = argument;
    } else {
      throw new UsageError(`unknown argument: ${argument}`);
    }
  }
  return options;
}

function requiredValue(argv, index, flag) {
  const value = argv[index];
  if (value === undefined || value.startsWith("--")) {
    throw new UsageError(`${flag} requires a value`);
  }
  return value;
}

function helpText() {
  return `Usage: node packages/admin-sdk/scripts/check-openapi-parity.mjs [options]

Options:
  --file <path>            Read the OpenAPI document from a local JSON file.
  --allow-missing <prefix> Allow SDK routes or schema names with this prefix.
                           Repeatable. Merged with openapi-sdk-gaps.json.
  --verbose                Print every warning instead of the first
                           ${DEFAULT_LIST_LIMIT} per section.
  -h, --help               Show this help.

Environment:
  OPENAPI_URL              Document URL when --file is absent
                           (default ${DEFAULT_URL}).`;
}

async function loadDocument(options) {
  if (options.file !== null) {
    const path = resolve(process.cwd(), options.file);
    const text = await readFile(path, "utf8");
    return { document: JSON.parse(text), source: `file ${path}` };
  }

  const response = await fetch(options.url, { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`GET ${options.url} returned ${response.status} ${response.statusText}`);
  }
  return { document: await response.json(), source: `url ${options.url}` };
}

async function loadGaps() {
  const parsed = JSON.parse(await readFile(GAPS_FILE, "utf8"));
  const gaps = {
    allowedMissingPaths: parsed.allowedMissingPaths ?? [],
    allowedMissingSchemas: parsed.allowedMissingSchemas ?? [],
    ignoredAdminPaths: parsed.ignoredAdminPaths ?? [],
    schemaAliases: parsed.schemaAliases ?? [],
  };
  for (const entry of gaps.schemaAliases) {
    requireEntryFields(entry, "schemaAliases", ["schema"]);
    if (typeof entry.type !== "string" || entry.type.length === 0) {
      throw new UsageError(`schemaAliases entry needs a "type": ${JSON.stringify(entry)}`);
    }
  }
  for (const entry of gaps.allowedMissingPaths) {
    requireEntryFields(entry, "allowedMissingPaths", ["path", "pathPrefix"]);
  }
  for (const entry of gaps.allowedMissingSchemas) {
    requireEntryFields(entry, "allowedMissingSchemas", ["name", "namePrefix"]);
  }
  for (const entry of gaps.ignoredAdminPaths) {
    requireEntryFields(entry, "ignoredAdminPaths", ["path", "pathPrefix"]);
  }
  return gaps;
}

function requireEntryFields(entry, section, alternatives) {
  if (!alternatives.some((field) => typeof entry?.[field] === "string")) {
    throw new UsageError(
      `${section} entry needs one of ${alternatives.join("/")}: ${JSON.stringify(entry)}`,
    );
  }
  if (typeof entry.reason !== "string" || entry.reason.length === 0) {
    throw new UsageError(`${section} entry needs a "reason": ${JSON.stringify(entry)}`);
  }
}

class UsageError extends Error {}

// ---------------------------------------------------------------------------
// JavaScript scanning helpers
// ---------------------------------------------------------------------------

// Blanks out comments while keeping every other character in place, so offsets
// and line numbers stay aligned with the original source. Path examples in
// documentation and commented-out code can no longer look like live requests.
function stripComments(source) {
  let output = "";
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    if (character === '"' || character === "'" || character === "`") {
      const end = skipString(source, index);
      output += source.slice(index, end + 1);
      index = end;
    } else if (character === "/" && source[index + 1] === "/") {
      const newline = source.indexOf("\n", index);
      const stop = newline === -1 ? source.length : newline;
      output += " ".repeat(stop - index);
      index = stop - 1;
    } else if (character === "/" && source[index + 1] === "*") {
      const close = source.indexOf("*/", index + 2);
      const stop = close === -1 ? source.length : close + 2;
      output += source.slice(index, stop).replace(/[^\n]/g, " ");
      index = stop - 1;
    } else {
      output += character;
    }
  }
  return output;
}

// Returns the index of the quote closing the literal that starts at `start`.
// Template interpolation is traversed so nested braces stay consistent.
function skipString(source, start) {
  const quote = source[start];
  for (let index = start + 1; index < source.length; index += 1) {
    const character = source[index];
    if (character === "\\") {
      index += 1;
      continue;
    }
    if (character === quote) return index;
    if (quote === "`" && character === "$" && source[index + 1] === "{") {
      const end = findBalancedEnd(source, index + 2, "{", "}");
      if (end === -1) return index;
      index = end;
    }
  }
  return source.length - 1;
}

// Scans forward from `start` (just after an opening delimiter) to its matching
// closer, skipping string literals.
function findBalancedEnd(source, start, open, close) {
  let depth = 1;
  for (let index = start; index < source.length; index += 1) {
    const character = source[index];
    if (character === '"' || character === "'" || character === "`") {
      index = skipString(source, index);
      continue;
    }
    if (character === open) depth += 1;
    else if (character === close) {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return -1;
}

// ---------------------------------------------------------------------------
// SDK request paths
// ---------------------------------------------------------------------------

// Finds `request<T>(...)` calls and returns each argument list plus the HTTP
// method inferred from the options object (GET when absent).
function findRequestCalls(source) {
  const calls = [];
  const pattern = /(?<![\w.$])request\s*</g;
  while (pattern.exec(source) !== null) {
    let depth = 1;
    let cursor = pattern.lastIndex;
    for (; cursor < source.length; cursor += 1) {
      if (source[cursor] === "<") depth += 1;
      else if (source[cursor] === ">") {
        depth -= 1;
        if (depth === 0) break;
      }
    }
    if (depth !== 0) continue;

    let open = cursor + 1;
    while (open < source.length && /\s/.test(source[open])) open += 1;
    if (source[open] !== "(") continue;

    const close = findBalancedEnd(source, open + 1, "(", ")");
    if (close === -1) continue;

    const arguments_ = source.slice(open + 1, close);
    calls.push({
      arguments: arguments_,
      method: inferMethod(arguments_),
      start: open + 1,
      end: close,
    });
    pattern.lastIndex = close;
  }
  return calls;
}

function inferMethod(arguments_) {
  const match = /(?:^|[^\w$])method\s*:\s*(["'`])([A-Za-z]+)\1/.exec(arguments_);
  return match === null ? "GET" : match[2].toUpperCase();
}

// Reads the first top-level argument of a call.
function firstArgument(arguments_) {
  let depth = 0;
  for (let index = 0; index < arguments_.length; index += 1) {
    const character = arguments_[index];
    if (character === '"' || character === "'" || character === "`") {
      index = skipString(arguments_, index);
      continue;
    }
    if (character === "(" || character === "[" || character === "{") depth += 1;
    else if (character === ")" || character === "]" || character === "}") depth -= 1;
    else if (character === "," && depth === 0) return arguments_.slice(0, index).trim();
  }
  return arguments_.trim();
}

// Converts a string or template literal into a path with `{}` for every
// interpolation. Returns null when the expression is not a literal.
function pathFromLiteral(raw) {
  if (raw.startsWith('"') || raw.startsWith("'")) {
    return raw.length >= 2 && raw.endsWith(raw[0]) ? raw.slice(1, -1) : null;
  }
  if (!raw.startsWith("`") || !raw.endsWith("`")) return null;
  let path = "";
  for (let index = 1; index < raw.length; index += 1) {
    const character = raw[index];
    if (character === "\\") {
      path += raw[index + 1] ?? "";
      index += 1;
    } else if (character === "`") {
      return path;
    } else if (character === "$" && raw[index + 1] === "{") {
      const end = findBalancedEnd(raw, index + 2, "{", "}");
      if (end === -1) return null;
      path += "{}";
      index = end;
    } else {
      path += character;
    }
  }
  return null;
}

// Parameter names differ between Rust (`return_id`) and the SDK (`returnId`);
// only the segment positions matter for parity.
function normalizePath(path) {
  return path
    .split("/")
    .map((segment) => (segment.startsWith("{") && segment.endsWith("}") ? "{}" : segment))
    .join("/");
}

function extractSdkRoutes(source) {
  const routes = new Set();
  const unresolved = [];
  const calls = findRequestCalls(source);
  for (const call of calls) {
    const raw = firstArgument(call.arguments);
    // The client's own generic helper is declared as `request<T>(path: string, ...)`.
    if (/^[A-Za-z_$][\w$]*\s*:\s*[A-Za-z_(]/.test(raw)) continue;
    const path = pathFromLiteral(raw);
    if (path === null || !path.startsWith("/api/v1/")) {
      unresolved.push(raw.replace(/\s+/g, " ").slice(0, 120));
      continue;
    }
    routes.add(`${call.method} ${normalizePath(path)}`);
  }

  // URL helpers such as `assetUrl` and `fileUrl` return API paths directly
  // instead of calling `request`; clients fetch them with GET, so they must
  // exist on the server too. Literals already counted above are skipped so no
  // operation is reported twice under a different method.
  const helperPattern = /`(\/api\/v1\/[^`]*)`/g;
  let helper;
  while ((helper = helperPattern.exec(source)) !== null) {
    if (calls.some((call) => helper.index >= call.start && helper.index < call.end)) continue;
    const path = pathFromLiteral(helper[0]);
    if (path !== null) routes.add(`GET ${normalizePath(path)}`);
  }

  return { routes, unresolved };
}

// ---------------------------------------------------------------------------
// TypeScript declarations
// ---------------------------------------------------------------------------

// Splits a declaration body on top-level separators, ignoring separators that
// belong to generics, tuples, or nested object types.
function splitTopLevel(text, separators) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    if (character === '"' || character === "'" || character === "`") {
      index = skipString(text, index);
      continue;
    }
    if (character === "{" || character === "(" || character === "[") depth += 1;
    else if (character === "}" || character === ")" || character === "]") depth -= 1;
    else if (character === "<" && /[\w$]/.test(text[index - 1] ?? "")) depth += 1;
    else if (character === ">" && text[index - 1] !== "=") depth -= 1;
    else if (depth === 0 && separators.includes(character)) {
      parts.push(text.slice(start, index));
      start = index + 1;
    }
  }
  parts.push(text.slice(start));
  return parts;
}

// Reads the property names of an object type body. Methods, index signatures
// and call signatures are skipped because they never describe a DTO payload.
function objectProperties(body) {
  const properties = new Set();
  for (const member of splitTopLevel(body, [";", ","])) {
    const match = /^(?:\s*(?:readonly\s+)?)(?:["']([^"']+)["']|([A-Za-z_$][\w$]*))\s*\??\s*:/.exec(
      member,
    );
    if (match !== null) properties.add(match[1] ?? match[2]);
  }
  return properties;
}

// Reads a union of string literals, ignoring `null`. Returns null when the
// declaration is not a pure literal union.
function literalUnion(text) {
  const parts = splitTopLevel(text, ["|"]);
  const values = [];
  for (const part of parts) {
    const token = part.trim();
    if (token === "" || token === "null") continue;
    const match = /^(["'])((?:\\.|(?!\1).)*)\1$/.exec(token);
    if (match === null) return null;
    values.push(match[2]);
  }
  return values.length > 0 ? values.sort() : null;
}

// Collects exported object shapes and literal unions. Parents named in
// `extends` clauses are merged so that inherited DTO fields are compared too.
function collectTypeShapes(source) {
  const interfaceHeaders = [];
  const aliasBodies = [];
  const patterns = [
    {
      regex: /^export\s+interface\s+([A-Za-z_$][\w$]*)(?:\s+extends\s+([^{]+))?\s*\{/gm,
      kind: "interface",
    },
    { regex: /^export\s+type\s+([A-Za-z_$][\w$]*)\s*=\s*/gm, kind: "alias" },
  ];

  for (const { regex, kind } of patterns) {
    let match;
    while ((match = regex.exec(source)) !== null) {
      const name = match[1];
      if (kind === "interface") {
        const bodyStart = regex.lastIndex;
        const bodyEnd = findBalancedEnd(source, bodyStart, "{", "}");
        if (bodyEnd === -1) continue;
        const parents = (match[2] ?? "")
          .split(",")
          .map((parent) => parent.trim())
          .filter((parent) => /^[A-Za-z_$][\w$]*$/.test(parent));
        interfaceHeaders.push({
          name,
          parents,
          properties: objectProperties(source.slice(bodyStart, bodyEnd)),
        });
        regex.lastIndex = bodyEnd + 1;
      } else {
        const declarationStart = regex.lastIndex;
        let index = declarationStart;
        let depth = 0;
        for (; index < source.length; index += 1) {
          const character = source[index];
          if (character === '"' || character === "'" || character === "`") {
            index = skipString(source, index);
            continue;
          }
          if (character === "{" || character === "(" || character === "[") depth += 1;
          else if (character === "}" || character === ")" || character === "]") depth -= 1;
          else if (character === ";" && depth === 0) break;
        }
        aliasBodies.push({ name, body: source.slice(declarationStart, index) });
        regex.lastIndex = index;
      }
    }
  }

  const byName = new Map(interfaceHeaders.map((entry) => [entry.name, entry]));
  const objectShapes = new Map(); // signature -> [type names]
  const enumShapes = new Map(); // signature -> [type names]

  const propertiesOf = (entry, seen = new Set()) => {
    if (seen.has(entry.name)) return new Set();
    seen.add(entry.name);
    const properties = new Set(entry.properties);
    for (const parentName of entry.parents) {
      const parent = byName.get(parentName);
      if (parent === undefined) continue;
      for (const property of propertiesOf(parent, seen)) properties.add(property);
    }
    return properties;
  };

  for (const entry of interfaceHeaders) {
    if (CLIENT_PLUMBING_TYPES.has(entry.name)) continue;
    const properties = propertiesOf(entry);
    addShape(objectShapes, [...properties].sort(), entry.name);
  }

  for (const { name, body } of aliasBodies) {
    if (CLIENT_PLUMBING_TYPES.has(name)) continue;
    const trimmed = body.trim();
    if (trimmed.startsWith("{")) {
      const end = findBalancedEnd(trimmed, 1, "{", "}");
      if (end === -1) continue;
      addShape(objectShapes, [...objectProperties(trimmed.slice(1, end))].sort(), name);
    } else {
      const values = literalUnion(trimmed);
      if (values !== null) addShape(enumShapes, values, name);
    }
  }

  return { objectShapes, enumShapes };
}

function addShape(shapes, keys, name) {
  if (keys.length === 0) return;
  const signature = JSON.stringify(keys);
  const existing = shapes.get(signature);
  if (existing === undefined) shapes.set(signature, [name]);
  else existing.push(name);
}

function extractSdkTypes(sdkSource, sharedTypesSource) {
  const names = new Set();
  const exported = /^export\s+(?:interface|type)\s+([A-Za-z0-9_$]+)/gm;
  for (const source of [sdkSource, sharedTypesSource]) {
    let match;
    while ((match = exported.exec(source)) !== null) names.add(match[1]);
    exported.lastIndex = 0;
  }

  // DTOs the SDK re-exports from the shared type package count as SDK types
  // even when the alias itself is not declared in index.ts.
  const importPattern = /import\s+type\s*\{([\s\S]*?)\}\s*from\s*["']@fastcart\/shared-types["']/g;
  let match;
  while ((match = importPattern.exec(sdkSource)) !== null) {
    for (const entry of match[1].split(",")) {
      const name = entry
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/\/\/.*$/gm, "")
        .trim()
        .split(/\s+as\s+/)[0]
        .trim();
      if (/^[A-Za-z0-9_$]+$/.test(name)) names.add(name);
    }
  }

  for (const name of CLIENT_PLUMBING_TYPES) names.delete(name);
  return names;
}

// ---------------------------------------------------------------------------
// OpenAPI extraction and matching
// ---------------------------------------------------------------------------

function extractOpenApi(document) {
  const paths = document?.paths;
  if (paths === null || typeof paths !== "object") {
    throw new Error("OpenAPI document has no paths object");
  }
  const routes = new Set();
  for (const [path, item] of Object.entries(paths)) {
    if (item === null || typeof item !== "object") continue;
    for (const key of Object.keys(item)) {
      if (HTTP_METHODS.has(key.toLowerCase())) {
        routes.add(`${key.toUpperCase()} ${normalizePath(path)}`);
      }
    }
  }
  return {
    routes,
    pathCount: Object.keys(paths).length,
    schemas: new Map(
      Object.entries(document?.components?.schemas ?? {}).map(([name, schema]) => [name, schema]),
    ),
  };
}

// Collects schema names reachable from a single operation, following
// component-to-component references transitively.
function operationSchemas(document, operation) {
  const referenced = new Set();
  const stack = [operation];
  while (stack.length > 0) {
    const value = stack.pop();
    if (Array.isArray(value)) {
      stack.push(...value);
    } else if (value !== null && typeof value === "object") {
      const reference = value.$ref;
      if (typeof reference === "string" && reference.startsWith("#/components/schemas/")) {
        const name = reference.slice("#/components/schemas/".length);
        if (!referenced.has(name)) {
          referenced.add(name);
          stack.push(document.components?.schemas?.[name]);
        }
      } else {
        stack.push(...Object.values(value));
      }
    }
  }
  return referenced;
}

// Maps `METHOD /normalized/path` to the schemas that operation uses. Schemas
// referenced only by storefront or webhook operations never enter the map.
function schemasByOperation(document, prefixes) {
  const byOperation = new Map();
  for (const [path, item] of Object.entries(document?.paths ?? {})) {
    if (!prefixes.some((prefix) => path.startsWith(prefix))) continue;
    for (const [key, operation] of Object.entries(item ?? {})) {
      if (!HTTP_METHODS.has(key.toLowerCase())) continue;
      const pair = `${key.toUpperCase()} ${normalizePath(path)}`;
      byOperation.set(pair, operationSchemas(document, operation));
    }
  }
  return byOperation;
}

function schemaCandidates(name) {
  const candidates = new Set([name]);
  for (const suffix of ["Dto", "Request", "Response", "Envelope"]) {
    if (name.endsWith(suffix) && name.length > suffix.length) {
      candidates.add(name.slice(0, -suffix.length));
    } else {
      // A handler may document `CouponValidation` while the SDK names the
      // response `CouponValidationResponse`.
      candidates.add(`${name}Response`);
    }
  }
  // utoipa names generic bodies `PageResponse_Product`; the SDK models those
  // as `PageResult<Product>`.
  const generic = /^PageResponse_(.+)$/.exec(name);
  if (generic !== null) {
    candidates.add("PageResult");
    candidates.add(generic[1]);
  }
  return candidates;
}

// True for identifier aliases such as `OrderId = String` and the primitives
// they alias. They carry no fields, so the SDK represents them with `string`
// or `number` rather than a named type; there is nothing to drift.
function isPrimitiveAlias(schema) {
  if (schema === null || typeof schema !== "object") return false;
  const hasProperties = Object.keys(schema.properties ?? {}).length > 0;
  if (hasProperties || schema.enum !== undefined || schema.oneOf !== undefined) return false;
  return ["string", "integer", "number", "boolean"].includes(schema.type);
}

// Flattens `allOf` composition (utoipa emits it for DTO inheritance) into one
// property set so that `CreatedPublishableKeyDto` compares against the SDK's
// `CreatedPublishableApiKey extends PublishableApiKey`.
function flattenObjectProperties(document, schema, seen = new Set()) {
  if (schema === null || typeof schema !== "object") return null;
  if (Array.isArray(schema.allOf)) {
    const properties = new Set();
    for (const part of schema.allOf) {
      const flattened = resolveSchema(document, part, seen);
      if (flattened === null) continue;
      for (const property of flattened) properties.add(property);
    }
    return properties;
  }
  const keys = Object.keys(schema.properties ?? {});
  return keys.length > 0 ? new Set(keys) : null;
}

function resolveSchema(document, schema, seen) {
  if (schema === null || typeof schema !== "object") return null;
  const reference = schema.$ref;
  if (typeof reference === "string" && reference.startsWith("#/components/schemas/")) {
    const name = reference.slice("#/components/schemas/".length);
    if (seen.has(name)) return null;
    seen.add(name);
    return flattenObjectProperties(document, document.components?.schemas?.[name], seen);
  }
  return flattenObjectProperties(document, schema, seen);
}

function matchSchema(document, name, schema, types, shapes, aliases) {
  const { objectShapes, enumShapes } = shapes;
  for (const candidate of schemaCandidates(name)) {
    if (types.has(candidate)) return { matchedBy: "name" };
  }

  // Deliberate renames are declared in openapi-sdk-gaps.json so the mapping is
  // reviewable instead of inferred from similar-looking names.
  const alias = aliases.get(name);
  if (alias !== undefined && types.has(alias)) return { matchedBy: "alias", type: alias };

  const enumValues = Array.isArray(schema?.enum) ? schema.enum : null;
  if (enumValues !== null) {
    const values = enumValues
      .filter((value) => value !== null)
      .map(String)
      .sort();
    const names = enumShapes.get(JSON.stringify(values));
    if (names !== undefined && names.length > 0) return { matchedBy: "shape", type: names[0] };
  }

  const properties = flattenObjectProperties(document, schema);
  if (properties !== null && properties.size >= STRUCTURAL_MIN_PROPERTIES) {
    const signature = JSON.stringify([...properties].sort());
    const names = objectShapes.get(signature);
    if (names !== undefined && names.length > 0) return { matchedBy: "shape", type: names[0] };
  }

  if (isPrimitiveAlias(schema)) return { matchedBy: "primitive" };
  return null;
}

// ---------------------------------------------------------------------------
// Allowlist
// ---------------------------------------------------------------------------

function pathEntryMatches(entry, pair) {
  const [method, path] = pair.split(" ");
  const methodMatches = entry.method === undefined || entry.method.toUpperCase() === method;
  if (!methodMatches) return false;
  if (typeof entry.path === "string" && entry.path === path) return true;
  return typeof entry.pathPrefix === "string" && path.startsWith(entry.pathPrefix);
}

function isPathAllowed(pair, prefixes, entries) {
  const path = pair.slice(pair.indexOf(" ") + 1);
  for (const prefix of prefixes) {
    if (path.startsWith(prefix) || pair.startsWith(prefix)) return true;
  }
  return entries.some((entry) => pathEntryMatches(entry, pair));
}

function isSchemaAllowed(name, prefixes, entries) {
  for (const prefix of prefixes) {
    if (name.startsWith(prefix)) return true;
  }
  return entries.some(
    (entry) =>
      (typeof entry.name === "string" && entry.name === name) ||
      (typeof entry.namePrefix === "string" && name.startsWith(entry.namePrefix)),
  );
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

function printSection(title, lines, options) {
  if (lines.length === 0) return;
  console.log(`\n${title} (${lines.length})`);
  const limit = options.verbose ? lines.length : DEFAULT_LIST_LIMIT;
  for (const line of lines.slice(0, limit)) console.log(`  - ${line}`);
  if (lines.length > limit) {
    console.log(`  ... and ${lines.length - limit} more (rerun with --verbose for the full list)`);
  }
}

function relativize(path) {
  return path.replace(`${process.cwd()}/`, "");
}

async function run(options) {
  const [{ document, source }, gaps] = await Promise.all([loadDocument(options), loadGaps()]);
  const openapi = extractOpenApi(document);
  const sdkSource = stripComments(await readFile(SDK_ENTRY, "utf8"));
  const sharedTypesSource = stripComments(await readFile(SHARED_TYPES_ENTRY, "utf8"));
  const sdk = extractSdkRoutes(sdkSource);
  const sdkTypes = extractSdkTypes(sdkSource, sharedTypesSource);
  const shapes = collectTypeShapes(sharedTypesSource);
  const sdkShapes = collectTypeShapes(sdkSource);
  for (const kind of ["objectShapes", "enumShapes"]) {
    for (const [key, names] of sdkShapes[kind]) {
      shapes[kind].set(key, [...(shapes[kind].get(key) ?? []), ...names]);
    }
  }

  // Hard errors: the SDK calls an operation the API does not document.
  const sdkMissing = [];
  const allowedPaths = [];
  for (const pair of [...sdk.routes].sort()) {
    if (openapi.routes.has(pair)) continue;
    if (isPathAllowed(pair, options.allowMissing, gaps.allowedMissingPaths)) {
      allowedPaths.push(pair);
    } else {
      sdkMissing.push(pair);
    }
  }

  // Warning: an admin operation exists but the SDK has no method for it yet.
  // Families listed under ignoredAdminPaths are deliberate gaps with a reason
  // (for example routes owned by parallel work) and are reported separately.
  const undocumentedAdmin = [];
  const ignoredAdmin = [];
  for (const pair of [...openapi.routes].sort()) {
    const path = pair.slice(pair.indexOf(" ") + 1);
    if (!path.startsWith("/api/v1/admin")) continue;
    if (sdk.routes.has(pair)) continue;
    if (gaps.ignoredAdminPaths.some((entry) => pathEntryMatches(entry, pair))) {
      ignoredAdmin.push(pair);
    } else {
      undocumentedAdmin.push(pair);
    }
  }

  // Only schemas behind operations the SDK actually calls are in scope: an
  // endpoint the SDK has not implemented yet is already reported above, and
  // its DTOs would otherwise repeat the same gap once per schema.
  const byOperation = schemasByOperation(document, ADMIN_PREFIXES);
  const inScopeSchemas = new Set();
  for (const pair of sdk.routes) {
    for (const name of byOperation.get(pair) ?? []) inScopeSchemas.add(name);
  }
  const aliases = new Map(gaps.schemaAliases.map((entry) => [entry.schema, entry.type]));
  const schemasWithoutTypes = [];
  const schemasMatchedIndirectly = [];
  const allowedSchemas = [];
  for (const name of [...inScopeSchemas].sort()) {
    const schema = openapi.schemas.get(name);
    if (schema === undefined) continue;
    const match = matchSchema(document, name, schema, sdkTypes, shapes, aliases);
    if (match === null) {
      if (isSchemaAllowed(name, options.allowMissing, gaps.allowedMissingSchemas)) {
        allowedSchemas.push(name);
      } else {
        schemasWithoutTypes.push(name);
      }
    } else if (match.matchedBy !== "name" && match.matchedBy !== "primitive") {
      schemasMatchedIndirectly.push(`${name} ~ ${match.type}`);
    }
  }

  console.log("OpenAPI -> admin SDK parity check");
  console.log(`  document: ${source}`);
  console.log(
    `  openapi: ${openapi.pathCount} paths, ${openapi.routes.size} operations, ` +
      `${openapi.schemas.size} schemas (${inScopeSchemas.size} used by SDK operations)`,
  );
  console.log(
    `  sdk: ${sdk.routes.size} request paths, ${sdkTypes.size} exported types, ` +
      `${schemasMatchedIndirectly.length} schemas matched by alias or shape`,
  );

  printSection("SDK paths missing from OpenAPI (hard error)", sdkMissing, options);
  if (sdk.unresolved.length > 0) {
    printSection("Request calls with an unresolvable path (hard error)", sdk.unresolved, options);
  }
  printSection("Admin operations missing from the SDK (warning)", undocumentedAdmin, options);
  printSection("Schemas without a matching SDK type (warning)", schemasWithoutTypes, options);
  printSection("Allowlisted exceptions", [...allowedPaths, ...allowedSchemas].sort(), options);
  if (options.verbose) {
    printSection(
      "Ignored admin operations (documented in openapi-sdk-gaps.json)",
      ignoredAdmin,
      options,
    );
    printSection(
      "Schemas matched by alias or shape (renamed DTOs)",
      schemasMatchedIndirectly,
      options,
    );
  }

  const hardErrors = sdkMissing.length + sdk.unresolved.length;
  console.log(
    `\nSummary: ${hardErrors} hard error(s), ${undocumentedAdmin.length} openapi-without-sdk ` +
      `warning(s), ${schemasWithoutTypes.length} schema warning(s), ` +
      `${allowedPaths.length + allowedSchemas.length} allowlisted, ` +
      `${ignoredAdmin.length} ignored.`,
  );

  if (hardErrors > 0) {
    console.error(
      `\nFix the SDK path, correct the OpenAPI document, or document the exception in ` +
        `${relativize(GAPS_FILE)} or --allow-missing.`,
    );
    return 1;
  }
  return 0;
}

async function main() {
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    console.error(`check-openapi-parity: ${error.message}`);
    console.error(helpText());
    return 2;
  }
  if (options.help) {
    console.log(helpText());
    return 0;
  }
  try {
    return await run(options);
  } catch (error) {
    if (error instanceof UsageError) {
      console.error(`check-openapi-parity: ${error.message}`);
      return 2;
    }
    console.error(`check-openapi-parity: ${error instanceof Error ? error.message : error}`);
    return 2;
  }
}

main().then((code) => process.exit(code));
