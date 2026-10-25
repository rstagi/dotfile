//! Checks the events of a loop against the Loop rules. The check is a pure function of the log.
//! A hard violation breaks a rule without an exception. A soft violation is a warning, and a
//! `loop.decision` event with the rule as its topic gives the reason that removes it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use ws_core::log::LogEntry;

/// The provider order of the AR rotation. Outer round `r` starts the reviewer at index `r - 1`.
const DEFAULT_ROTATION: [&str; 3] = ["gpt", "claude", "grok"];
/// The default inner cap and outer round maximum of a review phase.
const DEFAULT_CAP: u64 = 2;
/// The Loop facts that a tool or the daemon writes. The agent never writes them.
const TOOL_EVENTS: [&str; 5] = [
    "loop.phase",
    "loop.attempt",
    "loop.review",
    "loop.merge",
    "loop.gate",
];

/// One logged event: its sequence number, its type and its body.
#[derive(Clone, Copy)]
pub(crate) struct Event<'a> {
    pub seq: u64,
    pub kind: &'a str,
    pub body: &'a Value,
}

/// The phase, the repository and the outer round of a review step.
type Round<'a> = (u64, &'a str, u64);
/// The sequence numbers of the reviewer events and of the critic events of one provider.
type ProviderSeqs = (Vec<u64>, Vec<u64>);
/// The phase, the sessions and the event sequence numbers of one review step key.
type KeySessions<'a> = (Option<u64>, BTreeSet<&'a str>, Vec<u64>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    Hard,
    Soft,
}

#[derive(Debug)]
pub(crate) struct Violation {
    pub rule: &'static str,
    pub severity: Severity,
    pub phase: Option<u64>,
    /// The sequence numbers of the events that prove the violation, in ascending order.
    pub seqs: Vec<u64>,
    pub message: String,
}

/// Checks the log entries of a loop session and returns the result document.
pub(crate) fn check_entries(entries: &[LogEntry]) -> Value {
    let events: Vec<Event> = entries
        .iter()
        .filter(|entry| entry.event_type.starts_with("loop."))
        .map(|entry| Event {
            seq: entry.seq,
            kind: &entry.event_type,
            body: &entry.body,
        })
        .collect();
    report(&check(&events))
}

/// Applies every rule to the events, which are in log order.
pub(crate) fn check(events: &[Event]) -> Vec<Violation> {
    let phases = Phases::fold(events);
    let mut violations = Vec::new();
    merged_needs_verify(events, &mut violations);
    milestone_gate_order(events, &mut violations);
    review_step_sessions(events, &mut violations);
    final_review_once(events, &mut violations);
    let reviews = reviews(events);
    let decisions = decisions(events);
    let mut soft = Vec::new();
    reviewer_critic_provider(&reviews, &mut soft);
    provider_rotation(&reviews, &mut soft);
    inner_cap(&reviews, &phases, &mut soft);
    fix_timing(&reviews, &phases, &mut soft);
    soft.retain(|violation| !decisions.gives_reason(violation.rule, violation.phase));
    violations.extend(soft);
    agent_source(events, &mut violations);
    guard_events(events, &mut violations);
    for violation in &mut violations {
        violation.seqs.sort_unstable();
        violation.seqs.dedup();
    }
    violations.sort_by_key(|violation| (violation.seqs.first().copied(), violation.rule));
    violations
}

fn report(violations: &[Violation]) -> Value {
    let count = |severity| {
        violations
            .iter()
            .filter(|violation| violation.severity == severity)
            .count()
    };
    json!({
        "ok": count(Severity::Hard) == 0,
        "hard": count(Severity::Hard),
        "soft": count(Severity::Soft),
        "violations": violations.iter().map(|violation| json!({
            "rule": violation.rule,
            "severity": match violation.severity {
                Severity::Hard => "hard",
                Severity::Soft => "soft",
            },
            "phase": violation.phase,
            "seqs": violation.seqs,
            "message": violation.message,
        })).collect::<Vec<_>>(),
    })
}

/// A phase with the status `merged` has a `loop.merge` event with `verify: pass`.
fn merged_needs_verify(events: &[Event], out: &mut Vec<Violation>) {
    let passed: BTreeSet<u64> = events
        .iter()
        .filter(|event| event.kind == "loop.merge" && event.body["verify"] == "pass")
        .filter_map(|event| event.body["n"].as_u64())
        .collect();
    let mut merged: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for event in events {
        if event.kind == "loop.phase"
            && event.body["status"] == "merged"
            && let Some(n) = event.body["n"].as_u64()
            && !passed.contains(&n)
        {
            merged.entry(n).or_default().push(event.seq);
        }
    }
    for (n, mut seqs) in merged {
        seqs.extend(
            events
                .iter()
                .filter(|event| event.kind == "loop.merge" && event.body["n"] == n)
                .map(|event| event.seq),
        );
        out.push(Violation {
            rule: "merged-needs-verify",
            severity: Severity::Hard,
            phase: Some(n),
            seqs,
            message: format!("Phase {n} is merged without a loop.merge event with verify pass."),
        });
    }
}

/// No phase of milestone N+1 starts before the merge gate of milestone N is resolved.
fn milestone_gate_order(events: &[Event], out: &mut Vec<Violation>) {
    let mut gate_seqs: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
    for event in events {
        if event.kind == "loop.gate"
            && event.body["kind"] == "merge"
            && let Some(milestone) = event.body["milestone"].as_u64()
        {
            gate_seqs.entry(milestone).or_default().push(event.seq);
        }
    }
    let mut phases: BTreeMap<u64, (Option<u64>, bool)> = BTreeMap::new();
    let mut merged: BTreeSet<u64> = BTreeSet::new();
    let mut reported: BTreeSet<(u64, u64)> = BTreeSet::new();
    for event in events {
        match event.kind {
            "loop.phases" => {
                for phase in event.body["phases"].as_array().into_iter().flatten() {
                    note_phase(&mut phases, phase);
                }
            }
            "loop.phase" => note_phase(&mut phases, event.body),
            "loop.gate" if event.body["kind"] == "merge" && event.body["state"] == "merged" => {
                merged.extend(event.body["milestone"].as_u64());
            }
            _ => {}
        }
        let Some(n) = started_phase(event) else {
            continue;
        };
        let Some(milestone) = phases.get(&n).and_then(|(milestone, _)| *milestone) else {
            continue;
        };
        let required = gate_seqs
            .keys()
            .copied()
            .chain(
                phases
                    .values()
                    .filter(|(_, gate)| *gate)
                    .filter_map(|(milestone, _)| *milestone),
            )
            .collect::<BTreeSet<u64>>();
        for gate in required {
            if gate < milestone && !merged.contains(&gate) && reported.insert((n, gate)) {
                let mut seqs = vec![event.seq];
                seqs.extend(gate_seqs.get(&gate).into_iter().flatten());
                out.push(Violation {
                    rule: "milestone-gate-order",
                    severity: Severity::Hard,
                    phase: Some(n),
                    seqs,
                    message: format!(
                        "Phase {n} of milestone {milestone} started before the merge gate of milestone {gate} was resolved."
                    ),
                });
            }
        }
    }
}

fn note_phase(phases: &mut BTreeMap<u64, (Option<u64>, bool)>, phase: &Value) {
    let Some(n) = phase["n"].as_u64() else {
        return;
    };
    let entry = phases.entry(n).or_default();
    if let Some(milestone) = phase["milestone"].as_u64() {
        entry.0 = Some(milestone);
    }
    if let Some(gate) = phase.get("gate") {
        entry.1 = gate == "merge";
    }
}

/// The phase that an attempt or a review step starts, if the event starts one.
fn started_phase(event: &Event) -> Option<u64> {
    match event.kind {
        "loop.attempt" if event.body["state"] == "started" => event.body["n"].as_u64(),
        "loop.review" if event.body["state"] == "started" => event.body["phase"].as_u64(),
        _ => None,
    }
}

/// No review step key has two sessions.
fn review_step_sessions(events: &[Event], out: &mut Vec<Violation>) {
    let mut keys: BTreeMap<&str, KeySessions> = BTreeMap::new();
    for event in events.iter().filter(|event| event.kind == "loop.review") {
        let (Some(key), Some(session)) =
            (event.body["key"].as_str(), event.body["session"].as_str())
        else {
            continue;
        };
        let entry = keys.entry(key).or_default();
        entry.0 = entry.0.or(event.body["phase"].as_u64());
        entry.1.insert(session);
        entry.2.push(event.seq);
    }
    for (key, (phase, sessions, seqs)) in keys {
        if sessions.len() > 1 {
            out.push(Violation {
                rule: "review-step-key-sessions",
                severity: Severity::Hard,
                phase,
                seqs,
                message: format!("The review step key {key} has {} sessions.", sessions.len()),
            });
        }
    }
}

/// Each review phase posts its final review at most once for each repository.
fn final_review_once(events: &[Event], out: &mut Vec<Violation>) {
    let mut finals: BTreeMap<(u64, &str), Vec<u64>> = BTreeMap::new();
    for event in events.iter().filter(|event| {
        event.kind == "loop.review"
            && event.body["role"] == "final"
            && event.body["state"] == "finished"
    }) {
        if let (Some(phase), Some(repo)) =
            (event.body["phase"].as_u64(), event.body["repo"].as_str())
        {
            finals.entry((phase, repo)).or_default().push(event.seq);
        }
    }
    for ((phase, repo), seqs) in finals {
        if seqs.len() > 1 {
            out.push(Violation {
                rule: "final-review-once",
                severity: Severity::Hard,
                phase: Some(phase),
                seqs,
                message: format!("Phase {phase} posted its final review twice for {repo}."),
            });
        }
    }
}

/// A review event with the fields that the soft rules read.
struct Review<'a> {
    seq: u64,
    phase: u64,
    repo: &'a str,
    role: &'a str,
    round: u64,
    inner: Option<u64>,
    provider: Option<&'a str>,
}

fn reviews<'a>(events: &[Event<'a>]) -> Vec<Review<'a>> {
    events
        .iter()
        .filter(|event| event.kind == "loop.review")
        .filter_map(|event| {
            Some(Review {
                seq: event.seq,
                phase: event.body["phase"].as_u64()?,
                repo: event.body["repo"].as_str()?,
                role: event.body["role"].as_str()?,
                round: event.body["round"].as_u64()?,
                inner: event.body["inner"].as_u64(),
                provider: event.body["provider"].as_str(),
            })
        })
        .collect()
}

/// The reviewer and the critic of a round use the same provider.
fn reviewer_critic_provider(reviews: &[Review], out: &mut Vec<Violation>) {
    let mut rounds: BTreeMap<Round, BTreeMap<&str, ProviderSeqs>> = BTreeMap::new();
    for review in reviews {
        let Some(provider) = review.provider else {
            continue;
        };
        let entry = rounds
            .entry((review.phase, review.repo, review.round))
            .or_default()
            .entry(provider)
            .or_default();
        match review.role {
            "review" | "respond" => entry.0.push(review.seq),
            "critic" => entry.1.push(review.seq),
            _ => {}
        }
    }
    for ((phase, repo, round), providers) in rounds {
        for (provider, (reviewer, critic)) in providers {
            if !reviewer.is_empty() && !critic.is_empty() {
                out.push(Violation {
                    rule: "reviewer-critic-provider",
                    severity: Severity::Soft,
                    phase: Some(phase),
                    seqs: [reviewer, critic].concat(),
                    message: format!(
                        "The reviewer and the critic of round {round} for {repo} both use {provider}."
                    ),
                });
            }
        }
    }
}

/// The reviewer of an outer round follows the default provider rotation.
fn provider_rotation(reviews: &[Review], out: &mut Vec<Violation>) {
    let mut rounds: BTreeMap<Round, (Vec<u64>, &str)> = BTreeMap::new();
    for review in reviews {
        let Some(provider) = review.provider else {
            continue;
        };
        if !["review", "respond"].contains(&review.role) || review.round == 0 {
            continue;
        }
        let expected = DEFAULT_ROTATION[(review.round as usize - 1) % DEFAULT_ROTATION.len()];
        if provider != expected {
            rounds
                .entry((review.phase, review.repo, review.round))
                .or_insert_with(|| (Vec::new(), expected))
                .0
                .push(review.seq);
        }
    }
    for ((phase, repo, round), (seqs, expected)) in rounds {
        out.push(Violation {
            rule: "provider-rotation",
            severity: Severity::Soft,
            phase: Some(phase),
            seqs,
            message: format!(
                "The reviewer of round {round} for {repo} is not {expected}, the default rotation."
            ),
        });
    }
}

/// An inner exchange goes past the inner cap.
fn inner_cap(reviews: &[Review], phases: &Phases, out: &mut Vec<Violation>) {
    let mut rounds: BTreeMap<Round, (Vec<u64>, u64, u64)> = BTreeMap::new();
    for review in reviews {
        let cap = phases.cap(review.phase, "inner");
        if let Some(inner) = review.inner.filter(|inner| *inner > cap) {
            let entry = rounds
                .entry((review.phase, review.repo, review.round))
                .or_insert_with(|| (Vec::new(), inner, cap));
            entry.0.push(review.seq);
            entry.1 = entry.1.max(inner);
        }
    }
    for ((phase, repo, round), (seqs, inner, cap)) in rounds {
        out.push(Violation {
            rule: "inner-cap",
            severity: Severity::Soft,
            phase: Some(phase),
            seqs,
            message: format!(
                "Round {round} for {repo} reached inner exchange {inner}, past the cap of {cap}."
            ),
        });
    }
}

/// A fix runs between outer rounds: after the review of its round and before the next round.
fn fix_timing(reviews: &[Review], phases: &Phases, out: &mut Vec<Violation>) {
    let mut reported: BTreeSet<(u64, &str, u64)> = BTreeSet::new();
    for fix in reviews.iter().filter(|review| review.role == "fix") {
        if !reported.insert((fix.phase, fix.repo, fix.round)) {
            continue;
        }
        let same_step = |review: &&Review| {
            review.phase == fix.phase && review.repo == fix.repo && review.role != "fix"
        };
        let cap = phases.cap(fix.phase, "rounds");
        let reviewed = reviews
            .iter()
            .filter(same_step)
            .any(|review| review.round == fix.round && review.seq < fix.seq);
        let later = reviews
            .iter()
            .filter(same_step)
            .find(|review| review.round > fix.round && review.seq < fix.seq);
        let problem = if fix.round >= cap {
            Some(format!("after the last of {cap} outer rounds"))
        } else if !reviewed {
            Some(format!("before the review of round {}", fix.round))
        } else if later.is_some() {
            Some(format!("after round {} started", fix.round + 1))
        } else {
            None
        };
        if let Some(problem) = problem {
            out.push(Violation {
                rule: "fix-timing",
                severity: Severity::Soft,
                phase: Some(fix.phase),
                seqs: [fix.seq].into_iter().chain(later.map(|r| r.seq)).collect(),
                message: format!(
                    "The fix of round {} for {} ran {problem}.",
                    fix.round, fix.repo
                ),
            });
        }
    }
}

/// A Loop fact with `source: agent`. The tools and the daemon write these facts.
fn agent_source(events: &[Event], out: &mut Vec<Violation>) {
    for event in events {
        if TOOL_EVENTS.contains(&event.kind) && event.body["source"] == "agent" {
            out.push(Violation {
                rule: "agent-source",
                severity: Severity::Soft,
                phase: event.body["n"]
                    .as_u64()
                    .or_else(|| event.body["phase"].as_u64()),
                seqs: vec![event.seq],
                message: format!("An agent wrote the {} event.", event.kind),
            });
        }
    }
}

/// Each `loop.guard` event records a command that the guard refused.
fn guard_events(events: &[Event], out: &mut Vec<Violation>) {
    for event in events.iter().filter(|event| event.kind == "loop.guard") {
        out.push(Violation {
            rule: "guard-event",
            severity: Severity::Soft,
            phase: event.body["phase"].as_u64(),
            seqs: vec![event.seq],
            message: format!(
                "The guard refused a command under the rule {}.",
                event.body["rule"].as_str().unwrap_or("unknown")
            ),
        });
    }
}

/// The `inner` and `rounds` caps that a phase record declares.
struct Phases(BTreeMap<u64, Value>);

impl Phases {
    fn fold(events: &[Event]) -> Self {
        let mut phases: BTreeMap<u64, Value> = BTreeMap::new();
        for event in events {
            let records: Vec<&Value> = match event.kind {
                "loop.phases" => event.body["phases"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .collect(),
                "loop.phase" => vec![event.body],
                _ => continue,
            };
            for record in records {
                let (Some(n), Some(fields)) = (record["n"].as_u64(), record.as_object()) else {
                    continue;
                };
                let entry = phases.entry(n).or_insert_with(|| json!({}));
                for (name, value) in fields {
                    entry[name] = value.clone();
                }
            }
        }
        Self(phases)
    }

    fn cap(&self, phase: u64, name: &str) -> u64 {
        self.0
            .get(&phase)
            .and_then(|record| record[name].as_u64())
            .unwrap_or(DEFAULT_CAP)
    }
}

/// The `loop.decision` events. A decision names the rule in `topic`, and a phase in `phase`.
struct Decisions(Vec<(String, Option<u64>)>);

fn decisions(events: &[Event]) -> Decisions {
    Decisions(
        events
            .iter()
            .filter(|event| event.kind == "loop.decision")
            .filter_map(|event| {
                Some((
                    event.body["topic"].as_str()?.to_owned(),
                    event.body["phase"].as_u64(),
                ))
            })
            .collect(),
    )
}

impl Decisions {
    fn gives_reason(&self, rule: &str, phase: Option<u64>) -> bool {
        self.0.iter().any(|(topic, decided)| {
            topic == rule && (decided.is_none() || phase.is_none() || *decided == phase)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds the events of a fixture. Each event gets the next sequence number from 1.
    fn run(fixture: &[(&str, Value)]) -> Vec<Violation> {
        let bodies: Vec<(u64, &str, &Value)> = fixture
            .iter()
            .enumerate()
            .map(|(index, (kind, body))| (index as u64 + 1, *kind, body))
            .collect();
        let events: Vec<Event> = bodies
            .iter()
            .map(|(seq, kind, body)| Event {
                seq: *seq,
                kind,
                body,
            })
            .collect();
        check(&events)
    }

    fn rules(violations: &[Violation]) -> Vec<&'static str> {
        violations.iter().map(|violation| violation.rule).collect()
    }

    fn only<'a>(violations: &'a [Violation], rule: &str, severity: Severity) -> &'a Violation {
        let found: Vec<_> = violations
            .iter()
            .filter(|violation| violation.rule == rule)
            .collect();
        assert_eq!(found.len(), 1, "{rule}: {violations:?}");
        assert_eq!(found[0].severity, severity);
        found[0]
    }

    fn review(role: &str, round: u64, provider: &str, seq_key: &str) -> (&'static str, Value) {
        (
            "loop.review",
            json!({"phase":9,"repo":"o/r","key":seq_key,"role":role,"round":round,
                "state":"started","session":seq_key,"provider":provider,"source":"daemon"}),
        )
    }

    fn phases(extra: Value) -> (&'static str, Value) {
        let mut review_phase = json!({"n":9,"milestone":1,"gate":null,"status":"todo"});
        review_phase
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        ("loop.phases", json!({"phases":[review_phase]}))
    }

    #[test]
    fn a_log_without_facts_passes() {
        assert!(run(&[]).is_empty());
        assert_eq!(report(&[])["ok"], true);
    }

    #[test]
    fn merged_needs_verify_passes_with_a_passing_merge() {
        let violations = run(&[
            (
                "loop.merge",
                json!({"n":3,"repo":"o/r","head":"a","verify":"pass"}),
            ),
            ("loop.phase", json!({"n":3,"status":"merged"})),
        ]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn merged_needs_verify_fails_without_a_passing_merge() {
        let violations = run(&[
            (
                "loop.merge",
                json!({"n":3,"repo":"o/r","head":"a","verify":"fail"}),
            ),
            ("loop.phase", json!({"n":3,"status":"merged"})),
        ]);
        let violation = only(&violations, "merged-needs-verify", Severity::Hard);
        assert_eq!(violation.phase, Some(3));
        assert_eq!(violation.seqs, vec![1, 2]);
    }

    fn two_milestones(gate: Vec<(&'static str, Value)>) -> Vec<(&'static str, Value)> {
        let mut log = vec![
            (
                "loop.phases",
                json!({"phases":[
                    {"n":1,"milestone":1,"gate":null,"status":"todo"},
                    {"n":2,"milestone":1,"gate":"merge","status":"todo"},
                    {"n":3,"milestone":2,"gate":null,"status":"todo"}]}),
            ),
            (
                "loop.gate",
                json!({"kind":"merge","state":"open","milestone":1,"prs":{}}),
            ),
        ];
        log.extend(gate);
        log.push((
            "loop.attempt",
            json!({"n":3,"attempt":1,"session":"s","state":"started"}),
        ));
        log
    }

    #[test]
    fn a_next_milestone_phase_starts_after_the_merged_gate() {
        let violations = run(&two_milestones(vec![(
            "loop.gate",
            json!({"kind":"merge","state":"merged","milestone":1,"prs":{}}),
        )]));
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn a_next_milestone_phase_starting_at_an_open_gate_fails() {
        let violations = run(&two_milestones(vec![]));
        let violation = only(&violations, "milestone-gate-order", Severity::Hard);
        assert_eq!(violation.phase, Some(3));
        assert_eq!(violation.seqs, vec![2, 3]);
    }

    #[test]
    fn a_next_milestone_phase_starting_before_the_gate_opens_fails() {
        let violations = run(&[
            (
                "loop.phases",
                json!({"phases":[
                    {"n":2,"milestone":1,"gate":"merge","status":"todo"},
                    {"n":3,"milestone":2,"gate":null,"status":"todo"}]}),
            ),
            (
                "loop.review",
                json!({"phase":3,"repo":"o/r","key":"k","role":"review","round":1,
                    "state":"started","session":"s","provider":"gpt"}),
            ),
        ]);
        assert_eq!(rules(&violations), vec!["milestone-gate-order"]);
    }

    #[test]
    fn a_review_step_key_with_one_session_passes() {
        let started = review("review", 1, "gpt", "k1");
        let mut finished = started.clone();
        finished.1["state"] = json!("finished");
        finished.1["verdict"] = json!("AGREE");
        assert!(run(&[started, finished]).is_empty());
    }

    #[test]
    fn a_review_step_key_with_two_sessions_fails() {
        let first = review("review", 1, "gpt", "k1");
        let mut second = first.clone();
        second.1["session"] = json!("other");
        let violations = run(&[first, second]);
        let violation = only(&violations, "review-step-key-sessions", Severity::Hard);
        assert_eq!(violation.seqs, vec![1, 2]);
        assert_eq!(violation.phase, Some(9));
    }

    fn finished_final(repo: &str, key: &str) -> (&'static str, Value) {
        (
            "loop.review",
            json!({"phase":9,"repo":repo,"key":key,"role":"final","round":2,"state":"finished",
                "session":key,"provider":"claude","verdict":"APPROVE"}),
        )
    }

    #[test]
    fn two_repositories_each_post_one_final_review() {
        let violations = run(&[finished_final("o/a", "k1"), finished_final("o/b", "k2")]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn two_final_reviews_for_one_repository_fail() {
        let violations = run(&[finished_final("o/a", "k1"), finished_final("o/a", "k2")]);
        let violation = only(&violations, "final-review-once", Severity::Hard);
        assert_eq!(violation.seqs, vec![1, 2]);
    }

    #[test]
    fn matching_providers_in_a_round_pass_when_the_critic_differs() {
        let violations = run(&[
            review("review", 1, "gpt", "k1"),
            review("critic", 1, "claude", "k2"),
        ]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn a_reviewer_and_critic_with_one_provider_warn() {
        let violations = run(&[
            review("review", 1, "gpt", "k1"),
            review("critic", 1, "gpt", "k2"),
        ]);
        let violation = only(&violations, "reviewer-critic-provider", Severity::Soft);
        assert_eq!(violation.seqs, vec![1, 2]);
    }

    #[test]
    fn a_decision_removes_the_reviewer_critic_warning() {
        let violations = run(&[
            review("review", 1, "gpt", "k1"),
            review("critic", 1, "gpt", "k2"),
            (
                "loop.decision",
                json!({"topic":"reviewer-critic-provider","default":"split","choice":"same",
                    "reason":"The other providers are out of quota."}),
            ),
        ]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn the_default_rotation_passes() {
        let violations = run(&[
            review("review", 1, "gpt", "k1"),
            review("review", 2, "claude", "k2"),
            review("review", 3, "grok", "k3"),
        ]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn another_rotation_warns_until_a_decision_gives_the_reason() {
        let log = [review("review", 1, "claude", "k1")];
        let violation = only(&run(&log), "provider-rotation", Severity::Soft)
            .seqs
            .clone();
        assert_eq!(violation, vec![1]);
        let decided = run(&[
            log[0].clone(),
            (
                "loop.decision",
                json!({"topic":"provider-rotation","phase":9,"default":"gpt","choice":"claude",
                    "reason":"The Codex quota is used."}),
            ),
        ]);
        assert!(decided.is_empty(), "{decided:?}");
    }

    #[test]
    fn a_decision_for_another_phase_keeps_the_warning() {
        let violations = run(&[
            review("review", 1, "claude", "k1"),
            (
                "loop.decision",
                json!({"topic":"provider-rotation","phase":4,"default":"gpt","choice":"claude",
                    "reason":"r"}),
            ),
        ]);
        assert_eq!(rules(&violations), vec!["provider-rotation"]);
    }

    fn inner(role: &str, inner: u64, key: &str) -> (&'static str, Value) {
        let mut event = review(
            role,
            1,
            if role == "critic" { "claude" } else { "gpt" },
            key,
        );
        event.1["inner"] = json!(inner);
        event
    }

    #[test]
    fn an_inner_exchange_within_the_cap_passes() {
        let violations = run(&[inner("review", 1, "k1"), inner("respond", 2, "k2")]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn an_inner_exchange_past_the_cap_warns() {
        let violations = run(&[inner("review", 1, "k1"), inner("respond", 3, "k2")]);
        assert_eq!(only(&violations, "inner-cap", Severity::Soft).seqs, vec![2]);
    }

    #[test]
    fn a_declared_inner_cap_moves_the_limit() {
        let violations = run(&[
            phases(json!({"inner":3})),
            inner("review", 1, "k1"),
            inner("respond", 3, "k2"),
        ]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn a_fix_between_outer_rounds_passes() {
        let violations = run(&[
            review("review", 1, "gpt", "k1"),
            review("fix", 1, "claude", "k2"),
            review("review", 2, "claude", "k3"),
        ]);
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn a_fix_after_the_last_round_warns() {
        let violations = run(&[
            review("review", 2, "claude", "k1"),
            review("fix", 2, "claude", "k2"),
        ]);
        assert_eq!(
            only(&violations, "fix-timing", Severity::Soft).seqs,
            vec![2]
        );
    }

    #[test]
    fn a_fix_before_the_review_of_its_round_warns() {
        let violations = run(&[review("fix", 1, "claude", "k1")]);
        only(&violations, "fix-timing", Severity::Soft);
    }

    #[test]
    fn a_fix_after_the_next_round_started_warns() {
        let violations = run(&[
            review("review", 1, "gpt", "k1"),
            review("review", 2, "claude", "k2"),
            review("fix", 1, "claude", "k3"),
        ]);
        assert_eq!(
            only(&violations, "fix-timing", Severity::Soft).seqs,
            vec![2, 3]
        );
    }

    #[test]
    fn tool_facts_pass_and_agent_facts_warn() {
        let mut agent = review("review", 1, "gpt", "k1");
        agent.1["source"] = json!("agent");
        let violations = run(&[
            (
                "loop.merge",
                json!({"n":1,"repo":"o/r","head":"a","verify":"pass","source":"tool"}),
            ),
            ("loop.note", json!({"text":"hi","source":"agent"})),
            agent,
        ]);
        let violation = only(&violations, "agent-source", Severity::Soft);
        assert_eq!(violation.seqs, vec![3]);
        assert_eq!(violation.phase, Some(9));
    }

    #[test]
    fn a_decision_cannot_remove_an_agent_source_warning() {
        let mut agent = review("review", 1, "gpt", "k1");
        agent.1["source"] = json!("agent");
        let violations = run(&[
            agent,
            (
                "loop.decision",
                json!({"topic":"agent-source","default":"tool","choice":"agent","reason":"r"}),
            ),
        ]);
        only(&violations, "agent-source", Severity::Soft);
    }

    #[test]
    fn each_guard_event_warns() {
        let guard = json!({"session":"s","rule":"no-push","command":"git push"});
        let violations = run(&[("loop.guard", guard.clone()), ("loop.guard", guard)]);
        assert_eq!(rules(&violations), vec!["guard-event", "guard-event"]);
        assert_eq!(violations[1].seqs, vec![2]);
    }

    #[test]
    fn the_report_counts_hard_and_soft_violations() {
        let violations = run(&[
            ("loop.phase", json!({"n":3,"status":"merged"})),
            (
                "loop.guard",
                json!({"session":"s","rule":"r","command":"c"}),
            ),
        ]);
        let report = report(&violations);
        assert_eq!(report["ok"], false);
        assert_eq!(report["hard"], 1);
        assert_eq!(report["soft"], 1);
        assert_eq!(report["violations"][0]["rule"], "merged-needs-verify");
        assert_eq!(report["violations"][0]["severity"], "hard");
        assert_eq!(report["violations"][0]["seqs"], json!([1]));
    }
}
