//! Readable account labels. A label is display data only. `account_id` stays the key.
//!
//! The auth files also hold tokens. These functions read only the account id, email and
//! team-name claims. They never return, log or store a token. A label is not stored: the
//! launch bindings and the usage events keep the opaque account id only.

use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub(crate) type Labels = HashMap<(String, String), String>;

/// The login files of the daemon user. They follow the variables that agent launches use.
pub(crate) struct Homes {
    pub claude_config: PathBuf,
    pub codex: PathBuf,
    pub grok: PathBuf,
}

impl Homes {
    pub(crate) fn from_env() -> Option<Self> {
        let var = |key| std::env::var_os(key).map(PathBuf::from);
        let home = var("HOME")?;
        Some(Self {
            claude_config: var("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| home.clone())
                .join(".claude.json"),
            codex: var("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            grok: var("GROK_HOME").unwrap_or_else(|| home.join(".grok")),
        })
    }
}

/// Labels the account of each current login. Other accounts get no label.
pub(crate) fn current(homes: &Homes) -> Labels {
    let mut labels = Labels::new();
    let read =
        |path: &Path| -> Option<Value> { serde_json::from_slice(&fs::read(path).ok()?).ok() };
    if let Some(config) = read(&homes.claude_config)
        && let (Some(account), Some(label)) = (
            text(&config["oauthAccount"]["accountUuid"]),
            claude(&config),
        )
    {
        labels.insert(("claude".into(), account), label);
    }
    if let Some(auth) = read(&homes.codex.join("auth.json"))
        && let (Some(account), Some(label)) = (text(&auth["tokens"]["account_id"]), codex(&auth))
    {
        labels.insert(("codex".into(), account), label);
    }
    if let (Some(identity), Some(auth)) = (
        super::grok::identity(&homes.grok),
        read(&homes.grok.join("auth.json")),
    ) && let Some(label) = auth
        .as_object()
        .into_iter()
        .flat_map(|entries| entries.values())
        .find(|entry| entry["user_id"] == identity.user_id.as_str())
        .and_then(grok)
    {
        let account = serde_json::to_string(&identity).expect("Identity contains only strings.");
        labels.insert(("grok".into(), account), label);
    }
    labels
}

/// The email of the Claude OAuth account in `.claude.json`.
pub fn claude(config: &Value) -> Option<String> {
    text(&config["oauthAccount"]["emailAddress"])
        .or_else(|| text(&config["oauthAccount"]["organizationName"]))
}

/// The email claim of the Codex ID token. The token signature is not checked: the label is display data.
pub fn codex(auth: &Value) -> Option<String> {
    let payload = auth["tokens"]["id_token"].as_str()?.split('.').nth(1)?;
    let claims: Value = serde_json::from_slice(&base64url(payload)?).ok()?;
    text(&claims["email"])
}

/// The email and the team name of one Grok OIDC entry.
pub fn grok(entry: &Value) -> Option<String> {
    let parts: Vec<_> = [text(&entry["email"]), text(&entry["team_name"])]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn base64url(input: &str) -> Option<Vec<u8>> {
    let mut bits = 0_u32;
    let mut count = 0;
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    for byte in input.trim_end_matches('=').bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            output.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A fake unsigned JWT. It holds no real token.
    fn fake_jwt(claims: &Value) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let bytes = claims.to_string().into_bytes();
        let mut text = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0_u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..=chunk.len() {
                text.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        format!("e30.{text}.fake-signature")
    }

    #[test]
    fn codex_label_is_the_email_claim_of_the_id_token() {
        for email in ["a@example.com", "ab@example.com", "abc@example.com"] {
            let auth = json!({"tokens": {
                "id_token": fake_jwt(&json!({"email": email, "sub": "user-1"})),
                "access_token": "fake-access", "account_id": "account-a",
            }});
            assert_eq!(codex(&auth).as_deref(), Some(email));
        }
        assert_eq!(codex(&json!({"tokens": {"id_token": "not-a-jwt"}})), None);
        assert_eq!(codex(&json!({"tokens": {"id_token": "a.%%%.c"}})), None);
        assert_eq!(codex(&json!({})), None);
    }

    #[test]
    fn claude_label_is_the_oauth_email_else_the_organization() {
        let config = json!({"oauthAccount": {"accountUuid": "u", "emailAddress": "c@example.com", "organizationName": "Org"}});
        assert_eq!(claude(&config).as_deref(), Some("c@example.com"));
        let config = json!({"oauthAccount": {"accountUuid": "u", "organizationName": "Org"}});
        assert_eq!(claude(&config).as_deref(), Some("Org"));
        assert_eq!(claude(&json!({})), None);
    }

    #[test]
    fn grok_label_names_the_email_and_the_team() {
        let entry = json!({"email": "g@example.com", "team_name": "Team A", "key": "fake-key", "refresh_token": "fake-refresh"});
        assert_eq!(grok(&entry).as_deref(), Some("g@example.com · Team A"));
        assert_eq!(
            grok(&json!({"team_name": "Team A"})).as_deref(),
            Some("Team A")
        );
        assert_eq!(grok(&json!({"user_id": "u"})), None);
    }

    #[test]
    fn current_labels_only_the_logged_in_accounts_without_tokens() {
        let home = tempfile::tempdir().unwrap();
        let homes = Homes {
            claude_config: home.path().join(".claude.json"),
            codex: home.path().join("codex"),
            grok: home.path().join("grok"),
        };
        assert!(current(&homes).is_empty());
        fs::create_dir_all(&homes.codex).unwrap();
        fs::create_dir_all(&homes.grok).unwrap();
        fs::write(
            &homes.claude_config,
            json!({"oauthAccount": {"accountUuid": "claude-a", "emailAddress": "c@example.com"}})
                .to_string(),
        )
        .unwrap();
        fs::write(
            homes.codex.join("auth.json"),
            json!({"tokens": {"account_id": "codex-a", "access_token": "fake-access",
                "id_token": fake_jwt(&json!({"email": "x@example.com"}))}})
            .to_string(),
        )
        .unwrap();
        fs::write(
            homes.grok.join("auth.json"),
            json!({"issuer::client": {"auth_mode": "oidc", "user_id": "u1", "principal_type": "Team",
                "principal_id": "t1", "team_id": "t1", "email": "g@example.com", "team_name": "Team A",
                "key": "fake-key"}})
            .to_string(),
        )
        .unwrap();
        let labels = current(&homes);
        let get =
            |runner: &str, account: &str| labels.get(&(runner.into(), account.into())).cloned();
        assert_eq!(get("claude", "claude-a").as_deref(), Some("c@example.com"));
        assert_eq!(get("codex", "codex-a").as_deref(), Some("x@example.com"));
        let grok_account =
            r#"{"user_id":"u1","principal_type":"Team","principal_id":"t1","team_id":"t1"}"#;
        assert_eq!(
            get("grok", grok_account).as_deref(),
            Some("g@example.com · Team A")
        );
        assert_eq!(labels.len(), 3);
        assert!(labels.values().all(|label| !label.contains("fake")));
    }
}
