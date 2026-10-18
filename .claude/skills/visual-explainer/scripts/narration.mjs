// Narration check: the schema, then the rules that need the props.
import fs from "node:fs";
import path from "node:path";
import { SECTION_IDS, sectionIds } from "../video/src/storyboard.mjs";
import { validate } from "./validate.mjs";

const SCHEMA = path.join(import.meta.dirname, "..", "schema", "narration.schema.json");
const MAX_TOTAL_CHARS = 9000;

// checkNarration(file, props) -> { narration, errors }. `props` must be valid. `errors` is [] for
// a valid narration, else [{ path, message }] with paths that start with "narration".
// Throws an Error "cannot read <file>: <reason>" when the file cannot be read.
export function checkNarration(file, props) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (err) {
    throw new Error(`cannot read ${file}: ${err.message}`);
  }
  let narration;
  try {
    narration = JSON.parse(text);
  } catch (err) {
    return { narration: null, errors: [{ path: "narration", message: `invalid JSON (${err.message})` }] };
  }
  const schemaErrors = validate(JSON.parse(fs.readFileSync(SCHEMA, "utf8")), narration);
  const errors = schemaErrors.length ? schemaErrors : propsErrors(narration, props);
  return { narration, errors: errors.map((e) => ({ ...e, path: e.path.replace(/^\$/, "narration") })) };
}

function propsErrors(narration, props) {
  if (narration.mode !== props.mode) return [{ path: "$.mode", message: `must be ${props.mode}, the mode of the props` }];
  const present = new Set(sectionIds(props));
  const seen = new Set();
  const errors = [];
  narration.sections.forEach(({ id }, i) => {
    const at = `$.sections[${i}].id`;
    if (!SECTION_IDS[props.mode].includes(id)) errors.push({ path: at, message: `"${id}" is not a ${props.mode} section` });
    else if (seen.has(id)) errors.push({ path: at, message: `"${id}" appears more than once` });
    else if (!present.has(id)) errors.push({ path: at, message: `the props have no ${id} section` });
    seen.add(id);
  });
  const total = narration.sections.reduce((n, s) => n + s.text.length, 0);
  if (total > MAX_TOTAL_CHARS) errors.push({ path: "$.sections", message: `has ${total} characters in total, the limit is ${MAX_TOTAL_CHARS}` });
  return errors;
}
