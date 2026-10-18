//! Names for the WS semantic conventions v1.
//! See `docs/ws-v6/semconv-v1.md` for types, requirements and source events.

// Resource attributes.
pub const SESSION_ID: &str = "ws.session.id";
pub const PARENT_SESSION_ID: &str = "ws.parent_session.id";
pub const DEVICE_ID: &str = "ws.device.id";
pub const PROJECT: &str = "ws.project";

// Common attributes and values.
pub const SEMCONV_VERSION: &str = "ws.semconv.version";
pub const VERSION: &str = "1";
pub const EVENT_ID: &str = "event_id";
pub const EVENT_DOMAIN: &str = "event.domain";
pub const SESSION_DOMAIN: &str = "ws.session";
pub const EVENT_NAME: &str = "event.name";
pub const SESSION_STATE: &str = "ws.session.state";
pub const SESSION_NAME: &str = "ws.session.name";
pub const SESSION_KIND: &str = "ws.session.kind";
pub const SESSION_EPOCH: &str = "ws.session.epoch";
pub const SESSION_SEQ: &str = "ws.session.seq";
pub const MESSAGE_ID: &str = "ws.session.message.id";
pub const MESSAGE_FROM: &str = "ws.session.message.from";
pub const MESSAGE_TO: &str = "ws.session.message.to";
pub const MESSAGE_PAIR_SEQ: &str = "ws.session.message.pair_seq";
pub const EDGE_ID: &str = "ws.session.edge.id";
pub const EDGE_KIND: &str = "ws.session.edge.kind";
pub const EDGE_TARGET: &str = "ws.session.edge.target";
pub const TRIGGER_ID: &str = "ws.session.trigger.id";
pub const TURN_ID: &str = "ws.session.turn.id";
pub const TOOL_USE_ID: &str = "ws.session.tool.use_id";
pub const TOOL_NAME: &str = "ws.session.tool.name";
pub const TIMING: &str = "ws.timing";
pub const PHASE_ID: &str = "ws.phase.id";
pub const PHASE_ATTEMPT: &str = "ws.phase.attempt";
pub const REASON: &str = "ws.session.reason";

// GenAI model-call attributes.
pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
pub const GEN_AI_PROVIDER_NAME: &str = "gen_ai.provider.name";
pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
pub const GEN_AI_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
pub const GEN_AI_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
pub const GEN_AI_CACHE_READ_INPUT_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";
pub const GEN_AI_CACHE_CREATION_INPUT_TOKENS: &str = "gen_ai.usage.cache_creation.input_tokens";

// Spans.
pub const TURN_SPAN: &str = "ws.session.turn";
pub const TOOL_SPAN: &str = "ws.session.tool";
pub const MODEL_CALL_SPAN: &str = "ws.session.model_call";
pub const PHASE_ATTEMPT_SPAN: &str = "ws.session.phase_attempt";

// Log records and span events.
pub const SESSION_CREATED: &str = "ws.session.created";
pub const SESSION_RUNNING: &str = "ws.session.running";
pub const SESSION_BLOCKED: &str = "ws.session.blocked";
pub const SESSION_FINISHED: &str = "ws.session.finished";
pub const SESSION_FAILED: &str = "ws.session.failed";
pub const SESSION_REOPENED: &str = "ws.session.reopened";
pub const SESSION_RENAMED: &str = "ws.session.renamed";
pub const SESSION_ORPHANED: &str = "ws.session.orphaned";
pub const SESSION_RESUMED: &str = "ws.session.resumed";
pub const SESSION_FAILED_OVER: &str = "ws.session.failed_over";
pub const SESSION_CLAIM_REJECTED: &str = "ws.session.claim_rejected";
pub const MESSAGE_SENT: &str = "ws.session.message.sent";
pub const MESSAGE_DELIVERED: &str = "ws.session.message.delivered";
pub const MESSAGE_READ: &str = "ws.session.message.read";
pub const EDGE_ADDED: &str = "ws.session.edge.added";
pub const EDGE_REMOVED: &str = "ws.session.edge.removed";
pub const TRIGGER_REGISTERED: &str = "ws.session.trigger.registered";
pub const TRIGGER_REMOVED: &str = "ws.session.trigger.removed";
pub const TRIGGER_FIRED: &str = "ws.session.trigger.fired";
pub const TRIGGER_FAILED: &str = "ws.session.trigger.failed";
pub const TURN_STARTED: &str = "ws.session.turn.started";
pub const TURN_ENDED: &str = "ws.session.turn.ended";
pub const TURN_CONTINUED: &str = "ws.session.turn.continued";
pub const TURN_CANCELLED: &str = "ws.session.turn.cancelled";
pub const TOOL_PRE: &str = "ws.session.tool.pre";
pub const TOOL_POST: &str = "ws.session.tool.post";
pub const AGENT_WAITING: &str = "ws.session.agent.waiting";
pub const SUBAGENT_STARTED: &str = "ws.session.subagent.started";
pub const SUBAGENT_STOPPED: &str = "ws.session.subagent.stopped";
pub const SESSION_EXIT: &str = "ws.session.exit";
pub const PHASE_STARTED: &str = "phase.started";
pub const PHASE_FINISHED: &str = "phase.finished";
pub const RUNNER_USAGE_UPDATED: &str = "ws.runner.usage.updated";

// Runner usage attributes.
pub const RUNNER: &str = "ws.runner";
pub const RUNNER_ACCOUNT_ID: &str = "ws.runner.account.id";
pub const RUNNER_USAGE_BUCKET: &str = "ws.runner.usage.bucket";
pub const RUNNER_USAGE_WINDOW: &str = "ws.runner.usage.window";
pub const RUNNER_USAGE_WINDOW_DURATION: &str = "ws.runner.usage.window.duration";
pub const RUNNER_USAGE_WINDOW_START: &str = "ws.runner.usage.window.start";
pub const RUNNER_USAGE_WINDOW_RESET: &str = "ws.runner.usage.window.reset";
pub const RUNNER_USAGE_SOURCE: &str = "ws.runner.usage.source";
pub const RUNNER_USAGE_SOURCE_NAME: &str = "ws.runner.usage.source.name";
pub const RUNNER_USAGE_OBSERVED_AT: &str = "ws.runner.usage.observed_at";
pub const RUNNER_USAGE_COVERAGE: &str = "ws.runner.usage.coverage";
pub const RUNNER_USAGE_MODEL: &str = "ws.runner.usage.model";

// Metrics.
pub const RUNNER_USAGE_USED_RATIO: &str = "ws.runner.usage.used_ratio";
pub const RUNNER_USAGE_REMAINING_RATIO: &str = "ws.runner.usage.remaining_ratio";
pub const RUNNER_USAGE_TOKENS: &str = "ws.runner.usage.tokens";

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::Path};

    #[test]
    fn every_constant_has_a_documented_contract() {
        let document = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/ws-v6/semconv-v1.md"),
        )
        .expect("The semantic conventions document must exist.");
        let mut constants = BTreeSet::new();
        for line in include_str!("semconv.rs").lines() {
            let Some(declaration) = line.strip_prefix("pub const ") else {
                continue;
            };
            let (_, literal) = declaration
                .split_once(" = ")
                .expect("Each constant must use one line.");
            let value = literal
                .strip_prefix('"')
                .and_then(|literal| literal.strip_suffix("\";"))
                .expect("Each constant must contain a string literal.");
            assert!(constants.insert(value), "Duplicate semantic name: {value}");
            let prefix = format!("| `{value}` |");
            let row = document
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap_or_else(|| panic!("The document must define {value}."));
            let columns: Vec<_> = row.split('|').map(str::trim).collect();
            assert_eq!(
                columns.len(),
                6,
                "Expected name, type, requirement and source: {row}"
            );
            assert!(columns[2..5].iter().all(|column| !column.is_empty()));
            assert!(columns[3].starts_with("Required") || columns[3].starts_with("Optional"));
        }
        assert!(
            !constants.is_empty(),
            "The semantic catalog must not be empty."
        );
        let documented: BTreeSet<_> = document
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .map(|row| {
                row.split_once("` |")
                    .expect("A name must end with a backtick.")
                    .0
            })
            .collect();
        assert_eq!(constants, documented, "Each catalog name needs a constant.");
    }
}
