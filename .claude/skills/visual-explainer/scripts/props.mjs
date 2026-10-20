// Shared props check of render.mjs and render-video.mjs: read the JSON, check the mode, then the
// schema of that mode, then the ids in the relation fields (graph.mjs relationErrors), then the
// fields of the report page (one sequence diagram at most, its participants, the media files).
import fs from "node:fs";
import path from "node:path";
import { relationErrors } from "./graph.mjs";
import { validate } from "./validate.mjs";

const SCHEMA_DIR = path.join(import.meta.dirname, "..", "schema");
const MODES = ["plan", "ship"];
// The report inlines each image as a data URI. The artifact page limit is 16 MB in total.
const MAX_MEDIA_BYTES = 2 * 1024 * 1024;

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
  const errors = validate(loadSchema(props.mode), props);
  if (errors.length) return { props, errors };
  return { props, errors: [...relationErrors(props), ...reportErrors(props, path.dirname(path.resolve(file)))] };
}

// reportErrors(props, dir) -> [{ path, message }] for schema-valid props. `dir` is the directory of
// the props file: media paths are relative to it.
export function reportErrors(props, dir) {
  const errors = [];
  const add = (at, message) => errors.push({ path: at, message });
  if (props.mode === "ship") {
    checkpointErrors(props, add);
    sequenceErrors(props.changes ?? [], add);
  }
  const areas = new Set((props.changes ?? []).map((c) => c.area));
  (props.media ?? []).forEach((m, i) => {
    const at = `$.media[${i}]`;
    if (m.path.split("/").includes("..")) add(`${at}.path`, 'must not have a ".." segment');
    else mediaFileError(path.join(dir, m.path), (message) => add(`${at}.path`, message));
    if (m.change !== undefined && !areas.has(m.change)) add(`${at}.change`, `no change with the area ${JSON.stringify(m.change)}`);
  });
  return errors;
}

// A checkpoint report may have no review yet. Any other ship report needs one.
function checkpointErrors(props, add) {
  const cp = props.meta.checkpoint;
  if (!cp && !props.reviews.length) add("$.reviews", "must have at least 1 item (only a checkpoint report, with meta.checkpoint, may have none)");
  if (cp && cp.phasesDone > cp.phasesTotal) add("$.meta.checkpoint.phasesDone", `must be <= phasesTotal (${cp.phasesTotal})`);
}

function sequenceErrors(changes, add) {
  const withSequence = changes.map((c, i) => (c.sequence ? i : -1)).filter((i) => i >= 0);
  withSequence.slice(1).forEach((i) => add(`$.changes[${i}].sequence`, `only one change may have a sequence; $.changes[${withSequence[0]}] has one already`));
  for (const ci of withSequence) {
    const { participants, messages } = changes[ci].sequence;
    const at = `$.changes[${ci}].sequence`;
    const names = new Set();
    participants.forEach((p, i) => {
      if (names.has(p.name)) add(`${at}.participants[${i}].name`, `${JSON.stringify(p.name)} appears more than once`);
      names.add(p.name);
    });
    messages.forEach((m, i) => {
      for (const end of ["from", "to"]) {
        if (!names.has(m[end])) add(`${at}.messages[${i}].${end}`, `no participant ${JSON.stringify(m[end])}`);
      }
      if (m.kind === "self" && m.from !== m.to) add(`${at}.messages[${i}].to`, "a self message must have to equal to from");
    });
  }
}

function mediaFileError(file, report) {
  let stat;
  try {
    stat = fs.statSync(file);
  } catch {
    return report(`no file at ${file}`);
  }
  if (!stat.isFile()) report(`${file} is not a file`);
  else if (stat.size > MAX_MEDIA_BYTES) report(`${file} has ${stat.size} bytes; the limit is ${MAX_MEDIA_BYTES}`);
}

function loadSchema(mode) {
  return JSON.parse(fs.readFileSync(path.join(SCHEMA_DIR, `${mode}.schema.json`), "utf8"));
}
