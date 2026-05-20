//! Frame schema for the WebSocket relay (issue #27, paired with auth-worker #117 Phase 7).
//!
//! Wire format: **JSON text frame** on the WS, one Frame per WS message.
//!
//! - `Req`   Claude Code Web → auth-worker `POST /u/<user>/mcp` → DO → binary
//! - `Resp`  binary → DO → auth-worker → Claude Code Web (HTTP response body)
//! - `Hello` binary → DO の application-level handshake (binary 起動直後に 1 回送る)
//!
//! 設計理由:
//! - JSON: msgpack より debug 容易。MCP payload 自体 JSON なので帯域差は誤差。
//! - `body_b64`: HTTP body は任意 byte sequence。base64 standard alphabet で encode して JSON に乗せる。
//! - `id` (uuid v4): 1 WS 上で複数 in-flight を許すための correlation id。auth-worker 側は
//!   `POST /u/:user/mcp` を `Frame::Req { id }` 1 件に変換し、`Frame::Resp` を `id` で待ち合わせる。
//! - `v` / `proto`: forward-compat 用。現行 v1 のみ。auth-worker は `proto != FRAME_VERSION` なら
//!   WS を `close(1002, "proto mismatch")` する想定。
//!
//! Out of scope (frame v2 以降で対応):
//! - SSE event boundary / `Last-Event-ID` / `mcp-session-id` 透過
//! - request cancellation
//! - response chunking (MVP は 1 MiB body 上限)

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Frame schema version. auth-worker Phase 7 plan と同期して bump する。
pub const FRAME_VERSION: u32 = 1;

/// 1 WS frame に乗る payload。`#[serde(tag = "kind")]` で discriminator は `kind`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Frame {
    /// Claude Code Web → binary (HTTP request to be dispatched to MCP service).
    Req {
        v: u32,
        id: String,
        method: String,
        path: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        body_b64: String,
    },
    /// binary → Claude Code Web (HTTP response).
    Resp {
        v: u32,
        id: String,
        status: u16,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        body_b64: String,
    },
    /// binary → auth-worker, application-level handshake. WS open 直後に 1 回。
    Hello {
        v: u32,
        binary_version: String,
        proto: u32,
        /// Phase 2 option C multiplex: auth-worker 1 DO 内で複数 binary を区別する
        /// service id。binary の crate 名 (`"github-mcp-server-rs"` /
        /// `"ref-files-mcp-server-rs"`) を入れる。v1 sender (service field 無し)
        /// との後方互換のため `serde(default)` で `"github-mcp-server-rs"` に
        /// fallback する。
        #[serde(default = "default_service_v1_compat")]
        service: String,
        /// auth-worker issue #178 (a): application-level ping/pong keepalive
        /// に binary が対応していれば `true`。 DO 側はこの field を見て ping
        /// を送るか決める (旧 binary には ping を出さない、 後方互換)。
        /// 旧 sender (field 無し) は false に倒れる。
        #[serde(default)]
        keepalive_supported: bool,
    },
    /// auth-worker DO → binary, application-level keepalive ping (issue #178)。
    /// binary は `Frame::Pong { id }` で即返す。 同 id を echo する事で
    /// DO 側は「どの ping が応答済か」を一応 trace 可能 (現状の DO 実装は
    /// id を見ずに最新の応答有無だけ使う)。
    Ping { v: u32, id: String },
    /// binary → auth-worker DO, Ping への即時応答 (issue #178)。
    Pong { v: u32, id: String },
}

/// v1 frame (service field 無し) 受信時の fallback service id。Phase 1 までは
/// auth-worker 1 DO に 1 binary しか接続しなかったので、必ず github-mcp-server-rs
/// だった、という歴史的事実に基づく。
fn default_service_v1_compat() -> String {
    "github-mcp-server-rs".into()
}

impl Frame {
    /// `Hello` frame を作る。`binary_version` は `CARGO_PKG_VERSION`、`service` は
    /// 各 binary の crate 名 (`"github-mcp-server-rs"` / `"ref-files-mcp-server-rs"`)
    /// を埋める想定。
    ///
    /// auth-worker issue #178: 本 crate がビルドする binary は ping/pong に
    /// 対応しているため `keepalive_supported: true` を載せる。 旧 binary
    /// (本 PR より前にビルドされた) は field を載せず、 auth-worker DO 側で
    /// `serde(default)` により false 扱いになるので ping を受け取らない。
    pub fn hello(binary_version: impl Into<String>, service: impl Into<String>) -> Self {
        Frame::Hello {
            v: FRAME_VERSION,
            binary_version: binary_version.into(),
            proto: FRAME_VERSION,
            service: service.into(),
            keepalive_supported: true,
        }
    }

    /// auth-worker issue #178: 受信 `Ping { id }` への即応答 `Pong { id }` を組む。
    pub fn pong(id: impl Into<String>) -> Self {
        Frame::Pong {
            v: FRAME_VERSION,
            id: id.into(),
        }
    }

    /// `Resp` frame を組み立てる。`body` は raw byte。base64 encode は内部で実施。
    pub fn resp(
        id: impl Into<String>,
        status: u16,
        headers: BTreeMap<String, String>,
        body: &[u8],
    ) -> Self {
        Frame::Resp {
            v: FRAME_VERSION,
            id: id.into(),
            status,
            headers,
            body_b64: encode_body(body),
        }
    }

    /// JSON 文字列に encode (WS Text frame に乗せる用)。
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// JSON 文字列から decode。
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Frame の `v` field (Hello は `v == proto`、Req/Resp は `v` field を持つ) を返す。
    /// 受信側 version check 用。
    pub fn version(&self) -> u32 {
        match self {
            Frame::Req { v, .. }
            | Frame::Resp { v, .. }
            | Frame::Hello { v, .. }
            | Frame::Ping { v, .. }
            | Frame::Pong { v, .. } => *v,
        }
    }
}

/// `Frame::Req.body_b64` / `Frame::Resp.body_b64` の base64 → bytes decode。
pub fn decode_body(b64: &str) -> Result<Vec<u8>, base64::DecodeError> {
    if b64.is_empty() {
        return Ok(Vec::new());
    }
    B64.decode(b64.as_bytes())
}

/// bytes → base64 standard alphabet (no url-safe variant — frame schema は standard 固定)。
pub fn encode_body(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    B64.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_req() -> Frame {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        headers.insert(
            "accept".into(),
            "application/json, text/event-stream".into(),
        );
        Frame::Req {
            v: 1,
            id: "11111111-2222-3333-4444-555555555555".into(),
            method: "POST".into(),
            path: "/".into(),
            headers,
            body_b64: encode_body(br#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#),
        }
    }

    #[test]
    fn req_roundtrip_preserves_fields() {
        let f = sample_req();
        let s = f.to_json().unwrap();
        let parsed = Frame::from_json(&s).unwrap();
        assert_eq!(parsed, f);
    }

    #[test]
    fn resp_roundtrip_preserves_fields() {
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        let f = Frame::resp("abc", 200, headers, br#"{"ok":true}"#);
        let s = f.to_json().unwrap();
        let parsed = Frame::from_json(&s).unwrap();
        assert_eq!(parsed, f);
    }

    #[test]
    fn hello_roundtrip_preserves_fields() {
        let f = Frame::hello("0.1.0", "ref-files-mcp-server-rs");
        let s = f.to_json().unwrap();
        let parsed = Frame::from_json(&s).unwrap();
        assert_eq!(parsed, f);
        assert_eq!(parsed.version(), FRAME_VERSION);
    }

    /// Phase 2 後方互換: v1 sender が送る `service` field 無しの Hello が
    /// `service: "github-mcp-server-rs"` で decode できることを保証する。
    /// これが壊れると deployed v0.0.16 等の旧 binary が auth-worker に弾かれる。
    #[test]
    fn hello_v1_compat_decodes_missing_service_as_github_mcp() {
        let s = r#"{"kind":"hello","v":1,"binary_version":"0.0.16","proto":1}"#;
        let f = Frame::from_json(s).unwrap();
        match f {
            Frame::Hello {
                service,
                keepalive_supported,
                ..
            } => {
                assert_eq!(service, "github-mcp-server-rs");
                // issue #178: 旧 sender が `keepalive_supported` を載せて
                // いない場合は default false に倒れる
                assert!(!keepalive_supported);
            }
            _ => panic!("expected Hello"),
        }
    }

    /// auth-worker issue #178: `Frame::hello()` が新規 ping/pong 対応を opt-in
    /// した状態で Hello を生成する事を保証する。
    #[test]
    fn hello_builder_advertises_keepalive_support() {
        let f = Frame::hello("0.4.0", "github-mcp-server-rs");
        match f {
            Frame::Hello {
                keepalive_supported,
                ..
            } => assert!(keepalive_supported),
            _ => panic!("expected Hello"),
        }
    }

    /// auth-worker issue #178: Ping frame の roundtrip + `pong(id)` builder で
    /// 同 id を echo した Pong frame を作れる事を確認する。
    #[test]
    fn ping_pong_roundtrip_and_builder() {
        let ping = Frame::Ping {
            v: FRAME_VERSION,
            id: "ping-1".into(),
        };
        let s = ping.to_json().unwrap();
        let parsed = Frame::from_json(&s).unwrap();
        assert_eq!(parsed, ping);

        let pong = Frame::pong("ping-1");
        let s = pong.to_json().unwrap();
        let parsed2 = Frame::from_json(&s).unwrap();
        assert_eq!(parsed2, pong);
        match parsed2 {
            Frame::Pong { id, v } => {
                assert_eq!(id, "ping-1");
                assert_eq!(v, FRAME_VERSION);
            }
            _ => panic!("expected Pong"),
        }
    }

    #[test]
    fn body_b64_round_trip() {
        let bytes: &[u8] = &[0u8, 1, 2, 3, 0xff, 0xfe, 0xfd];
        let b64 = encode_body(bytes);
        let back = decode_body(&b64).unwrap();
        assert_eq!(back, bytes);
    }

    #[test]
    fn empty_body_round_trip() {
        assert_eq!(encode_body(&[]), "");
        assert_eq!(decode_body("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let s = r#"{"kind":"bogus","v":1}"#;
        assert!(Frame::from_json(s).is_err());
    }

    #[test]
    fn unknown_field_is_rejected_thanks_to_deny_unknown_fields() {
        // future-proofing: Phase 7 で誤って extra field を入れたら decode に気付ける
        let s =
            r#"{"kind":"hello","v":1,"binary_version":"0.1.0","proto":1,"service":"x","extra":42}"#;
        assert!(Frame::from_json(s).is_err());
    }

    #[test]
    fn missing_id_in_req_is_rejected() {
        let s = r#"{"kind":"req","v":1,"method":"POST","path":"/","headers":{},"body_b64":""}"#;
        assert!(Frame::from_json(s).is_err());
    }

    #[test]
    fn req_with_default_headers_and_body_decodes() {
        // headers / body_b64 は #[serde(default)] なので省略可
        let s = r#"{"kind":"req","v":1,"id":"a","method":"GET","path":"/"}"#;
        let f = Frame::from_json(s).unwrap();
        if let Frame::Req {
            id,
            headers,
            body_b64,
            ..
        } = f
        {
            assert_eq!(id, "a");
            assert!(headers.is_empty());
            assert_eq!(body_b64, "");
        } else {
            panic!("expected Req");
        }
    }

    #[test]
    fn version_field_extraction() {
        assert_eq!(
            Frame::hello("0.1.0", "ref-files-mcp-server-rs").version(),
            1
        );
        assert_eq!(sample_req().version(), 1);
    }
}
