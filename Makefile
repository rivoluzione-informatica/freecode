# FreeCode dev convenience. The daemon runs as a launchd service (scripts/*.plist) pointing at the
# RELEASE binary; the VSCode extension loads from its built bundle (vscode-plugin/dist).

DAEMON_LABEL := org.freecoders.freecode-daemon
UID := $(shell id -u)

.PHONY: daemon plugin restart test

## Rebuild the daemon (release) and restart the launchd service so the new build is live.
daemon:
	cargo build --release -p freecode-daemon
	launchctl kickstart -k gui/$(UID)/$(DAEMON_LABEL)
	@echo "✓ daemon rebuilt (release) + restarted on :50051"

## Rebuild the VSCode extension bundle (then run 'Developer: Reload Window' in VSCode to load it).
plugin:
	cd vscode-plugin && npm run typecheck && node esbuild.js
	@echo "✓ plugin dist rebuilt — run 'Developer: Reload Window' in VSCode to load it"

## Restart the daemon without rebuilding.
restart:
	launchctl kickstart -k gui/$(UID)/$(DAEMON_LABEL)

## Run the workspace tests.
test:
	cargo test --workspace
