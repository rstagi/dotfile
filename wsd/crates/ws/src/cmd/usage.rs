use std::path::Path;

use hyper::Method;
use ws_daemon::quota::{Source, usage::Usage};

use crate::{Result, client};

#[derive(serde::Serialize)]
struct Demand {
    phase: String,
    runner: String,
    before_minutes: f64,
    review_minutes: f64,
    remaining_minutes: Option<f64>,
    warning: bool,
    alternative: Option<String>,
}

struct Phase<'a> {
    id: &'a str,
    header: &'a str,
    body: String,
    milestone: u64,
}

type Models = std::collections::BTreeMap<String, String>;

pub async fn run(socket: &Path, json: bool) -> Result<()> {
    let body = client::request(socket, Method::GET, "/v1/usage", Vec::new()).await?;
    let usage: Usage = serde_json::from_slice(&body)?;
    if json {
        println!("{}", serde_json::to_string(&usage)?);
    } else {
        println!("RUNNER\tACCOUNT\tBUCKET\tWINDOW\tREMAINING\tSOURCE\tAGE");
        for window in usage.windows {
            let remaining = window
                .remaining_ratio
                .map(|ratio| format!("{:.1}%", ratio * 100.0))
                .unwrap_or(window.status);
            let source = match window.source {
                Some(Source::Reported) => "reported",
                Some(Source::Estimated) => "estimated",
                None => "unknown",
            };
            let age = window
                .age_secs
                .map(|age| format!("{age}s ago"))
                .unwrap_or_else(|| "unknown".into());
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                window.runner,
                window.account_id.as_deref().unwrap_or("unknown"),
                window.bucket.as_deref().unwrap_or("-"),
                window.window,
                remaining,
                source,
                age
            );
        }
    }
    Ok(())
}

pub async fn preflight(
    socket: &Path,
    json: bool,
    plan: &Path,
    models: Option<&Path>,
    milestone: Option<u64>,
) {
    let result = estimate(socket, plan, models, milestone).await;
    match result {
        Ok(demands) if json => println!(
            "{}",
            serde_json::json!({
                "assumption": "Heuristic: active minutes consume the same fraction of a quota window. Chain legs and AR exchanges use conservative upper bounds. Resets and retries are not predicted.",
                "demands": demands,
            })
        ),
        Ok(demands) => {
            println!(
                "Quota preflight: heuristic active-minute estimates. Resets and retries are not predicted."
            );
            for d in demands {
                let remaining = d
                    .remaining_minutes
                    .map(|v| format!("{v:.0} min"))
                    .unwrap_or_else(|| "unknown".into());
                println!(
                    "{}: {:.0} min before review phase {}, {:.0} min for review, {} remaining.",
                    d.runner, d.before_minutes, d.phase, d.review_minutes, remaining
                );
                if d.warning {
                    println!(
                        "Warning: {} may run out before review phase {} completes.",
                        d.runner, d.phase
                    );
                    if let Some(alternative) = d.alternative {
                        println!(
                            "Consider {alternative}. It has more remaining quota. Check the review roles before reassignment."
                        );
                    }
                } else if d.remaining_minutes.is_none() {
                    println!(
                        "Warning: no quota data for {}. Continue the plan.",
                        d.runner
                    );
                }
            }
        }
        Err(error) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"demands": [], "warning": format!("no quota data: {error}")})
                );
            }
            eprintln!("Warning: no quota data: {error}. Continue the plan.");
        }
    }
}

async fn estimate(
    socket: &Path,
    plan: &Path,
    models: Option<&Path>,
    milestone: Option<u64>,
) -> Result<Vec<Demand>> {
    let text = std::fs::read_to_string(plan)?;
    let model_path = match models {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()?
            .ancestors()
            .chain(plan.ancestors())
            .map(|dir| dir.join("loop-models.conf"))
            .find(|path| path.is_file())
            .ok_or("Cannot find loop-models.conf")?,
    };
    // Read literal assignments only. Never execute the configuration as shell code.
    let config: Models = std::fs::read_to_string(model_path)?
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .filter(|(key, _)| key.starts_with("CHAIN_") || key.starts_with("LOOP_"))
        .map(|(key, value)| {
            (
                key.trim().into(),
                value
                    .split(['#', ';'])
                    .next()
                    .unwrap()
                    .trim()
                    .trim_matches('"')
                    .into(),
            )
        })
        .collect();
    let body = client::request(socket, Method::GET, "/v1/usage", Vec::new()).await?;
    let usage: Usage = serde_json::from_slice(&body)?;
    let phases = phases(&text);
    if phases.is_empty() {
        return Err("The plan has no phases".into());
    }
    let mut before = std::collections::BTreeMap::<String, f64>::new();
    let mut demands = Vec::new();
    for phase in &phases {
        if milestone.is_some_and(|m| m != phase.milestone)
            || tag(phase.header, "status") == Some("done")
        {
            continue;
        }
        if tag(phase.header, "kind") != Some("pr-review") {
            let minutes = phase
                .body
                .lines()
                .find_map(|line| line.strip_prefix("- **Estimate:**"))
                .and_then(|value| value.split("min").next())
                .and_then(|value| value.split(['–', '—', '-']).next_back())
                .and_then(|value| value.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(number(&config, "LOOP_TIMEOUT_TASK", 1800.0) / 60.0);
            add_chain(&mut before, &config, "CHAIN_TASK", minutes)?;
            continue;
        }
        let tier = tag(phase.header, "review")
            .unwrap_or_else(|| {
                config
                    .get("LOOP_REVIEW_TIER_DEFAULT")
                    .map(String::as_str)
                    .unwrap_or("standard")
            })
            .to_uppercase();
        let rounds = count(
            phase.header,
            "rounds",
            &config,
            "LOOP_REVIEW_ROUNDS_DEFAULT",
        );
        let inner = count(phase.header, "inner", &config, "LOOP_REVIEW_INNER_DEFAULT");
        let repositories: std::collections::BTreeSet<_> = phases
            .iter()
            .filter(|p| p.milestone == phase.milestone)
            .flat_map(|p| {
                p.body
                    .lines()
                    .filter_map(|line| line.strip_prefix("- **Repository:**"))
            })
            .collect();
        let copies = repositories.len().max(1) as f64;
        let mut review = std::collections::BTreeMap::<String, f64>::new();
        let minutes = number(&config, "LOOP_TIMEOUT_REVIEW", 1800.0) / 60.0;
        // Each provider can review, criticize or confirm after a veto. Reserve the inner cap.
        for role in ["GPT", "CLAUDE", "GROK"] {
            add_chain(
                &mut review,
                &config,
                &format!("CHAIN_REVIEW_{tier}_{role}"),
                rounds * inner * minutes * copies,
            )?;
        }
        add_chain(
            &mut review,
            &config,
            &format!("CHAIN_REVIEW_{tier}_FIX"),
            (rounds - 1.0) * number(&config, "LOOP_TIMEOUT_REMEDIATE", 1800.0) / 60.0 * copies,
        )?;
        add_chain(
            &mut review,
            &config,
            &format!("CHAIN_REVIEW_{tier}_FINAL"),
            minutes * copies,
        )?;
        for (runner, review_minutes) in &review {
            let before_minutes = before.get(runner).copied().unwrap_or(0.0);
            let remaining = capacity(&usage, runner);
            let warning = remaining
                .0
                .is_some_and(|value| value < before_minutes + review_minutes);
            let alternative = if warning {
                ["claude", "codex", "grok"]
                    .into_iter()
                    .filter(|other| *other != runner)
                    .filter_map(|other| {
                        let (value, known) = capacity(&usage, other);
                        let available = value? - before.get(other).copied().unwrap_or(0.0);
                        (known && available > remaining.0.unwrap() - before_minutes)
                            .then_some((other, available))
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(other, _)| other.to_owned())
            } else {
                None
            };
            demands.push(Demand {
                phase: phase.id.into(),
                runner: runner.clone(),
                before_minutes,
                review_minutes: *review_minutes,
                remaining_minutes: remaining.0,
                warning,
                alternative,
            });
        }
        for (runner, minutes) in review {
            *before.entry(runner).or_default() += minutes;
        }
    }
    Ok(demands)
}

fn phases(text: &str) -> Vec<Phase<'_>> {
    let mut result = Vec::new();
    let mut milestone = 1;
    let mut current: Option<Phase<'_>> = None;
    let mut fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if (line.starts_with("## ") || line.starts_with("### "))
            && let Some(phase) = current.take()
        {
            result.push(phase);
        }
        if let Some(rest) = line.strip_prefix("## Milestone ") {
            milestone = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
        }
        if let Some(rest) = line.strip_prefix("### Phase ") {
            current = Some(Phase {
                id: rest.split_whitespace().next().unwrap_or("?"),
                header: line,
                body: String::new(),
                milestone,
            });
        } else if let Some(phase) = &mut current {
            phase.body.push_str(line);
            phase.body.push('\n');
        }
    }
    if let Some(phase) = current {
        result.push(phase);
    }
    result
}

fn tag<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header
        .split_once(&format!("[{name}: "))?
        .1
        .split_once(']')
        .map(|(value, _)| value.trim())
}

fn number(models: &Models, key: &str, default: f64) -> f64 {
    models
        .get(key)
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(default)
}

fn count(header: &str, name: &str, models: &Models, key: &str) -> f64 {
    tag(header, name)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(number(models, key, 2.0))
        .clamp(1.0, 5.0)
}

fn add_chain(
    demand: &mut std::collections::BTreeMap<String, f64>,
    models: &Models,
    key: &str,
    minutes: f64,
) -> Result<()> {
    let value = models
        .get(key)
        .ok_or_else(|| format!("Missing model chain {key}"))?;
    let runners: std::collections::BTreeSet<_> = value
        .trim_matches(['(', ')'])
        .split_whitespace()
        .filter_map(|leg| {
            leg.trim_matches(['"', '\''])
                .split_once(':')
                .map(|(runner, _)| runner)
        })
        .filter(|runner| ["claude", "codex", "grok"].contains(runner))
        .collect();
    if runners.is_empty() {
        return Err(format!("Cannot read model chain {key}").into());
    }
    for runner in runners {
        *demand.entry(runner.into()).or_default() += minutes;
    }
    Ok(())
}

fn capacity(usage: &Usage, runner: &str) -> (Option<f64>, bool) {
    let mut remaining: Option<f64> = None;
    let mut known = true;
    for window in usage.windows.iter().filter(|w| w.runner == runner) {
        let duration = match window.window.as_str() {
            "five_hour" => Some(300.0),
            "weekly" => Some(10080.0),
            _ => None,
        };
        if let Some(value) = duration
            .zip(window.remaining_ratio)
            .filter(|_| window.status == "available")
            .map(|(duration, ratio)| duration * ratio)
        {
            remaining = Some(remaining.map_or(value, |old| old.min(value)));
        } else {
            known = false;
        }
    }
    (remaining, known && remaining.is_some())
}
