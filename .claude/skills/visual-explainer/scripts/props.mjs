// Shared props check of render.mjs and render-video.mjs: read the JSON, check the mode, then the
// schema of that mode.
import fs from "node:fs";
import path from "node:path";
import { validate } from "./validate.mjs";

const SCHEMA_DIR = path.join(import.meta.dirname, "..", "schema");
const MODES = ["plan", "ship"];

// checkProps(file) -> { props, errors }. `errors` is [] for valid props, else [{ path, message }].
// Throws an Error "cannot read <file>: <reason>" when the file cannot be read.
export function checkProps(file) {
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (err) {
    throw new Error(`cannot read ${file}: ${err.message}`);
  }
  let props;
  try {
    props = JSON.parse(text);
  } catch (err) {
    return { props: null, errors: [{ path: "$", message: `invalid JSON (${err.message})` }] };
  }
  // The mode selects the schema, so check it first: one clear error beats a cascade.
  if (!MODES.includes(props?.mode)) return { props, errors: [{ path: "$.mode", message: `must be one of ${MODES.join(", ")}` }] };
  return { props, errors: validate(loadSchema(props.mode), props) };
}

function loadSchema(mode) {
  return JSON.parse(fs.readFileSync(path.join(SCHEMA_DIR, `${mode}.schema.json`), "utf8"));
}
