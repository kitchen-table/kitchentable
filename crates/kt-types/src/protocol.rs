//! JSON-RPC 2.0 envelope and the error taxonomy.
//!
//! Transport is a Unix domain socket at
//! `~/Library/Application Support/KitchenTable/kt.sock`, mode 0600. Possession
//! of the socket is the authorisation: there is no token, because only the
//! logged-in user can open it (docs/architecture.md section 4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Bumped only on a breaking change to the envelope or to a method's shape.
/// Additive changes within a major do not bump it; the shell refuses to talk to
/// a daemon advertising a different major and asks it to drain and restart
/// (docs/architecture.md section 15).
pub const PROTOCOL_VERSION: u32 = 1;

/// A request from a client - the shell, the CLI, or an agent via kt-mcp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    /// Absent for notifications, which expect no reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    pub id: Option<u64>,
    #[serde(flatten)]
    pub payload: ResponsePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponsePayload {
    Result(serde_json::Value),
    Error(KtError),
}

/// Serialises as the literal string `"2.0"`, so a malformed envelope is a
/// deserialisation failure rather than something we have to check by hand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        if v == "2.0" {
            Ok(JsonRpcVersion)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected jsonrpc \"2.0\", got {v:?}"
            )))
        }
    }
}

/// A failure a client can act on.
///
/// Every user-visible failure maps to one of these; the UI branches on `code`
/// and never parses `message`, which is for humans and logs only.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KtError {
    pub code: ErrorCode,
    pub message: String,
    /// Structured context, shape depending on `code`. Never contains secrets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub detail: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    /// Malformed envelope, unknown method, or params that do not deserialise.
    BadRequest,
    /// The daemon understood the request but the subject does not exist.
    NotFound,
    /// A slug, hostname, or invite already exists.
    Conflict,
    /// Refused on policy grounds: a path escaping the app folder, a
    /// cross-app storage read, an admin call from a LAN interface.
    Forbidden,
    /// The workspace, a manifest, or the filesystem is in a state the daemon
    /// will not guess its way out of.
    InvalidState,
    /// Quota exceeded - per-app storage, invite rate limit.
    LimitExceeded,
    /// Underlying IO failed. `detail` carries the path where one applies.
    Io,
    /// A bug in the daemon. Always logged with a backtrace.
    Internal,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::BadRequest => "bad_request",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::Forbidden => "forbidden",
            Self::InvalidState => "invalid_state",
            Self::LimitExceeded => "limit_exceeded",
            Self::Io => "io",
            Self::Internal => "internal",
        };
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_wrong_jsonrpc_version() {
        let bad = r#"{"jsonrpc":"1.0","id":1,"method":"sys.status"}"#;
        assert!(serde_json::from_str::<Request>(bad).is_err());
    }

    #[test]
    fn round_trips_a_request() {
        let json =
            r#"{"jsonrpc":"2.0","id":7,"method":"app.get","params":{"slug":"trip-planner"}}"#;
        let req: Request = serde_json::from_str(json).expect("parses");
        assert_eq!(req.method, "app.get");
        assert_eq!(req.id, Some(7));
        assert_eq!(serde_json::to_string(&req).expect("serialises"), json);
    }

    #[test]
    fn a_notification_has_no_id() {
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: None,
            method: "sys.resume_serving".into(),
            params: None,
        };
        let out = serde_json::to_string(&req).expect("serialises");
        assert_eq!(out, r#"{"jsonrpc":"2.0","method":"sys.resume_serving"}"#);
    }

    #[test]
    fn error_responses_carry_a_machine_readable_code() {
        let res = Response {
            jsonrpc: JsonRpcVersion,
            id: Some(1),
            payload: ResponsePayload::Error(KtError {
                code: ErrorCode::NotFound,
                message: "no app with slug \"nope\"".into(),
                detail: None,
            }),
        };
        let out = serde_json::to_value(&res).expect("serialises");
        assert_eq!(out["error"]["code"], "not_found");
    }
}
