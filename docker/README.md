# FreeCode `run` sandbox (RFC-002)

The `run` tool lets the agent execute commands (tests, linters, search). It is **OFF by default**
and gate-first: a deterministic policy (`daemon/src/run_policy.rs`) classifies every command
**Allow / Approve / Deny**, and execution is mode-gated. This directory is the **container boundary**
that must exist before any *auto*-mode execution.

## Safety model (recap)

- **OFF unless globally enabled** — config is read ONLY from `~/.freecode/config.json` (never a
  per-repo `.freecode/config.json`), so a cloned project can't switch `run` on.
- **Default-deny policy** — `Deny` (rm -rf, sudo, pipe-to-shell, curl/wget/ssh, package installs,
  git push, secret reads, …) never runs; `Approve` needs a human; only `Allow` (read-only/test) runs.
- **Suggest (HITL):** `Allow` runs (on the host, or in the container if enabled).
- **Auto:** `Allow` runs **only inside this container** — never on the host. No container → no
  auto-exec (fail-closed).
- Each command: no shell (simple invocation only), `cwd = workspace`, 60s timeout + kill-on-drop,
  output log-compressed.

## Build the sandbox image

```bash
docker build -t freecode-sandbox -f docker/freecode-sandbox.Dockerfile .
```

## Enable (your explicit call — this is a security decision)

Edit the **global** config `~/.freecode/config.json`:

```jsonc
{
  "enable_run": true,          // master switch for the run tool
  "run_in_container": true,    // route every command through the ephemeral, no-network container
  "run_container_image": "freecode-sandbox"
}
```

Then restart the daemon so it re-reads the config:

```bash
launchctl kickstart -k gui/$(id -u)/org.freecoders.freecode-daemon
```

- `enable_run: true` + Suggest (HITL) → the agent can run `Allow` commands (on the host unless
  `run_in_container` is also on).
- `enable_run: true` + `run_in_container: true` → `Allow` commands run inside the container in **both**
  Suggest and **Auto**. This is the only way `auto` ever executes a command.
- `docker` must be on the daemon's PATH; if the image is missing or docker isn't found, execution
  fails closed (the agent gets an error, nothing runs on the host).

## How FreeCode invokes it

```bash
docker run --rm --network none -v <workspace>:<workspace> -w <workspace> freecode-sandbox <cmd...>
```

Ephemeral (`--rm`), no network egress (`--network none`), only the workspace mounted. The container
is defense-in-depth on top of the policy + mode gates — not a replacement for them.
