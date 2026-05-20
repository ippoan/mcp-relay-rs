#!/bin/bash
# Reusable Claude Code SessionStart hook published by
# https://github.com/ippoan/mcp-relay-rs (#9 Phase 4)
#
# Purpose:
#   Make the `ref-files-mcp-server-rs` MCP server (now built from this
#   monorepo's `binaries/ref-files-mcp-server-rs/`) available to a Claude Code
#   on the web session from any consumer repo. Outbound WebSocket relay against
#   auth-worker `mcp(-staging).ippoan.org` (issue #27, paired with
#   ippoan/auth-worker#117) — no cloudflared, no inbound port.
#
# Consumer usage — drop this into the consumer repo's
# `.claude/hooks/session-start.sh`:
#
#   #!/bin/bash
#   set -euo pipefail
#   [ "${CLAUDE_CODE_REMOTE:-}" != "true" ] && exit 0
#   curl -sSfL \
#     https://raw.githubusercontent.com/ippoan/mcp-relay-rs/main/.claude/hooks/install-mcp-ref-files.sh \
#     | bash
#
# The old `ippoan/ref-files-mcp-server-rs/.claude/hooks/install-mcp.sh` is now a
# 1-line redirect shim to this file, so existing consumer hooks keep working
# through one extra `curl | bash` hop.
#
# auth-worker INTERNAL_SHARED_SECRET は v0.0.5 から release binary に build-time
# embed されている (#25)。consumer 側 secret 登録は不要。
#
# Optional env (with defaults):
#   REF_FILES_MCP_ENV          staging|prod                          (default: staging)
#   REF_FILES_MCP_PIN_TAG      pin release tag (e.g. v0.0.6, dev-12) (default: resolved per channel)
#   REF_FILES_MCP_CHANNEL      stable|dev                            (default: stable)
#                                   - stable: GitHub `releases/latest` (= 正式 v0.0.X タグ)
#                                   - dev:    `releases?per_page=100` から `dev-N` の max を解決
#                                             (= main push の度に dev-release.yml が打つ prerelease)
#   REF_FILES_MCP_FORCE_REINSTALL=1  force re-download even when tag matches
#   GITHUB_LOGIN            github username (REQUIRED on no-token path,
#                                   used by 1-click pair flow as `claim_login`)
#   REF_FILES_MCP_AUTO_DEVICE_FLOW=1   opt-in to the legacy RFC 8628 device-code
#                                   prompt instead of the 1-click pair flow
#                                   (advanced; CLI / local dev / offline).
#
# Override (advanced; 通常は不要):
#   REF_FILES_MCP_INTERNAL_SHARED_SECRET — embed されている値を上書きしたい時のみ
#                                       (例: 自分の auth-worker fork を叩く dev)
#
# On success:
#   - binary installed at  $HOME/.local/bin/ref-files-mcp-server-rs
#   - relay running (outbound WS to mcp(-staging).ippoan.org)
#   -固定 MCP URL written to:
#       $CLAUDE_PROJECT_DIR/.claude/mcp-state-ref-files/mcp-url
#     and exported as $REF_FILES_MCP_URL via $CLAUDE_ENV_FILE.
#
# Re-running is safe: existing binary / token cache / running relay are reused.

set -euo pipefail

# ─── 0. only run in Claude Code on the web ────────────────────────────────────
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  echo "[install-mcp] skipped: not a remote Claude Code session (CLAUDE_CODE_REMOTE != true)" >&2
  exit 0
fi

REPO="ippoan/mcp-relay-rs"
BIN_NAME="ref-files-mcp-server-rs"
ENV_NAME="${REF_FILES_MCP_ENV:-staging}"

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
INSTALL_DIR="$HOME/.local/bin"
STATE_DIR="$PROJECT_DIR/.claude/mcp-state-ref-files"
mkdir -p "$INSTALL_DIR" "$STATE_DIR"

# Cleanup state files from old cloudflared-based versions (issue #27 hard-cut).
rm -f "$STATE_DIR/serve.pid" "$STATE_DIR/serve.log" \
      "$STATE_DIR/cloudflared.pid" "$STATE_DIR/cloudflared.log" \
      "$STATE_DIR/url" 2>/dev/null || true

# Make $HOME/.local/bin reachable for the rest of the session.
case ":$PATH:" in
  *":$INSTALL_DIR:"*) : ;;
  *) export PATH="$INSTALL_DIR:$PATH" ;;
esac
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$CLAUDE_ENV_FILE"
fi

# ─── 1. resolve target release tag & (re)download if stale ───────────────────
BIN="$INSTALL_DIR/$BIN_NAME"
TAG_FILE="$BIN.tag"

CHANNEL="${REF_FILES_MCP_CHANNEL:-stable}"
if [ -n "${REF_FILES_MCP_PIN_TAG:-}" ]; then
  TAG="$REF_FILES_MCP_PIN_TAG"
elif [ "$CHANNEL" = "dev" ]; then
  echo "[install-mcp] resolving latest dev release tag (channel=dev)..." >&2
  DEV_N="$(curl -sSfL "https://api.github.com/repos/$REPO/releases?per_page=100" \
            | grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"dev-[0-9]+"' \
            | cut -d'"' -f4 \
            | sed 's|^dev-||' \
            | sort -n \
            | tail -1 || true)"
  if [ -n "$DEV_N" ]; then
    TAG="dev-$DEV_N"
  fi
elif [ "$CHANNEL" != "stable" ]; then
  echo "[install-mcp] ERROR: unknown REF_FILES_MCP_CHANNEL=$CHANNEL (expected: stable, dev)" >&2
  exit 1
else
  echo "[install-mcp] resolving latest release tag (channel=stable)..." >&2
  TAG="$(curl -sSfL "https://api.github.com/repos/$REPO/releases/latest" \
          | grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"' \
          | head -1 | cut -d'"' -f4)"
fi
if [ -z "${TAG:-}" ]; then
  echo "[install-mcp] ERROR: could not resolve a release tag for $REPO (channel=$CHANNEL)" >&2
  exit 1
fi

# Per-binary stable tag は `ref-files-mcp-server-rs-v0.0.X` の形で打たれる
# (`release.yml` の `tag_strip_prefix: ref-files-mcp-server-rs-` と対称)。
# 詳細は install-mcp.sh (github 用) の同じ block を参照。
# (quoted expansion は shellcheck SC2295 回避用、bash 4.4+ の literal-strip)
ASSET_TAG="${TAG#"${BIN_NAME}-"}"

INSTALLED_TAG=""
[ -s "$TAG_FILE" ] && INSTALLED_TAG="$(cat "$TAG_FILE" 2>/dev/null || true)"

EMBEDDED_TAG=""
if [ -x "$BIN" ]; then
  EMBEDDED_TAG="$("$BIN" --version 2>/dev/null \
    | grep -oE '\((v[0-9]|dev-)[^)]*\)' \
    | head -1 \
    | tr -d '()' || true)"
fi

need_install=0
if [ ! -x "$BIN" ]; then
  need_install=1
elif [ "$INSTALLED_TAG" != "$TAG" ]; then
  echo "[install-mcp] upgrading binary: $INSTALLED_TAG -> $TAG" >&2
  need_install=1
elif [ -n "$EMBEDDED_TAG" ] && [ "$EMBEDDED_TAG" != "$ASSET_TAG" ] && [ "$EMBEDDED_TAG" != "$TAG" ]; then
  echo "[install-mcp] binary embeds $EMBEDDED_TAG but expected $TAG / $ASSET_TAG -- re-downloading" >&2
  need_install=1
elif [ "${REF_FILES_MCP_FORCE_REINSTALL:-}" = "1" ]; then
  echo "[install-mcp] REF_FILES_MCP_FORCE_REINSTALL=1 set, re-downloading $TAG" >&2
  need_install=1
fi

if [ "$need_install" = "1" ]; then
  ASSET="${BIN_NAME}-${ASSET_TAG}-x86_64-unknown-linux-gnu.tar.gz"
  URL="https://github.com/$REPO/releases/download/$TAG/$ASSET"
  echo "[install-mcp] downloading $ASSET (from $TAG)..." >&2
  TMP="$(mktemp -d)"
  curl -sSfL "$URL" -o "$TMP/binary.tar.gz"
  curl -sSfL "$URL.sha256" -o "$TMP/binary.tar.gz.sha256" || true
  if [ -s "$TMP/binary.tar.gz.sha256" ]; then
    EXPECTED="$(awk '{print $1}' "$TMP/binary.tar.gz.sha256")"
    ACTUAL="$(sha256sum "$TMP/binary.tar.gz" | awk '{print $1}')"
    if [ "$EXPECTED" != "$ACTUAL" ]; then
      echo "[install-mcp] ERROR: sha256 mismatch (expected=$EXPECTED actual=$ACTUAL)" >&2
      exit 1
    fi
  fi
  tar -xzf "$TMP/binary.tar.gz" -C "$TMP"
  EXTRACTED="$(find "$TMP" -maxdepth 3 -type f -name "$BIN_NAME" -perm -u+x | head -1)"
  if [ -z "$EXTRACTED" ]; then
    echo "[install-mcp] ERROR: $BIN_NAME not found in $ASSET" >&2
    ls -la "$TMP" >&2
    exit 1
  fi
  install -m 0755 "$EXTRACTED" "$BIN"
  printf '%s\n' "$TAG" > "$TAG_FILE"
  rm -rf "$TMP"
fi
echo "[install-mcp] binary: $($BIN --version 2>/dev/null || echo "$BIN") (tag=$TAG)" >&2

# ─── 2. binary は relay subcommand を持つか? (古い tag の pin 対策) ──────────
if ! "$BIN" relay --help >/dev/null 2>&1; then
  echo "[install-mcp] ERROR: installed binary does not support 'relay' subcommand." >&2
  echo "[install-mcp]        Required: v0.0.6 or later (issue #27)." >&2
  echo "[install-mcp]        If REF_FILES_MCP_PIN_TAG is set, bump it to v0.0.6+." >&2
  exit 1
fi

# ─── 3. device-flow auth if no token cache yet ────────────────────────────────
TOKEN_FILE="$HOME/.config/ref-files-mcp-server-rs/token-${ENV_NAME}.json"
if [ ! -f "$TOKEN_FILE" ] && [ -n "${REF_FILES_MCP_TOKEN_JSON:-}" ]; then
  echo "[install-mcp] hydrating $TOKEN_FILE from \$REF_FILES_MCP_TOKEN_JSON" >&2
  mkdir -p "$(dirname "$TOKEN_FILE")"
  printf '%s' "$REF_FILES_MCP_TOKEN_JSON" > "$TOKEN_FILE"
  chmod 600 "$TOKEN_FILE"
fi
if [ ! -f "$TOKEN_FILE" ]; then
  if [ "${REF_FILES_MCP_AUTO_DEVICE_FLOW:-}" = "1" ]; then
    echo "" >&2
    echo "[install-mcp] ───── device authorization required (env=$ENV_NAME) ─────" >&2
    echo "[install-mcp] (\$REF_FILES_MCP_AUTO_DEVICE_FLOW=1 opt-in path)" >&2
    echo "[install-mcp] OPEN the verification_uri_complete URL printed below in a" >&2
    echo "[install-mcp] browser, sign in with GitHub, and Approve.  The hook will" >&2
    echo "[install-mcp] block until polling completes." >&2
    echo "[install-mcp]" >&2
    echo "[install-mcp] Tip: to skip this prompt on future fresh containers, copy" >&2
    echo "[install-mcp]   $TOKEN_FILE" >&2
    echo "[install-mcp] into a CCoW Setup-script secret named REF_FILES_MCP_TOKEN_JSON." >&2
    echo "" >&2
    "$BIN" auth --env "$ENV_NAME" >&2
  else
    if [ -z "${GITHUB_LOGIN:-}" ]; then
      echo "" >&2
      echo "[install-mcp] ERROR: \$GITHUB_LOGIN is not set, and no token cache exists." >&2
      echo "[install-mcp]" >&2
      echo "[install-mcp] The 1-click pair flow needs to know your GitHub username so" >&2
      echo "[install-mcp] the auth-worker can match your browser cookie session against" >&2
      echo "[install-mcp] the pair_code this binary mints. Add it as an env var:" >&2
      echo "[install-mcp]" >&2
      echo "[install-mcp]   Claude Code on the Web → Settings → Environment variables" >&2
      echo "[install-mcp]   GITHUB_LOGIN=<your-github-username>" >&2
      echo "[install-mcp]" >&2
      echo "[install-mcp] Alternatively, set \$REF_FILES_MCP_AUTO_DEVICE_FLOW=1 to fall" >&2
      echo "[install-mcp] back to the legacy RFC 8628 device-code prompt." >&2
      exit 1
    fi

    for pidfile in "$STATE_DIR/pair.pid" "$STATE_DIR/relay.pid"; do
      [ -f "$pidfile" ] || continue
      old_pid="$(cat "$pidfile" 2>/dev/null || true)"
      if [ -n "$old_pid" ] && kill -0 "$old_pid" 2>/dev/null; then
        kill "$old_pid" 2>/dev/null || true
      fi
      rm -f "$pidfile"
    done

    : > "$STATE_DIR/pair.log"
    nohup "$BIN" pair --env "$ENV_NAME" \
        --user "$GITHUB_LOGIN" \
        --state-dir "$STATE_DIR" \
        > "$STATE_DIR/pair.log" 2>&1 &
    echo $! > "$STATE_DIR/pair.pid"

    LINK=""
    for _ in $(seq 1 5); do
      if grep -qE 'https?://[^[:space:]]+/mcp/pair/[A-Za-z0-9_-]{30,60}([[:space:]]|$)' "$STATE_DIR/pair.log" 2>/dev/null; then
        LINK="$(grep -oE 'https?://[^[:space:]]+/mcp/pair/[A-Za-z0-9_-]{30,60}' "$STATE_DIR/pair.log" \
                | head -1 || true)"
        [ -n "$LINK" ] && break
      fi
      if ! kill -0 "$(cat "$STATE_DIR/pair.pid")" 2>/dev/null; then
        echo "[install-mcp] ERROR: pair process exited during startup. Log:" >&2
        tail -n 30 "$STATE_DIR/pair.log" >&2 || true
        exit 1
      fi
      sleep 1
    done

    echo "" >&2
    echo "[install-mcp] ─── 1-click pair required ────────────────────────" >&2
    if [ -n "$LINK" ]; then
      echo "[install-mcp]   → $LINK" >&2
    else
      echo "[install-mcp] (waiting; see $STATE_DIR/pair.log)" >&2
    fi
    echo "[install-mcp] open the link in a browser (auth.ippoan.org session is sticky;" >&2
    echo "[install-mcp]   1 click should complete pair within ~5s)." >&2
    echo "[install-mcp]" >&2
    echo "[install-mcp] The binary will bridge MCP traffic as soon as you click. To" >&2
    echo "[install-mcp] skip this step on future containers, drop the token cache JSON" >&2
    echo "[install-mcp] into a CCoW Setup-script secret named \$REF_FILES_MCP_TOKEN_JSON," >&2
    echo "[install-mcp] or set \$REF_FILES_MCP_AUTO_DEVICE_FLOW=1 for the legacy CLI prompt." >&2
    echo "" >&2

    PAIR_MODE=1
  fi
fi

# ─── 4. (re)start relay in the background ─────────────────────────────────────
if [ "${PAIR_MODE:-0}" != "1" ]; then
  if [ -f "$STATE_DIR/relay.pid" ]; then
    old_pid="$(cat "$STATE_DIR/relay.pid" 2>/dev/null || true)"
    if [ -n "$old_pid" ] && kill -0 "$old_pid" 2>/dev/null; then
      kill "$old_pid" 2>/dev/null || true
      sleep 1
    fi
  fi

  : > "$STATE_DIR/relay.log"
  nohup "$BIN" relay --env "$ENV_NAME" --state-dir "$STATE_DIR" \
    > "$STATE_DIR/relay.log" 2>&1 &
  echo $! > "$STATE_DIR/relay.pid"
fi

# ─── 5. publish MCP public URL ───────────────────────────────────────────────
if [ "${PAIR_MODE:-0}" = "1" ]; then
  case "$ENV_NAME" in
    prod)    MCP_HOST="mcp.ippoan.org" ;;
    staging) MCP_HOST="mcp-staging.ippoan.org" ;;
    *)       MCP_HOST="mcp-staging.ippoan.org" ;;
  esac
  MCP_URL="https://${MCP_HOST}/u/${GITHUB_LOGIN}/mcp"
  echo "$MCP_URL" > "$STATE_DIR/url"
else
  ready=0
  for _ in $(seq 1 30); do
    if [ -s "$STATE_DIR/url" ]; then
      ready=1; break
    fi
    if ! kill -0 "$(cat "$STATE_DIR/relay.pid")" 2>/dev/null; then
      echo "[install-mcp] ERROR: relay process died during startup. Log:" >&2
      tail -n 50 "$STATE_DIR/relay.log" >&2 || true
      exit 1
    fi
    sleep 1
  done
  if [ "$ready" != "1" ]; then
    echo "[install-mcp] ERROR: relay did not produce $STATE_DIR/url within 30s." >&2
    tail -n 50 "$STATE_DIR/relay.log" >&2 || true
    exit 1
  fi
  MCP_URL="$(cat "$STATE_DIR/url")"
fi

echo "$MCP_URL" > "$STATE_DIR/mcp-url"
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo "export REF_FILES_MCP_URL=\"$MCP_URL\"" >> "$CLAUDE_ENV_FILE"
fi

if [ "${PAIR_MODE:-0}" = "1" ]; then
  cat >&2 <<EOF

[install-mcp] ✓ ref-files-mcp-server-rs is ready (pair mode, waiting on browser click).
[install-mcp]   MCP URL (Streamable HTTP via auth-worker WS relay): $MCP_URL
[install-mcp]   This URL is **stable** — register it once in Claude Code Web's MCP
[install-mcp]   settings; the bridge comes up the moment you click the pair link above.
[install-mcp]   Also exported as \$REF_FILES_MCP_URL and written to:
[install-mcp]     $STATE_DIR/mcp-url
[install-mcp]   Pair log: $STATE_DIR/pair.log
EOF
else
  cat >&2 <<EOF

[install-mcp] ✓ ref-files-mcp-server-rs is ready (relay mode).
[install-mcp]   MCP URL (Streamable HTTP via auth-worker WS relay): $MCP_URL
[install-mcp]   This URL is **stable** — register it once in Claude Code Web's MCP
[install-mcp]   settings, no need to update per-session.
[install-mcp]   Also exported as \$REF_FILES_MCP_URL and written to:
[install-mcp]     $STATE_DIR/mcp-url
[install-mcp]   Relay log: $STATE_DIR/relay.log
EOF
fi
