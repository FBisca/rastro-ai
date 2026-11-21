# Writing a connector plugin

A connector reads one kind of source — a mailbox, a tracker, a chat — and submits what it
finds as evidence. You can write it in any language that can read lines from stdin and write
lines to stdout. This guide walks the protocol once, then lists the traps a TypeScript author
hits first. The machine-readable contract is `schema/plugin-protocol-v1.schema.json` at the
repository root, generated from the protocol library; the reasoning behind it lives in
[ADR 0003](../adr/0003-json-rpc-stdio-plugin-protocol.md) and
[ADR 0004](../adr/0004-plugin-manifest-and-grants.md).

Nothing about Rastro reaches the source you read: submit only the content you read there.
Justification, ids and links back to Rastro stay inside Rastro.

## Install

A plugin is a directory holding `rastro-plugin.toml` and everything it runs. Put it under
`<config>/plugins/` (with the default layout, `~/.config/rastro/plugins/`), or point
`RASTRO_PLUGIN_PATH` at a directory of plugins while you develop.

```toml
id = "work"
name = "Work mailbox"
version = "0.1.0"
author = "You"
kinds = ["connector"]
protocol = 1
command = "run"
triggers = []
dependencies = []

[permissions]
submit = ["message"]
scopes = ["work"]
env = ["WORK_TOKEN"]
actions = []

[[source_options]]
name = "mailbox"
required = true
help = "The mailbox address to read"
```

- `command` is one executable, resolved relative to the plugin directory. It takes no
  arguments in the manifest; wrap a runtime invocation in a script (below).
- `permissions.submit` lists the evidence kinds you may send; `scopes` the scopes a source
  may bind you to. A connector holds no actions.
- `source_options` declares what `rastro source add --option key=value` accepts. An option
  you did not declare is refused.

## The entry point

The core starts your `command` with a cleared environment: only `PATH`, `HOME`, `TMPDIR`,
`LANG`, `LC_*` and the variables your manifest lists under `permissions.env` survive. For
TypeScript, make an executable script the entry point:

```sh
#!/usr/bin/env node
require("./connector.js");
```

`chmod +x run`. The shebang finds `node` through `PATH` — see the traps below.

## One run, message by message

The core spawns you and speaks JSON-RPC 2.0: one JSON object per line on stdout, requests
from the core on stdin. Logs go to stderr, never stdout.

**1. `initialize` (core → you).** The core names the protocol, your manifest id, and the run
binding:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
  "protocol":"1.0","manifest_id":"work",
  "run":{"run_id":42,"source":"work-mail","scope":"work",
         "selector":{"mailbox":"me@example.com"},
         "window":{"from":"2026-11-20T09:00:00.000Z","to":"2026-11-21T09:00:00.000Z"},
         "keep_original":false}}}
```

Answer with the protocol and manifest id you speak; the major version and the id must match:

```json
{"jsonrpc":"2.0","id":1,"result":{"protocol":"1.0","manifest_id":"work"}}
```

The binding is your whole world for this run: read the `selector`, read only inside
`window`, and submit evidence for that source and scope. You never read Rastro's store.

**2. `evidence.submit` (you → core, notification).** One observation of one field at one
location, with the exact quote and its half-open UTF-8 byte span:

```json
{"jsonrpc":"2.0","method":"evidence.submit","params":{
  "kind":"message","location":"mail:123","field":"/body",
  "start":0,"end":11,"quote":"hello world","trust_class":"untrusted content"}}
```

- `kind` must be one your manifest permits.
- `end - start` must equal the quote's length **in UTF-8 bytes**.
- `trust_class` is `structured`, `unstructured` or `untrusted content`.
- `source`, `scope` and `window` are inherited from the binding; omit them, or repeat the
  bound values exactly. Anything else is rejected.
- `source_updated_at` (RFC 3339) is optional but earns its place: it lets Rastro order
  updates. `original` is sent only when the binding says `keep_original`.

A rejected submission is counted and named on stderr; the run continues.

**3. `coverage.report` (you → core, notification).** Say whether you looked at everything
the window promises:

```json
{"jsonrpc":"2.0","method":"coverage.report","params":{"status":"complete"}}
```

`complete` means you read the whole selection. When you could not, report
`{"status":"couldnt_look","reason":"rate_limited"}` with a typed reason (`auth`, `forbidden`,
`not_found_or_hidden`, `rate_limited`, `network`, `timeout`, `protocol`, `interrupted`,
`storage`, `result_cap`, `not_installed`), optionally `retry_at`, and `gaps` naming the
selections you could not read. A run that finishes without a coverage report has looked at
nothing Rastro can vouch for.

**4. `run.finish` (you → core, request).** Your own view; the core answers with its counts,
which are the authority:

```json
{"jsonrpc":"2.0","id":101,"method":"run.finish","params":{"status":"ok","submitted":1}}
```

Read the reply, then exit. A process that exits before `run.finish` fails the run as
`interrupted`; one that lingers past the shutdown deadline is killed.

The host's bounds: a line over 1 MiB, a line that is not a well-formed JSON-RPC message, or
silence past 60 seconds ends the run. Keep every message on one line.

## Check it without storing anything

```sh
rastro plugin check work --option mailbox=me@example.com
```

This runs your connector once through the same validation a sync applies, and stores
nothing: no evidence, no coverage, no run. The synthetic run binds the last 24 hours, your
manifest's first declared scope, a throwaway source name and the options you pass. It prints
the accepted counts and exits zero when nothing is violated; otherwise it prints each
violation with its reason and exits 1 — an invalid stdout line, a kind you did not declare,
an exit without a finish. A manifest kind the core does not know earns a warning naming it,
not a failure. `--json` prints the same report as an envelope.

The check runs your code with the trust a sync gives it; Rastro does not sandbox plugins
([decisions/plugins.md](../decisions/plugins.md) §13). Check plugins you wrote or trust.

## TypeScript traps

- **`console.log` fails the run.** stdout is the protocol; a plain log line there is an
  invalid protocol line, named by the check and by a sync. Log with `console.error`.
- **`node` must be on the basic `PATH`.** The environment is cleared before you start:
  `NVM_DIR`, `NODE_OPTIONS`, version-manager shims and anything else you rely on are gone
  unless your manifest declares them in `permissions.env`. A shebang of
  `#!/usr/bin/env node` resolves `node` from the `PATH` the core passes through; a hardcoded
  interpreter path from your version manager does not survive another machine.
- **Secrets belong in `permissions.env`.** Declare the variable (`env = ["WORK_TOKEN"]`),
  read `process.env.WORK_TOKEN`, and let the owner set it in their own environment. Never
  write a token into the manifest, the code, or a log line.
- **Spans are UTF-8 bytes, not `string.length`.** JavaScript counts UTF-16 code units;
  `end - start` must be `Buffer.byteLength(quote, "utf8")`. Compute `start` as the byte
  offset of the quote inside the field, not the character offset.
- **One JSON object per line.** `JSON.stringify` without indentation is one line; pretty
  printing is several, and each extra line is a protocol violation.
- **Exit when you are done.** Open handles (timers, sockets, watchers) keep the process
  alive after `run.finish`; the core kills the process group past the shutdown deadline.
  Close what you opened, or call `process.exit(0)` after reading the finish reply.
