//! Bearer tokens of the remote MCP endpoint and the audit table of its calls.
//!
//! The hub shows a token one time, when the human creates it. It stores only the SHA-256 hash
//! of the token. A revoked token stops working at once, because each `/mcp` request looks the
//! hash up again.

use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use http_body_util::{BodyExt, Full, Limited};
use hyper::{Method, Request, Response, StatusCode, body::Bytes, body::Incoming};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    identity::Result,
    registry::DB_FILE,
    server::{json, reply},
};

/// The only scope of a token. The MCP tools read and never write.
pub const SCOPE_READ: &str = "read";

const TOKEN_PREFIX: &str = "wsmcp_";
const TOKENS_PATH: &str = "/v1/tokens/";

/// One token as `GET /v1/tokens` shows it. The row has no token and no hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRow {
    pub name: String,
    pub scope: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

/// One audited MCP tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRow {
    pub ts_ms: i64,
    pub token: String,
    pub tool: String,
    pub result: String,
}

/// The body of `POST /v1/tokens`.
#[derive(Deserialize)]
struct CreateToken {
    name: String,
}

pub struct TokenStore {
    connection: Mutex<Connection>,
}

impl TokenStore {
    /// Opens the token and audit tables in the hub database of `state`.
    pub fn open(state: &Path) -> Result<Self> {
        fs::create_dir_all(state)?;
        let connection = Connection::open(state.join(DB_FILE))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS mcp_tokens (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                hash TEXT NOT NULL UNIQUE,
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                revoked_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS mcp_audit (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_ms INTEGER NOT NULL,
                token TEXT NOT NULL,
                tool TEXT NOT NULL,
                result TEXT NOT NULL
            );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// Creates a token and returns it. This is the only time the hub knows the token.
    pub fn create(&self, name: &str) -> Result<String> {
        if name.trim().is_empty() {
            return Err("The token name is empty.".into());
        }
        let connection = self.connection.lock().unwrap();
        let taken: Option<i64> = connection
            .query_row(
                "SELECT id FROM mcp_tokens WHERE name = ?1 AND revoked_at IS NULL",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(format!("A token named {name} exists.").into());
        }
        let token = format!("{TOKEN_PREFIX}{}", random_hex()?);
        connection.execute(
            "INSERT INTO mcp_tokens (name, hash, scope, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, hash(&token), SCOPE_READ, now_ms() / 1000],
        )?;
        Ok(token)
    }

    pub fn list(&self) -> Result<Vec<TokenRow>> {
        let connection = self.connection.lock().unwrap();
        let mut statement = connection
            .prepare("SELECT name, scope, created_at, revoked_at FROM mcp_tokens ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok(TokenRow {
                    name: row.get(0)?,
                    scope: row.get(1)?,
                    created_at: row.get(2)?,
                    revoked_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Revokes the unrevoked tokens with this name. Returns false when there is none.
    pub fn revoke(&self, name: &str) -> Result<bool> {
        let changed = self.connection.lock().unwrap().execute(
            "UPDATE mcp_tokens SET revoked_at = ?2 WHERE name = ?1 AND revoked_at IS NULL",
            params![name, now_ms() / 1000],
        )?;
        Ok(changed > 0)
    }

    /// The name of the unrevoked token, or `None` for an unknown or revoked token.
    pub fn authenticate(&self, token: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT name FROM mcp_tokens WHERE hash = ?1 AND revoked_at IS NULL",
                [hash(token)],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn record_call(&self, token: &str, tool: &str, result: &str) -> Result<()> {
        self.connection.lock().unwrap().execute(
            "INSERT INTO mcp_audit (ts_ms, token, tool, result) VALUES (?1, ?2, ?3, ?4)",
            params![now_ms(), token, tool, result],
        )?;
        Ok(())
    }

    /// The audited calls, oldest first.
    pub fn audit(&self) -> Result<Vec<AuditRow>> {
        let connection = self.connection.lock().unwrap();
        let mut statement =
            connection.prepare("SELECT ts_ms, token, tool, result FROM mcp_audit ORDER BY id")?;
        let rows = statement
            .query_map([], |row| {
                Ok(AuditRow {
                    ts_ms: row.get(0)?,
                    token: row.get(1)?,
                    tool: row.get(2)?,
                    result: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }
}

/// Serves `/v1/tokens`, `/v1/tokens/<name>` and `/v1/audit`. The caller checked the device.
pub async fn admin(store: &TokenStore, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let outcome = match (&method, path.as_str()) {
        (&Method::GET, "/v1/audit") => store.audit().map(|rows| json(StatusCode::OK, &rows)),
        (&Method::GET, "/v1/tokens") => store.list().map(|rows| json(StatusCode::OK, &rows)),
        (&Method::POST, "/v1/tokens") => return create(store, request).await,
        (&Method::DELETE, _) if path.starts_with(TOKENS_PATH) => {
            let Some(name) = decode_segment(&path[TOKENS_PATH.len()..]) else {
                return reply(
                    StatusCode::BAD_REQUEST,
                    "The token name has a bad encoding.",
                );
            };
            store.revoke(&name).map(|found| {
                if found {
                    reply(StatusCode::NO_CONTENT, "")
                } else {
                    reply(StatusCode::NOT_FOUND, "No such token.")
                }
            })
        }
        _ => Ok(reply(StatusCode::NOT_FOUND, "Unknown route.")),
    };
    outcome.unwrap_or_else(|error| reply(StatusCode::INTERNAL_SERVER_ERROR, &error.to_string()))
}

/// The path of one token: `/v1/tokens/` and the name, percent-encoded as one path segment.
/// Each byte outside the RFC 3986 unreserved set is encoded.
pub fn token_path(name: &str) -> String {
    let mut path = String::from(TOKENS_PATH);
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            path.push(char::from(byte));
        } else {
            path.push_str(&format!("%{byte:02X}"));
        }
    }
    path
}

async fn create(store: &TokenStore, request: Request<Incoming>) -> Response<Full<Bytes>> {
    let Ok(body) = Limited::new(request.into_body(), 4096).collect().await else {
        return reply(StatusCode::BAD_REQUEST, "Cannot read the request body.");
    };
    let Ok(CreateToken { name }) = serde_json::from_slice(&body.to_bytes()) else {
        return reply(StatusCode::BAD_REQUEST, "The body needs a name.");
    };
    match store.create(&name) {
        Ok(token) => json(
            StatusCode::CREATED,
            &serde_json::json!({ "name": name, "scope": SCOPE_READ, "token": token }),
        ),
        Err(error) => reply(StatusCode::CONFLICT, &error.to_string()),
    }
}

/// Decodes one percent-encoded path segment. `None` means a bad escape, a raw `/` or a name
/// that is not UTF-8.
fn decode_segment(segment: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(segment.len());
    let mut rest = segment.as_bytes();
    while let [first, tail @ ..] = rest {
        match first {
            b'%' => {
                let [high, low, after @ ..] = tail else {
                    return None;
                };
                bytes.push(hex_value(*high)? << 4 | hex_value(*low)?);
                rest = after;
            }
            b'/' => return None,
            _ => {
                bytes.push(*first);
                rest = tail;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit).to_digit(16).map(|value| value as u8)
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn random_hex() -> Result<String> {
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, TokenStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn a_token_authenticates_until_it_is_revoked() {
        let (_dir, store) = store();
        let token = store.create("claude").unwrap();
        assert_eq!(
            store.authenticate(&token).unwrap().as_deref(),
            Some("claude")
        );
        assert_eq!(store.authenticate("wsmcp_other").unwrap(), None);
        assert!(store.revoke("claude").unwrap());
        assert_eq!(store.authenticate(&token).unwrap(), None);
        assert!(!store.revoke("claude").unwrap());
    }

    #[test]
    fn the_hub_stores_the_hash_and_never_the_token() {
        let (dir, store) = store();
        let token = store.create("claude").unwrap();
        let db = fs::read(dir.path().join(DB_FILE)).unwrap();
        assert!(
            !db.windows(token.len())
                .any(|window| window == token.as_bytes())
        );
        let rows = store.list().unwrap();
        assert_eq!(
            (
                rows[0].name.as_str(),
                rows[0].scope.as_str(),
                rows[0].revoked_at
            ),
            ("claude", "read", None)
        );
    }

    #[test]
    fn a_name_is_free_again_after_its_token_is_revoked_and_tokens_differ() {
        let (_dir, store) = store();
        let first = store.create("claude").unwrap();
        assert!(store.create("claude").is_err());
        store.revoke("claude").unwrap();
        let second = store.create("claude").unwrap();
        assert_ne!(first, second);
        assert_eq!(store.authenticate(&first).unwrap(), None);
        assert!(store.authenticate(&second).unwrap().is_some());
        assert!(store.create(" ").is_err());
    }

    #[test]
    fn audit_rows_keep_the_call_order() {
        let (_dir, store) = store();
        store.record_call("claude", "list_sessions", "ok").unwrap();
        store.record_call("claude", "get_session", "error").unwrap();
        let rows = store.audit().unwrap();
        let calls: Vec<_> = rows
            .iter()
            .map(|row| (row.token.as_str(), row.tool.as_str(), row.result.as_str()))
            .collect();
        assert_eq!(
            calls,
            [
                ("claude", "list_sessions", "ok"),
                ("claude", "get_session", "error")
            ]
        );
        assert!(rows[0].ts_ms > 0);
    }

    #[test]
    fn a_token_name_round_trips_through_one_path_segment() {
        for name in ["grok", "grok?bot", "grok bot", "a/b#c%d", "naïve"] {
            let path = token_path(name);
            let segment = &path[TOKENS_PATH.len()..];
            assert!(!segment.contains(['/', '?', '#', ' ']), "{segment}");
            assert_eq!(decode_segment(segment).as_deref(), Some(name));
        }
        assert_eq!(token_path("grok?bot"), "/v1/tokens/grok%3Fbot");
    }

    #[test]
    fn a_bad_segment_encoding_decodes_to_none() {
        for segment in ["%", "%4", "%zz", "%+1", "a/b", "%FF"] {
            assert_eq!(decode_segment(segment), None, "{segment}");
        }
    }
}
