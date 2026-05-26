# mcp-relay

`ippoan/auth-worker` と組む MCP binary の共通ロジック (RFC 8628 device flow / 1-click
pair / WS frame schema / relay loop) を提供する Rust crate。

## 利用 binary

- [`github-mcp-server-rs`](binaries/github-mcp-server-rs/) (旧 `ippoan/github-mcp-server-rs`, archived)
- [`ref-files-mcp-server-rs`](binaries/ref-files-mcp-server-rs/) (旧 `ippoan/ref-files-mcp-server-rs`, archived)

## 依存方法

`Cargo.toml`:

```toml
[dependencies]
mcp-relay = { git = "https://github.com/ippoan/mcp-relay-rs.git", tag = "v0.0.1" }
```

binary 側は `mcp_relay::{auth, config, pair, relay, token_cache}` を `use` する。

## 公開 API

| module | 主要 item |
|---|---|
| `config` | `AuthEnv`, `Config { project_name, ... }` |
| `token_cache` | `TokenSet` (`access_token` / `refresh_token` / `expires_at`) |
| `auth` | `device_flow_start`, `device_flow_poll`, `refresh` |
| `pair` | `POST /mcp/pair/new` client |
| `relay` | `run_relay(RelayContext)`, `run_pair_session(PairRelayContext, code, deadline)` |
| `relay::frame` | `Frame::{Req, Resp, Hello}` (Hello に Phase 2 `service` field) |

## Frame schema

Phase 2 option C (auth-worker 1 DO multiplex) に対応するため `Frame::Hello` に
`service: String` field を追加 (`#[serde(default)]` で v1 sender は
`"github-mcp-server-rs"` に fallback)。詳細は
[`ippoan/ref-files-mcp-server-rs#4`](https://github.com/ippoan/ref-files-mcp-server-rs/issues/4)
の Phase 2 設計判断 (option C) を参照。

## License

MIT
