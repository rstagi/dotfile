#!/usr/bin/env node

const API_URL = process.env.TYPESAFE_API_URL || "https://api.typesafe.ai/v1/systemone";
const MAX_INPUT_BYTES = 64 * 1024;
const MODEL = "jev-latest";
const VERSION = 1;

await run();

async function run() {
  let input;
  try {
    input = parseInput(await readStdin());
    validateInput(input);
    if (!process.env.TYPESAFE_API_KEY) {
      writeFallback(input, "missing_credentials");
      return;
    }
    if (parseMode(process.env.LOOP_JEV_MODE, true) === "off") {
      writeFallback(input, "disabled");
      return;
    }
    await main(input);
  } catch (error) {
    const reason = error?.reason
      || (["AbortError", "TimeoutError"].includes(error?.name) ? "timeout" : "api_error");
    writeFallback(input, reason);
  }
}

async function main(input) {
  const mode = parseMode(process.env.LOOP_JEV_MODE, Boolean(process.env.TYPESAFE_API_KEY));
  const response = await fetch(API_URL, {
    method: "POST",
    headers: {
      authorization: `Bearer ${process.env.TYPESAFE_API_KEY}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({ state: input.state, model: MODEL, questions: input.questions }),
    signal: AbortSignal.timeout(parseTimeout(process.env.LOOP_JEV_TIMEOUT_MS)),
  });
  if (!response.ok) throw fallbackError("api_error");
  let body;
  try {
    body = await response.json();
  } catch (error) {
    if (["AbortError", "TimeoutError"].includes(error?.name)) throw error;
    throw fallbackError("invalid_response");
  }
  if (!isRecord(body)) throw fallbackError("invalid_response");
  const answers = validateAnswers(input.questions, body.answers);
  if (!isNonEmptyString(body.model)
    || !isNonNegativeInteger(body.usage?.input_tokens)
    || !isNonNegativeInteger(body.usage?.output_tokens)) {
    throw fallbackError("invalid_response");
  }
  const confidences = Object.values(answers).map((answer) => answer.confidence);
  const confidence = Math.min(...confidences);
  const result = {
    version: VERSION,
    status: confidence < parseThreshold(input.stage) ? "fallback" : "ok",
    stage: input.stage,
    mode,
    model: body.model,
    answers,
    confidence,
    usage: {
      inputTokens: body.usage.input_tokens,
      outputTokens: body.usage.output_tokens,
    },
  };
  if (result.status === "fallback") result.reason = "low_confidence";

  process.stdout.write(`${JSON.stringify(result)}\n`);
}

function writeFallback(input, reason) {
  process.stdout.write(`${JSON.stringify({
    version: VERSION,
    status: "fallback",
    stage: input?.stage ?? null,
    mode: parseMode(process.env.LOOP_JEV_MODE, Boolean(process.env.TYPESAFE_API_KEY)),
    reason,
  })}\n`);
}

function parseMode(value, hasKey) {
  if (!value) return hasKey ? "shadow" : "off";
  return ["off", "shadow", "active"].includes(value) ? value : "off";
}

function parseTimeout(value) {
  const timeout = Number.parseInt(value || "5000", 10);
  return Math.min(Math.max(timeout, 25), 10_000);
}

function parseThreshold(stage) {
  const name = `LOOP_JEV_${stage.replace("-", "_").toUpperCase()}_MIN_CONFIDENCE`;
  const threshold = Number.parseFloat(process.env[name] || "0.8");
  return Number.isFinite(threshold) && threshold >= 0 && threshold <= 1 ? threshold : 0.8;
}

function validateInput(input) {
  if (!isRecord(input)
    || !["route", "question", "merge-risk"].includes(input.stage)
    || !isState(input.state)
    || !isRecord(input.questions)
    || Object.keys(input.questions).length === 0) {
    throw fallbackError("invalid_input");
  }
  for (const [id, question] of Object.entries(input.questions)) {
    if (!id || !isRecord(question) || !isStructuredValue(question.instructions)) {
      throw fallbackError("invalid_input");
    }
    if (question.type === "choice") {
      const criteria = question.criteria;
      if (!isRecord(criteria) || Object.keys(criteria).length < 2 || Object.keys(criteria).length > 255
        || Object.values(criteria).some((value) => value !== null && !isStructuredValue(value))) {
        throw fallbackError("invalid_input");
      }
    } else if (question.type === "score") {
      if (!Array.isArray(question.criteria) || question.criteria.length < 2
        || question.criteria.length > 10 || question.criteria.some((value) => !isStructuredValue(value))) {
        throw fallbackError("invalid_input");
      }
    } else if (question.type === "noul") {
      if (question.criteria !== undefined && (!isRecord(question.criteria)
        || Object.keys(question.criteria).some((key) => !["true", "false"].includes(key))
        || Object.values(question.criteria).some((value) => !isStructuredValue(value)))) {
        throw fallbackError("invalid_input");
      }
    } else {
      throw fallbackError("invalid_input");
    }
  }
}

function validateAnswers(questions, answers) {
  if (!isRecord(answers)) throw fallbackError("invalid_response");
  return Object.fromEntries(Object.entries(questions).map(([id, question]) => {
    const answer = answers[id];
    if (!isRecord(answer) || answer.type !== question.type) {
      throw fallbackError("invalid_response");
    }
    if (answer.type === "noul" && isProbability(answer.noul)) {
      return [id, { type: "noul", noul: answer.noul, confidence: roundConfidence(answer.noul) }];
    }
    if (answer.type === "choice"
      && Object.hasOwn(question.criteria, answer.choice)
      && isProbability(answer.confidence)
      && isDistribution(answer.probabilities, Object.keys(question.criteria))) {
      return [id, {
        type: "choice",
        choice: answer.choice,
        probabilities: answer.probabilities,
        confidence: answer.confidence,
      }];
    }
    if (answer.type === "score"
      && isFiniteNumber(answer.score)
      && answer.score >= 0
      && answer.score <= question.criteria.length - 1
      && isProbability(answer.confidence)
      && isDistribution(answer.probabilities, question.criteria.map((_, index) => String(index)))
      && isRecord(answer.legend)) {
      return [id, {
        type: "score",
        score: answer.score,
        probabilities: answer.probabilities,
        confidence: answer.confidence,
      }];
    }
    throw fallbackError("invalid_response");
  }));
}

function roundConfidence(noul) {
  return Math.round(Math.abs(noul - 0.5) * 200) / 100;
}

async function readStdin() {
  let input = "";
  process.stdin.setEncoding("utf8");
  for await (const chunk of process.stdin) {
    input += chunk;
    if (Buffer.byteLength(input, "utf8") > MAX_INPUT_BYTES) {
      throw fallbackError("input_too_large");
    }
  }
  return input;
}

function parseInput(serialized) {
  try {
    return JSON.parse(serialized);
  } catch {
    throw fallbackError("invalid_input");
  }
}

function fallbackError(reason) {
  return Object.assign(new Error(reason), { reason });
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isState(value) {
  return typeof value === "string" || Array.isArray(value) || isRecord(value);
}

function isStructuredValue(value) {
  return typeof value === "string" || Array.isArray(value) || isRecord(value);
}

function isNonEmptyString(value) {
  return typeof value === "string" && value.length > 0;
}

function isFiniteNumber(value) {
  return typeof value === "number" && Number.isFinite(value);
}

function isProbability(value) {
  return isFiniteNumber(value) && value >= 0 && value <= 1;
}

function isNonNegativeInteger(value) {
  return Number.isInteger(value) && value >= 0;
}

function isDistribution(value, expectedKeys) {
  if (!isRecord(value)
    || Object.keys(value).length !== expectedKeys.length
    || expectedKeys.some((key) => !isProbability(value[key]))) return false;
  const sum = expectedKeys.reduce((total, key) => total + value[key], 0);
  return Math.abs(sum - 1) <= 0.01;
}
