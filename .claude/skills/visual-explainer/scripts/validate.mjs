// Zero-dependency validator for the JSON Schema subset that the visual-explainer schemas use.
// An unsupported keyword throws, so a schema edit can never be silently unenforced.

const ANNOTATIONS = new Set(["$schema", "$id", "$defs", "$comment", "title", "description", "examples"]);
const KEYWORDS = new Set([
  "$ref", "type", "const", "enum", "required", "properties", "additionalProperties",
  "items", "minItems", "maxItems", "minLength", "maxLength", "pattern", "minimum", "maximum",
]);

// validate(schema, value) -> [{ path, message }] in document order; [] means valid.
export function validate(schema, value) {
  const errors = [];
  check(schema, value, "$", schema, errors);
  return errors;
}

export function formatErrors(errors) {
  return errors.map((e) => `${e.path}: ${e.message}`).join("\n");
}

function check(schema, value, at, root, errors) {
  for (const key of Object.keys(schema)) {
    if (!KEYWORDS.has(key) && !ANNOTATIONS.has(key)) throw new Error(`unsupported schema keyword "${key}" at ${at}`);
  }
  if (schema.$ref) return check(resolve(root, schema.$ref), value, at, root, errors);
  if (schema.type && !matchesType(schema.type, value)) {
    return errors.push({ path: at, message: `must be ${[].concat(schema.type).join(" or ")}, got ${typeName(value)}` });
  }
  if ("const" in schema && value !== schema.const) {
    return errors.push({ path: at, message: `must be ${JSON.stringify(schema.const)}` });
  }
  if (schema.enum && !schema.enum.includes(value)) {
    return errors.push({ path: at, message: `must be one of ${schema.enum.join(", ")}` });
  }
  if (typeof value === "string") checkString(schema, value, at, errors);
  if (typeof value === "number") checkNumber(schema, value, at, errors);
  if (Array.isArray(value)) checkArray(schema, value, at, root, errors);
  else if (isObject(value)) checkObject(schema, value, at, root, errors);
}

function resolve(root, ref) {
  const match = /^#\/\$defs\/([A-Za-z0-9_-]+)$/.exec(ref);
  const target = match && root.$defs?.[match[1]];
  if (!target) throw new Error(`unresolvable $ref "${ref}"`);
  return target;
}

function matchesType(type, value) {
  return [].concat(type).some((t) => {
    if (t === "integer") return Number.isInteger(value);
    if (t === "number") return typeof value === "number" && Number.isFinite(value);
    if (t === "array") return Array.isArray(value);
    if (t === "object") return isObject(value);
    if (t === "null") return value === null;
    return typeof value === t;
  });
}

function typeName(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  if (Number.isInteger(value)) return "integer";
  return typeof value;
}

function checkString(schema, value, at, errors) {
  if (schema.minLength !== undefined && value.length < schema.minLength) {
    errors.push({ path: at, message: schema.minLength === 1 ? "must not be empty" : `must have at least ${schema.minLength} characters` });
  }
  if (schema.maxLength !== undefined && value.length > schema.maxLength) {
    errors.push({ path: at, message: `must have at most ${schema.maxLength} characters` });
  }
  if (schema.pattern && !new RegExp(schema.pattern, "u").test(value)) {
    const hint = schema.description ? `. Expected: ${schema.description}` : "";
    errors.push({ path: at, message: `must match /${schema.pattern}/${hint}` });
  }
}

function checkNumber(schema, value, at, errors) {
  if (schema.minimum !== undefined && value < schema.minimum) errors.push({ path: at, message: `must be >= ${schema.minimum}` });
  if (schema.maximum !== undefined && value > schema.maximum) errors.push({ path: at, message: `must be <= ${schema.maximum}` });
}

function checkArray(schema, value, at, root, errors) {
  if (schema.minItems !== undefined && value.length < schema.minItems) {
    errors.push({ path: at, message: `must have at least ${schema.minItems} item(s)` });
  }
  if (schema.maxItems !== undefined && value.length > schema.maxItems) {
    errors.push({ path: at, message: `must have at most ${schema.maxItems} item(s)` });
  }
  if (schema.items) value.forEach((item, i) => check(schema.items, item, `${at}[${i}]`, root, errors));
}

function checkObject(schema, value, at, root, errors) {
  for (const name of schema.required ?? []) {
    if (!(name in value)) errors.push({ path: `${at}.${name}`, message: "required" });
  }
  for (const [name, child] of Object.entries(value)) {
    const sub = schema.properties?.[name];
    if (sub) check(sub, child, `${at}.${name}`, root, errors);
    else if (schema.additionalProperties === false) errors.push({ path: `${at}.${name}`, message: "unknown field" });
  }
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
