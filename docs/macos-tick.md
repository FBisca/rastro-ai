# macOS minute tick: T9 preparation

Owner-operated procedure; no scheduler is installed by this repository. Template: [`packaging/macos/com.rastro.tick.plist`](../packaging/macos/com.rastro.tick.plist). The repository had no tracked launchd or packaging template; this is the proposed packaging location. Linux is deferred to P3.

## What runs

`rastro tick` executes eligible unfinished action runs first, then syncs stale configured sources serially; fresh sources are skipped ([tick.rs](../crates/rastro-core/src/tick.rs#L56)). Accepted runs, failed runs under three attempts, and pending Acts past grace are eligible ([runs.rs](../crates/rastro-core/src/actions/runs.rs#L270)). A due pending Act is rechecked for its rule, cited claims and observed duplicate before execution; failure returns it to the queue ([runs.rs](../crates/rastro-core/src/actions/runs.rs#L321)). Matching rules start Acts when an action is proposed, not as a general scheduled sweep ([actions/mod.rs](../crates/rastro-core/src/actions/mod.rs#L320)). Sync invokes connectors and their bound extractors ([runs/mod.rs](../crates/rastro-core/src/runs/mod.rs#L174)); tick has no direct judgment call, and the meeting judgment adapter is called by the meeting path ([meeting.rs](../crates/rastro-core/src/meeting.rs#L498)).

The template uses `StartInterval=60`, `KeepAlive=false` for a finite command, and `RunAtLoad=false` to avoid an immediate live tick on bootstrap. The first interval can perform writes; bootstrap only after the owner is ready. The local `launchd.plist(5)` says an interval is missed while asleep or while the job is running; it does not promise a tick exactly each minute or an immediate catch-up. A later tick handles eligible runs. `launchctl print` displays the last exit status, not a history. Logs contain tick output without timestamps; monitor their size and rotate them under an owner-chosen policy (TODO). Paths in the plist are absolute; launchd does not expand `~` or shell variables in them.

## Install or upgrade (owner only)

1. Build/install the intended binary and its required plugins using the project's normal process. TODO: confirm installed binary, plugin locations, old job label and old plist path; this repo has no installer or existing plist. Confirm the live `RASTRO_HOME` and log directory. Create the log directory with private permissions. Edit a copy of the template, replacing all four absolute-path placeholders with XML-escaped paths. Never put a token in the plist or logs.
2. **Credential gate:** the Linear connector can read `LINEAR_API_KEY` or keychain item `rastro-linear` ([connector API](../crates/rastro-connector-linear/src/api.rs#L119)); the action provider requires `LINEAR_API_KEY` ([provider](../crates/rastro-action-linear/src/linear.rs#L94)), passed from the host's allowed environment ([host/actions.rs](../crates/rastro-core/src/host/actions.rs#L139), [manifest](../crates/rastro-action-linear/rastro-plugin.toml#L14)). TODO: owner establish and verify a secure launchd environment delivery method for the writer; a shell export alone is not evidence the job receives it. Do not run the live Act until confirmed. `rastro check <source>` makes one cheap authenticated call through the source's connector and reports the credential's name (never its value) and whether the provider accepted it; it writes nothing and exits 1 on a failure. Run it with the same environment the job gets; `rastro source add` prints the same line after saving the source.
3. Validate the edited plist with `plutil -lint /path/to/edited.plist`. Inspect any existing job with `launchctl print gui/$(id -u)/OLD_LABEL`. On upgrade, identify and boot out the actual old job first to avoid two schedules; wait for any in-flight tick to finish and inspect its outcome before replacing the binary or job. `bootout` may terminate a running job: choose a quiet moment and inspect status first.
4. Copy the edited plist to `~/Library/LaunchAgents/com.rastro.tick.plist`, then, when ready for scheduled work:

   ```sh
   launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/com.rastro.tick.plist"
   launchctl print "gui/$(id -u)/com.rastro.tick"
   ```

   If an older job was disabled, explicitly enable the intended label before bootstrap with `launchctl enable "gui/$(id -u)/com.rastro.tick"`. Do not enable or remove an unidentified job.

## Smoke and failure visibility

- Observe two new summary lines in the configured stdout log across successive intervals while awake; check file modification times and `launchctl print "gui/$(id -u)/com.rastro.tick"` for runs and last exit status. A summary without timestamps alone cannot establish cadence. `rastro coverage` against the same `RASTRO_HOME` shows freshness; a fresh source is deliberately skipped, so its check time need not advance every minute ([tick.rs](../crates/rastro-core/src/tick.rs#L76)). Inspect coverage and source freshness rules together. No configured sources gives zero syncs and proves no connector work.
- A tick prints `Finished N action run(s), N failed; synced N source(s), N already fresh, N failed`. Exit 1 accompanies failed runs or syncs, with `error: action run ... failed: ...` or `error: source ... failed to sync: ...` on stderr; setup failures also print `error:` and exit 1. Another tick holding the lock prints `A tick is already running; nothing to do` and exits 0 ([CLI tick](../crates/rastro-cli/src/tick.rs#L9)). Failed runs at three attempts are no longer eligible and a later tick may exit 0; inspect `rastro actions` and `rastro actions show <run>` as well ([runs.rs](../crates/rastro-core/src/actions/runs.rs#L270)). Logs and last exit status are not an alerting system; TODO choose monitoring and rotation.
- Check the live rule with `RASTRO_HOME=/absolute/path rastro rules check`, and review `rastro queue` and `rastro actions show <run>` using the same environment. A manual tick is live, not a dry run; do not use it as a harmless probe ([tick.rs](../crates/rastro-core/src/tick.rs#L56)).

## Stop and uninstall (owner only)

Stopping the schedule alone does not cancel pending Acts: a later manual or scheduled tick can run them. Before grace ends, use `rastro actions stop <run>` for each pending Act the owner decides to cancel; check the result. Stop only accepts pending runs; a done run has a separate undo ([stop implementation](../crates/rastro-core/src/actions/stop.rs#L22)). Inspect the job for a running tick, choose a quiet moment, then:

```sh
launchctl bootout "gui/$(id -u)/com.rastro.tick"
launchctl print "gui/$(id -u)/com.rastro.tick"  # should report no such service
```

For uninstall, after successful bootout remove `~/Library/LaunchAgents/com.rastro.tick.plist`; preserve logs and data until the owner decides otherwise. For upgrade, replace the edited plist and bootstrap again. Investigate errors rather than assuming bootout succeeded.

## T9 checklist

- [ ] **Owner:** confirm T8 and installed build/plugins, live home, old job and safe credential delivery; install/upgrade the minute job and verify logs and coverage.
- [ ] **Owner:** write the owner-team rule in `config.toml`; inspect `rastro rules check`, including match, grace and cap.
- [ ] **Owner:** choose the next real Linear issue and have an in-session agent propose it; inspect its pending Act and due time. The owner decides whether to let grace expire or stop it; any ordinary proposal requiring acceptance remains an owner decision.
- [ ] **Owner:** let the chosen Act pass grace and verify the resulting Linear issue and `rastro actions show <run>` explanation. Investigate failed or returned runs instead of treating a tick summary as proof.
- [ ] **Owner:** record the switch day in `docs/specs/actions.md`, `docs/roadmap.md` and the README roadmap diagram, as T9 requires ([plan](plans/2026-11-19-act-by-rule/plan.yaml#L391)). `docs/specs/actions.md` does not exist yet by design: the spec gate lets an open plan hold its requirement titles ([workspace/specs.md](specs/workspace/specs.md)), so closing the act-by-rule plan at T9 archives its requirements into that new spec file.

## Isolated verification at 1c64bd8

`cargo build -p rastro-cli` succeeded. With a new empty temporary `RASTRO_HOME`, `target/debug/rastro tick` exited 0 and printed `Finished 0 action run(s), 0 failed; synced 0 source(s), 0 already fresh, 0 failed`. A separate temporary home with malformed `config/config.toml` exited 1 and printed `error: invalid config.toml` with a TOML parse error. These checks do not prove live sync, credentials, judgment, action execution, launchd cadence, or the grace-window switch. `plutil -lint packaging/macos/com.rastro.tick.plist` passed; the template was not loaded.
