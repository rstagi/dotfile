pub mod ack;
pub mod add;
pub mod agent;
pub mod attach;
pub mod backend;
pub mod build_notifier;
pub mod claude;
pub mod codex;
pub mod device;
pub mod dismiss;
pub mod done;
pub mod events;
pub mod grok;
pub mod hook;
pub mod inbox;
pub mod list;
pub mod loops;
pub mod mcp;
pub mod merge;
pub mod migrate;
pub mod name;
pub mod new;
pub mod on;
pub mod open;
pub mod phase;
pub mod pick;
pub mod picker;
pub mod pin;
pub mod rm;
pub mod send;
pub mod session;
pub mod sessions;
pub mod show;
pub mod top;
pub mod triggers;
pub mod usage;
pub mod wait;
pub mod whoami;

use ws_core::registry::Session;

pub fn agent(session: &Session) -> &str {
    session
        .body
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

/// Returns the session name, or its `#N` alias when it has no name.
pub fn label(session: &Session) -> String {
    session
        .name
        .clone()
        .unwrap_or_else(|| format!("#{}", session.alias))
}

/// Returns the session that created this session.
pub fn parent(session: &Session) -> Option<ws_core::ids::SessionId> {
    session
        .body
        .get("parent_session_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse().ok())
}

/// Returns the session JSON with its top-level `parent_session_id`, as the daemon returns it.
pub fn session_json(session: &Session) -> serde_json::Result<serde_json::Value> {
    let mut value = serde_json::to_value(session)?;
    value["parent_session_id"] = serde_json::json!(parent(session));
    Ok(value)
}

/// Orders items depth-first under their parents and returns `(index, depth)` pairs.
/// `parents[i]` is the index of the parent of item `i`. Roots keep their input order.
/// Siblings under a parent follow `rank`. An item in a parent cycle becomes a root.
pub fn nest<K: Ord>(parents: &[Option<usize>], rank: &[K]) -> Vec<(usize, usize)> {
    let mut order = Vec::with_capacity(parents.len());
    let mut visited = vec![false; parents.len()];
    let roots: Vec<_> = (0..parents.len())
        .filter(|index| parents[*index].is_none_or(|parent| parent == *index))
        .collect();
    for root in roots.into_iter().chain(0..parents.len()) {
        let mut stack = vec![(root, 0)];
        while let Some((index, depth)) = stack.pop() {
            if visited[index] {
                continue;
            }
            visited[index] = true;
            order.push((index, depth));
            let mut children: Vec<_> = (0..parents.len())
                .filter(|child| parents[*child] == Some(index) && !visited[*child])
                .collect();
            children.sort_by(|left, right| rank[*right].cmp(&rank[*left]));
            stack.extend(children.into_iter().map(|child| (child, depth + 1)));
        }
    }
    order
}

/// Returns the tree prefix for a nesting depth, for example `  └ ` at depth 2.
pub fn tree_prefix(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{}└ ", "  ".repeat(depth - 1))
    }
}

pub fn status(session: &Session) -> &str {
    match session.state.as_str() {
        "finished" => "done",
        "failed" => "closed",
        _ => "active",
    }
}

async fn caller(socket: &std::path::Path) -> crate::Result<Option<ws_core::ids::SessionId>> {
    if whoami::identity().is_some() {
        Ok(Some(whoami::current(socket).await?.id))
    } else {
        Ok(None)
    }
}

/// Returns the parent for a new session: the calling agent's session only.
/// A command that a person types in a session pane starts a top-level session.
async fn spawner(socket: &std::path::Path) -> crate::Result<Option<ws_core::ids::SessionId>> {
    if whoami::agent_identity().is_some() {
        caller(socket).await
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::{nest, tree_prefix};

    #[test]
    fn nest_orders_children_under_parents_at_any_depth() {
        let parents = [None, Some(0), None, Some(0), Some(1)];
        assert_eq!(
            nest(&parents, &[0, 1, 2, 3, 4]),
            [(0, 0), (1, 1), (4, 2), (3, 1), (2, 0)]
        );
        assert_eq!(
            nest(&parents, &[0, 9, 2, 3, 4]),
            [(0, 0), (3, 1), (1, 1), (4, 2), (2, 0)]
        );
    }

    #[test]
    fn nest_keeps_every_item_when_parents_form_a_cycle() {
        let parents = [Some(1), Some(0), None];
        assert_eq!(nest(&parents, &[0, 1, 2]), [(2, 0), (0, 0), (1, 1)]);
    }

    #[test]
    fn tree_prefix_indents_each_level() {
        assert_eq!(tree_prefix(0), "");
        assert_eq!(tree_prefix(1), "└ ");
        assert_eq!(tree_prefix(3), "    └ ");
    }
}
