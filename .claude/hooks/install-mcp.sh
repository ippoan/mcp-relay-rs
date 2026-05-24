#!/bin/bash
# Reusable Claude Code SessionStart hook published by
# https://github.com/ippoan/mcp-relay-rs (#9 Phase 4)
#
# Purpose:
#   Make the `github-mcp-server-rs` MCP server (now built from this monorepo's
#   `binaries/github-mcp-server-rs/`) available to a Claude Code on the web
#   session from any consumer repo. Outbound WebSocket relay against auth-worker
#   `mcp(-staging).ippoan.org` (issue #27, paired with ippoan/auth-worker#117) —
#   no cloudflared, no inbound port.
#
# Consumer usage — drop this into the consumer repo's
# `.claude/hooks/session-start.sh`:
#
#   #!/bin/bash
#   set -euo pipefail
#   [ "${CLAUDE_CODE_REMOTE:-}" != "true" ] && exit 0
#   curl -sSfL \
#     https://raw.githubusercontent.com/ippoan/mcp-relay-rs/main/.claude/hooks/install-mcp.sh \
#     | bash
#
# The old `ippoan/github-mcp-server-rs/.claude/hooks/install-mcp.sh` is now a
# 1-line redirect shim to this file, so existing consumer hooks keep working
# through one extra `curl | bash` hop.
#
# auth-worker INTERNAL_SHARED_SECRET は v0.0.5 から release binary に build-time
# embed されている (#25)。consumer 側 secret 登録は不要。
#
# Optional env (with defaults):
#   GITHUB_MCP_ENV          staging|prod                          (default: staging)
#   GITHUB_MCP_PIN_TAG      pin release tag (e.g. v0.0.6, dev-12) (default: resolved per channel)
#   GITHUB_MCP_CHANNEL      stable|dev                            (default: dev)
#                                   - dev:    `releases?per_page=100` から `dev-N` の max を解決
#                                             (= main push の度に dev-release.yml が打つ prerelease)
#                                             デフォルト: stable v0.0.X タグはまだ運用されておらず
#                                             channel=stable では tag 解決 fail (Refs ippoan/claude-md#38)
#                                   - stable: GitHub `releases/latest` (= 正式 v0.0.X タグ)
#   GITHUB_MCP_FORCE_REINSTALL=1  force re-download even when tag matches
#   GITHUB_LOGIN            github username (REQUIRED on no-token path,
#                                   used by 1-click pair flow as `claim_login`)
#   GITHUB_MCP_AUTO_DEVICE_FLOW=1   opt-in to the legacy RFC 8628 device-code
#                                   prompt instead of the 1-click pair flow
#                                   (advanced; CLI / local dev / offline).
#
# Override (advanced; 通常は不要):
#   GITHUB_MCP_INTERNAL_SHARED_SECRET — embed されている値を上書きしたい時のみ
#                                       (例: 自分の auth-worker fork を叩く dev)
#
# On success:
#   - binary installed at  $HOME/.local/bin/github-mcp-server-rs
#   - relay running (outbound WS to mcp(-staging).ippoan.org)
#   -固定 MCP URL written to:
#       $CLAUDE_PROJECT_DIR/.claude/mcp-state/mcp-url
#     and exported as $GITHUB_MCP_URL via $CLAUDE_ENV_FILE.
#
# Re-running is safe: existing binary / token cache / running relay are reused.

set -euo pipefail

# ─── 0. only run in Claude Code on the web ────────────────────────────────────
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  echo "[install-mcp] skipped: not a remote Claude Code session (CLAUDE_CODE_REMOTE != true)" >&2
  exit 0
fi

REPO="ippoan/mcp-relay-rs"
BIN_NAME="github-mcp-server-rs"
ENV_NAME="${GITHUB_MCP_ENV:-staging}"

PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
INSTALL_DIR="$HOME/.local/bin"
STATE_DIR="$PROJECT_DIR/.claude/mcp-state"
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
# Issue: previously the script skipped download whenever `$BIN` existed,
# so consumers stayed pinned to whatever tag was first installed (e.g.
# v0.0.10 staying live while v0.0.11 was already cut). The relay then
# advertised a stale tools/list (missing tools added in newer tags).
#
# Fix: always resolve the desired TAG, compare against `$BIN.tag` (the tag
# we recorded at last successful install), and re-download on mismatch.
# Honors `GITHUB_MCP_FORCE_REINSTALL=1` for ad-hoc forced refresh.
BIN="$INSTALL_DIR/$BIN_NAME"
TAG_FILE="$BIN.tag"

CHANNEL="${GITHUB_MCP_CHANNEL:-dev}"
# Resolve tags via `git ls-remote --tags` over **git smart-protocol**, which is
# anonymous-unlimited on public GitHub repos and bypasses the `api.github.com`
# rate limit entirely. Two source candidates, tried in order:
#
#   1. CCoW git proxy (`http://local_proxy@127.0.0.1:<port>/git/<owner>/<repo>`)
#      — Anthropic's per-session authenticated git mediator, picked up when
#      the attached consumer repo's origin URL has this shape.
#   2. Anonymous `https://github.com/<owner>/<repo>.git` — universal fallback
#      that works on every environment with git installed (local CLI runs,
#      CI runners, CCoW containers without an attached proxy, etc).
#
# Both paths use `git ls-remote` over HTTPS/git protocol — neither hits
# `api.github.com`, so the shared-IP rate-limit problem that anonymous REST
# requests routinely run into (`HTTP 403 — API rate limit exceeded for
# <CCoW egress IP>`, ippoan/auth-worker#174 / mcp-relay-rs#15 で観測) は
# structurally impossible. The earlier REST fallback was removed for this
# reason — having an "occasionally working" path was worse than a uniform
# anonymous-git path that works regardless of CCoW proxy presence.
_proxy_origin="$(git -C "$PROJECT_DIR" remote get-url origin 2>/dev/null || true)"
if [[ "$_proxy_origin" == *"local_proxy@"*"/git/"* ]]; then
  _proxy_base="${_proxy_origin%/git/*}"
  _target_remote="${_proxy_base}/git/$REPO"
  echo "[install-mcp] resolving release tag via CCoW git proxy ($CHANNEL)..." >&2
else
  _target_remote="https://github.com/$REPO.git"
  echo "[install-mcp] resolving release tag via anonymous git ls-remote ($CHANNEL)..." >&2
fi
ALL_TAGS="$(git ls-remote --tags --refs "$_target_remote" 2>/dev/null \
            | awk -F'refs/tags/' '/refs\/tags\// {print $2}')"

if [ -n "${GITHUB_MCP_PIN_TAG:-}" ]; then
  TAG="$GITHUB_MCP_PIN_TAG"
elif [ "$CHANNEL" = "dev" ]; then
  DEV_N="$(printf '%s\n' "$ALL_TAGS" \
            | grep -E '^dev-[0-9]+$' \
            | sed 's|^dev-||' \
            | sort -n \
            | tail -1 || true)"
  if [ -n "$DEV_N" ]; then
    TAG="dev-$DEV_N"
  fi
elif [ "$CHANNEL" != "stable" ]; then
  echo "[install-mcp] ERROR: unknown GITHUB_MCP_CHANNEL=$CHANNEL (expected: stable, dev)" >&2
  exit 1
else
  # stable: prefer per-binary tag (`github-mcp-server-rs-v0.0.X`) then fall
  # back to monorepo-wide (`v0.0.X`). `sort -V` handles semver ordering.
  TAG="$(printf '%s\n' "$ALL_TAGS" \
          | grep -E "^${BIN_NAME}-v[0-9]+\\.[0-9]+\\.[0-9]+\$" \
          | sort -V | tail -1 || true)"
  if [ -z "$TAG" ]; then
    TAG="$(printf '%s\n' "$ALL_TAGS" \
            | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
            | sort -V | tail -1 || true)"
  fi
fi
if [ -z "${TAG:-}" ]; then
  echo "[install-mcp] ERROR: could not resolve a release tag for $REPO (channel=$CHANNEL)" >&2
  exit 1
fi

# Per-binary stable tag は `github-mcp-server-rs-v0.0.18` の形で打たれる
# (`release.yml` の `tag_strip_prefix: github-mcp-server-rs-` と対称)。
# asset 名は prefix を strip した側に揃える:
#   - tag = github-mcp-server-rs-v0.0.18 → asset = github-mcp-server-rs-v0.0.18-...
#   - tag = v0.0.1 (monorepo-wide)        → asset = github-mcp-server-rs-v0.0.1-...
#   - tag = dev-5                         → asset = github-mcp-server-rs-dev-5-...
# URL path のほうは元 tag をそのまま使う。
# (quoted expansion は shellcheck SC2295 回避用、bash 4.4+ の literal-strip)
ASSET_TAG="${TAG#"${BIN_NAME}-"}"

INSTALLED_TAG=""
[ -s "$TAG_FILE" ] && INSTALLED_TAG="$(cat "$TAG_FILE" 2>/dev/null || true)"

# Read the release tag the binary was built from. Release builds embed it
# via build.rs (`BUILD_RELEASE_TAG` from `GITHUB_REF_NAME` on tag push),
# and clap prints it in parentheses, e.g.:
#   github-mcp-server-rs 0.1.0 (v0.0.11)
#   github-mcp-server-rs 0.1.0 (dev-12)
# Dev/local builds (no tag push) emit no parens, so $EMBEDDED_TAG stays empty.
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
  # Extra guard added on top of #39's TAG_FILE check: the file can lie
  # (manual touch, partial install, copy from another host), so cross-check
  # against the tag the binary itself was built from. Empty EMBEDDED_TAG
  # means a pre-guard release or a local dev build — skip the check in
  # that case to avoid clobbering legitimate dev binaries.
  #
  # Phase 4: monorepo binaries are built from a per-binary tag
  # (`github-mcp-server-rs-v0.0.X`) but build.rs records `GITHUB_REF_NAME`
  # verbatim, so the embedded tag may equal either the full tag or the
  # stripped form. Accept both.
  echo "[install-mcp] binary embeds $EMBEDDED_TAG but expected $TAG / $ASSET_TAG -- re-downloading" >&2
  need_install=1
elif [ "${GITHUB_MCP_FORCE_REINSTALL:-}" = "1" ]; then
  echo "[install-mcp] GITHUB_MCP_FORCE_REINSTALL=1 set, re-downloading $TAG" >&2
  need_install=1
fi

if [ "$need_install" = "1" ]; then
  # Note: the existing relay process (if any) is killed in step 4 below
  # before being restarted, so it picks up the new binary at $BIN.
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
  echo "[install-mcp]        If GITHUB_MCP_PIN_TAG is set, bump it to v0.0.6+." >&2
  exit 1
fi

# ─── 3. device-flow auth if no token cache yet ────────────────────────────────
# CCoW (Claude Code on the web) containers are ephemeral: $HOME is wiped on
# reclaim, so the local token cache file disappears too — every new container
# would otherwise re-prompt for device-flow auth. To make a fresh container
# bootstrap silently, the user can pre-stage the cached token JSON via the
# env var $GITHUB_MCP_TOKEN_JSON (registered as a CCoW Setup-script secret).
# The auth-worker refresh token in that JSON is long-lived (~30 days), so the
# user just rotates the secret once a month, not once per session.
TOKEN_FILE="$HOME/.config/github-mcp-server-rs/token-${ENV_NAME}.json"
if [ ! -f "$TOKEN_FILE" ] && [ -n "${GITHUB_MCP_TOKEN_JSON:-}" ]; then
  echo "[install-mcp] hydrating $TOKEN_FILE from \$GITHUB_MCP_TOKEN_JSON" >&2
  mkdir -p "$(dirname "$TOKEN_FILE")"
  printf '%s' "$GITHUB_MCP_TOKEN_JSON" > "$TOKEN_FILE"
  chmod 600 "$TOKEN_FILE"
fi
if [ ! -f "$TOKEN_FILE" ]; then
  if [ "${GITHUB_MCP_AUTO_DEVICE_FLOW:-}" = "1" ]; then
    # ─── Legacy RFC 8628 device-code path (opt-in) ─────────────────────────
    # Kept for CLI / local dev / offline where a sticky browser cookie
    # session against `auth(-staging).ippoan.org` is impractical. CCoW
    # containers should prefer the 1-click pair path below.
    echo "" >&2
    echo "[install-mcp] ───── device authorization required (env=$ENV_NAME) ─────" >&2
    echo "[install-mcp] (\$GITHUB_MCP_AUTO_DEVICE_FLOW=1 opt-in path)" >&2
    echo "[install-mcp] OPEN the verification_uri_complete URL printed below in a" >&2
    echo "[install-mcp] browser, sign in with GitHub, and Approve.  The hook will" >&2
    echo "[install-mcp] block until polling completes." >&2
    echo "[install-mcp]" >&2
    echo "[install-mcp] Tip: to skip this prompt on future fresh containers, copy" >&2
    echo "[install-mcp]   $TOKEN_FILE" >&2
    echo "[install-mcp] into a CCoW Setup-script secret named GITHUB_MCP_TOKEN_JSON." >&2
    echo "" >&2
    "$BIN" auth --env "$ENV_NAME" >&2
  else
    # ─── 1-click pair flow (default, issue #42) ────────────────────────────
    # The binary's `pair` subcommand is self-contained: it POSTs to
    # /mcp/pair/new, prints the pair_url to stdout, polls the WS upgrade
    # with `Pair-Status: pending` retries, and on 101 enters the frame
    # bridge loop. We launch it in the background, capture the pair_url
    # from its log, then surface it to the user. Step 4 (`relay` launch)
    # is skipped because `pair` already runs the bridge inline.
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
      echo "[install-mcp] Alternatively, set \$GITHUB_MCP_AUTO_DEVICE_FLOW=1 to fall" >&2
      echo "[install-mcp] back to the legacy RFC 8628 device-code prompt." >&2
      exit 1
    fi

    # Reap any leftover pair / relay process from a previous container session
    # before respawning (`relay.pid` is from the legacy step 4 path that we
    # skip entirely in pair mode, but a v0.0.x binary may have written it).
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

    # Wait up to 5s for the binary to surface a pair_url line; the POST is
    # quick (<300 ms on staging) so 5s is generous.
    #
    # The regex requires {30,60} characters after `/mcp/pair/` to mirror the
    # server-side `PAIR_CODE_REGEX = /^[A-Za-z0-9_-]{30,60}$/` and skip the
    # bare `/mcp/pair/new` POST URL that the binary logs to stderr before the
    # actual pair_url lands on stdout (smoke test 2026-05-18 bug).
    LINK=""
    for _ in $(seq 1 5); do
      if grep -qE 'https?://[^[:space:]]+/mcp/pair/[A-Za-z0-9_-]{30,60}([[:space:]]|$)' "$STATE_DIR/pair.log" 2>/dev/null; then
        LINK="$(grep -oE 'https?://[^[:space:]]+/mcp/pair/[A-Za-z0-9_-]{30,60}' "$STATE_DIR/pair.log" \
                | head -1 || true)"
        [ -n "$LINK" ] && break
      fi
      # If the binary died, surface the failure immediately instead of looping.
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
    echo "[install-mcp] into a CCoW Setup-script secret named \$GITHUB_MCP_TOKEN_JSON," >&2
    echo "[install-mcp] or set \$GITHUB_MCP_AUTO_DEVICE_FLOW=1 for the legacy CLI prompt." >&2
    echo "" >&2

    # `pair` self-contained: it surfaces $STATE_DIR/url after WS handshake.
    # Step 4 (legacy `relay` launch) is intentionally skipped.
    PAIR_MODE=1
  fi
fi

# ─── 4. (re)start relay in the background ─────────────────────────────────────
# Skipped when the 1-click pair path took over (step 3 above): the `pair`
# subcommand is self-contained — it both surfaces the pair_url AND runs the
# WS frame bridge loop, so launching a second `relay` process would race on
# `<state-dir>/url` and (worse) try a `relay`-mode handshake that requires a
# device-flow token cache we do not have.
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
# In `relay` (device-flow) mode the binary writes the public URL to
# `<state-dir>/url` only after it has done /mcp/introspect + WS handshake.
# We poll for that file with a 30s budget.
#
# In `pair` mode the URL is purely a function of (env, github_login), so we
# compute it up front instead — the user clicks the pair link asynchronously,
# possibly long after this hook exits, and the bridge comes up at that point.
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
  echo "export GITHUB_MCP_URL=\"$MCP_URL\"" >> "$CLAUDE_ENV_FILE"
fi

if [ "${PAIR_MODE:-0}" = "1" ]; then
  cat >&2 <<EOF

[install-mcp] ✓ github-mcp-server-rs is ready (pair mode, waiting on browser click).
[install-mcp]   MCP URL (Streamable HTTP via auth-worker WS relay): $MCP_URL
[install-mcp]   This URL is **stable** — register it once in Claude Code Web's MCP
[install-mcp]   settings; the bridge comes up the moment you click the pair link above.
[install-mcp]   Also exported as \$GITHUB_MCP_URL and written to:
[install-mcp]     $STATE_DIR/mcp-url
[install-mcp]   Pair log: $STATE_DIR/pair.log
EOF
else
  cat >&2 <<EOF

[install-mcp] ✓ github-mcp-server-rs is ready (relay mode).
[install-mcp]   MCP URL (Streamable HTTP via auth-worker WS relay): $MCP_URL
[install-mcp]   This URL is **stable** — register it once in Claude Code Web's MCP
[install-mcp]   settings, no need to update per-session.
[install-mcp]   Also exported as \$GITHUB_MCP_URL and written to:
[install-mcp]     $STATE_DIR/mcp-url
[install-mcp]   Relay log: $STATE_DIR/relay.log
EOF
fi
