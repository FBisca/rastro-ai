# Evidence
Paths: crates/rastro-core/, crates/rastro-storage-sqlite/, crates/rastro-cli/, crates/rastro-plugin-sdk/, crates/rastro-test-connector/, crates/rastro-connector-github/, schema/

## Requirements

### Requirement: Adding a source saves it and listing shows it
`rastro source add` SHALL save a source with its connector, selector, scope, freshness rule and window start, and `rastro source list` SHALL show each saved source with those fields.

#### Scenario: the owner adds a project source
- WHEN the owner runs `rastro source add project --connector github --repo o/p --scope personal`
- THEN `rastro source list` shows `project` with its connector, selector, scope, freshness rule and window start, and the config file holds it

### Requirement: A source that never synced reports never looked
A source that has never synced SHALL report "never looked".

#### Scenario: coverage of a source registered and never synced
- WHEN a source is registered but has never synced
- THEN `rastro coverage project` reports "never looked"

### Requirement: Data and config live where the environment says
Data and config SHALL live under `RASTRO_HOME` when it is set, otherwise under `XDG_DATA_HOME` and `XDG_CONFIG_HOME`, and failing those under `~/.local/share/rastro` and `~/.config/rastro`.

#### Scenario: the Rastro variable names the location
- WHEN `RASTRO_HOME` is set
- THEN data and config live under it

#### Scenario: the XDG variables name the location
- WHEN `RASTRO_HOME` is unset and `XDG_DATA_HOME` and `XDG_CONFIG_HOME` are set
- THEN data and config live under them

#### Scenario: nothing names the location
- WHEN neither `RASTRO_HOME` nor the XDG variables are set
- THEN data and config live under `~/.local/share/rastro` and `~/.config/rastro`

### Requirement: Stored evidence is never updated or deleted
The store SHALL reject any update or delete of stored evidence and leave the evidence unchanged.

#### Scenario: an update or a delete is refused
- WHEN anything tries to update or delete stored evidence
- THEN the store rejects it and the evidence is unchanged

### Requirement: Every migration applies once in timestamp order
Migrations written on different branches SHALL each apply exactly once, in timestamp order, on new and existing databases, and a database holding an unknown newer migration SHALL be refused.

#### Scenario: migrations merged in any order
- WHEN migrations written on different branches are merged in any order
- THEN each applies exactly once, in timestamp order, on new and existing databases

#### Scenario: an unknown newer migration is refused
- WHEN a database holds an unknown migration newer than the build's own
- THEN the store refuses it

### Requirement: A complete sync stores evidence and reports found
A sync whose connector submits evidence and a complete coverage report SHALL store the evidence, record coverage "found" for the window, and log the run `ok` with the plugin version and the core's counts.

#### Scenario: a complete sync that finds evidence
- WHEN `rastro sync project` runs a connector that submits evidence and a complete coverage report
- THEN the evidence is stored, coverage records "found" for the window, and the run log shows `ok` with the plugin version and the core's counts

### Requirement: A complete sync that finds nothing records empty
A complete sync that submits no evidence SHALL record coverage "empty" for the window.

#### Scenario: a quiet complete sync
- WHEN a complete sync submits no evidence
- THEN coverage records "empty" for the window

### Requirement: Incomplete evidence is rejected and counted
The core SHALL reject evidence without a location, span, quote or trust class, or whose quote's UTF-8 byte length differs from `end - start`, and SHALL count it as rejected.

#### Scenario: a submission missing a required part
- WHEN a connector submits evidence without a location, span, quote or trust class, or whose quote's UTF-8 byte length differs from `end - start`
- THEN the core rejects it and counts it as rejected

### Requirement: Evidence outside the source's grant is rejected
The core SHALL reject evidence of a kind the connector's grant doesn't permit, or that sets a source or scope other than the bound one, and SHALL count it as rejected.

#### Scenario: a forbidden kind or a forged source
- WHEN a connector submits a kind its grant doesn't permit, or sets a source or scope other than the bound one
- THEN the core rejects it and counts it as rejected

### Requirement: A source or window outside the manifest is refused
Sync SHALL refuse to run a source whose scope is outside the scopes its connector's manifest declares, and record "couldn't look"; the core SHALL reject a coverage report or evidence that names a window other than the bound one.

#### Scenario: a scope the manifest doesn't declare
- WHEN a source's scope is outside the scopes its connector's manifest declares
- THEN sync refuses to run it and records "couldn't look"

#### Scenario: a window other than the bound one
- WHEN a connector's coverage report or evidence names a window other than the bound one
- THEN the core rejects it

### Requirement: A connector sees only its declared environment
A connector SHALL see only the allowed environment variables and those its manifest declares, never the owner's other secrets.

#### Scenario: the owner's secrets are absent
- WHEN a connector runs
- THEN it sees only the allowed environment variables and those its manifest declares, never the owner's other secrets

### Requirement: Connector text is escaped on the terminal
Connector-supplied text that reaches the terminal SHALL have its control characters escaped.

#### Scenario: a rejection reason or a search result reaches the terminal
- WHEN connector-supplied text reaches the terminal (rejection reasons, search results)
- THEN control characters are escaped

### Requirement: A mismatched handshake fails as protocol
A run SHALL fail with `protocol`, before any evidence is ingested, when the connector answers `initialize` with an unsupported major version or a different manifest id.

#### Scenario: the connector answers with another major or manifest
- WHEN the connector answers `initialize` with an unsupported major version or a different manifest id
- THEN the run fails with `protocol` before any evidence is ingested

### Requirement: A manifest without permissions env still loads
A manifest that omits `permissions.env` SHALL be treated as declaring an empty list, and the plugin SHALL still load.

#### Scenario: no environment is declared
- WHEN a manifest omits `permissions.env`
- THEN it is treated as an empty list and the plugin still loads

### Requirement: A connector that exits early is interrupted
A run whose connector exits before `run.finish` SHALL fail with `interrupted`, record coverage "couldn't look", keep the evidence already accepted, and make `rastro sync` exit non-zero.

#### Scenario: the connector dies mid-run
- WHEN the connector exits before `run.finish`
- THEN the run fails with `interrupted`, coverage records "couldn't look", the evidence already accepted stays, and `rastro sync` exits non-zero

### Requirement: A malformed, oversized or silent connector is killed
The core SHALL kill a connector that sends a malformed or oversized line or is silent past the timeout, SHALL fail the run with `protocol` or `timeout`, and SHALL ingest no partial message.

#### Scenario: a bad line or a silence
- WHEN the connector sends a malformed or oversized line, or is silent past the timeout
- THEN the core kills it, the run fails with `protocol` or `timeout`, and no partial message is ingested

### Requirement: Connector stderr is relayed escaped and bounded
The core SHALL relay each stderr line a connector writes with control characters escaped and its length bounded, without ever blocking the connector.

#### Scenario: the connector writes to stderr
- WHEN a connector writes to stderr
- THEN the core relays each line with control characters escaped and length bounded, without ever blocking the connector

### Requirement: Fast stdout waits in bounded memory
When a connector writes to stdout faster than the core reads, the core SHALL hold a bounded number of lines in memory and the connector SHALL wait, the run still completing.

#### Scenario: stdout arrives faster than it is read
- WHEN a connector writes to stdout faster than the core reads
- THEN the core holds a bounded number of lines in memory and the connector waits (backpressure); the run still completes

### Requirement: A connector that stops reading its stdin times out
The core's write SHALL give up at its deadline, kill the connector and fail the run with `timeout` when the connector stops reading its stdin.

#### Scenario: the connector stops reading
- WHEN a connector stops reading its stdin
- THEN the core's write gives up at its deadline, kills the connector, and the run fails with `timeout`

### Requirement: A connector that lingers after finish is killed
The core SHALL kill a connector that sends `run.finish` but doesn't exit, or whose child keeps its pipes open, after a shutdown deadline, and SHALL keep the finished run and release the sync lock.

#### Scenario: the connector or its child holds the pipes
- WHEN a connector sends `run.finish` but doesn't exit, or a child of it keeps its pipes open
- THEN the core kills it after a shutdown deadline, keeps the finished run, and releases the sync lock

### Requirement: Stderr over the bound is dropped and counted
The relay SHALL drop stderr beyond its bound and say how many lines it dropped, and neither the connector nor the run SHALL wait on it.

#### Scenario: more stderr than the relay holds
- WHEN a connector writes more stderr than the relay's bound, or the core's own stderr blocks
- THEN the relay drops the excess and says how many lines it dropped, and neither the connector nor the run waits on it

### Requirement: Invalid UTF-8 or a bad envelope fails as protocol
A run SHALL fail with `protocol` and SHALL ingest nothing from the line when a connector sends invalid UTF-8 or a message that isn't a JSON-RPC 2.0 envelope.

#### Scenario: the line isn't a JSON-RPC 2.0 message
- WHEN a connector sends invalid UTF-8, or a message that isn't a JSON-RPC 2.0 envelope (wrong `jsonrpc`, missing `method` or `id` where required)
- THEN the run fails with `protocol` and nothing from that line is ingested

### Requirement: Core diagnostics never make a run wait
Core diagnostics SHALL go through the same bounded, lossy relay when a connector causes many of them and the core's stderr isn't drained, and the run SHALL never wait on them.

#### Scenario: many rejected submissions or coverage reports
- WHEN a connector causes many core diagnostics (rejected submissions or coverage reports) and the core's stderr isn't drained
- THEN they go through the same bounded, lossy relay and the run never waits on them

### Requirement: An unwritable finish reply times out
A run SHALL fail with `timeout` when the core's reply to `run.finish` can't be written before its deadline.

#### Scenario: the reply doesn't fit its deadline
- WHEN the core's reply to `run.finish` can't be written before its deadline
- THEN the run fails with `timeout`

### Requirement: An id JSON-RPC forbids fails as protocol
A run SHALL fail with `protocol`, ingesting nothing from the line, when a message's `id` is an object, array or boolean, or mixes `method` with `result` or `error`.

#### Scenario: the id or the shape is not allowed
- WHEN a message has an `id` JSON-RPC doesn't allow (object, array, boolean), or mixes `method` with `result` or `error`
- THEN the run fails with `protocol` and nothing from that line is ingested

### Requirement: The process group is killed and its pipes released
When a connector or a child of it still holds its pipes at the end of a run, the host SHALL kill the connector's process group and release its reader threads and pipe descriptors within the shutdown deadline.

#### Scenario: a pipe holder outlives the run
- WHEN a connector, or a child of it, still holds its pipes when the run ends
- THEN the host kills the connector's process group and releases its reader threads and pipe descriptors within the shutdown deadline

### Requirement: The group is signalled before the leader is reaped
Once the connector has exited, the host SHALL signal its process group before reaping it, so the signal can never reach a process that reused its id.

#### Scenario: the leader has exited but its group lives
- WHEN the connector has exited
- THEN the host signals its process group before reaping it, so the signal can never reach a process that reused its id

### Requirement: A storage refusal reaches the relay
A submission storage rejects, including span offsets the store can't represent, which are refused before storage, SHALL put its diagnostic through the bounded relay while the core's stderr is blocked, and the run SHALL not wait on it.

#### Scenario: storage refuses while stderr is blocked
- WHEN storage rejects a submission (including span offsets the store can't represent, which are refused before storage) while the core's stderr is blocked
- THEN the diagnostic goes through the bounded relay and the run doesn't wait on it

### Requirement: One host process keeps printing bounded across runs
The threads and queued output for printing SHALL stay bounded across the syncs one host process runs while its stderr is blocked, not per run.

#### Scenario: many syncs with stderr blocked
- WHEN one host process runs many syncs while its stderr is blocked
- THEN the threads and queued output for printing stay bounded across runs, not per run

### Requirement: A run that never finished is marked interrupted
The core SHALL mark a never-finished run `interrupted` before starting its own sync, whether it finds one at start with no live sync or the next sync takes `sync.lock`, even when a reader blocked startup recovery.

#### Scenario: startup finds a run with no live sync
- WHEN the core starts and finds a run that never finished with no live sync
- THEN it marks that run `interrupted` before starting its own

#### Scenario: the next sync takes the lock after a blocked recovery
- WHEN the next sync takes `sync.lock` and a reader blocked startup recovery
- THEN it marks the never-finished run `interrupted` before starting its own

### Requirement: A recovered run records couldnt look for its window
Recovering a run that never finished SHALL record coverage "couldn't look: interrupted" for that run's source, scope and window, which are stored when the run starts.

#### Scenario: the recovered run's own window
- WHEN the core recovers a run that never finished
- THEN coverage records "couldn't look: interrupted" for that run's source, scope and window, which are stored when the run starts

### Requirement: A typed couldnt look reason is stored
Coverage SHALL store a typed "couldn't look" reason a connector reports, such as `rate_limited` with a retry time, and the window SHALL never be reported as empty or complete.

#### Scenario: the connector names why it couldn't look
- WHEN the connector reports "couldn't look" with a typed reason (e.g. `rate_limited`, with a retry time)
- THEN coverage stores the reason and the window is never reported as empty or complete

### Requirement: A finish without an accepted coverage report fails
A run SHALL fail with `protocol` and record coverage "couldn't look: protocol" when the connector sends `run.finish` without a coverage report the core accepted.

#### Scenario: no coverage report was accepted
- WHEN a connector sends `run.finish` without a coverage report the core accepted (none sent, or every one rejected)
- THEN the run fails with `protocol` and coverage records "couldn't look: protocol"

### Requirement: Only owner involved items become evidence
Syncing a fixture repo SHALL make exactly the issues and PRs the owner authored, is assigned to, was asked to review, or is @mentioned in evidence, together with their comments, and SHALL not make an item with none of those evidence.

#### Scenario: the fixture repository
- WHEN syncing a fixture repo
- THEN exactly the issues and PRs the owner authored, is assigned to, was asked to review, or is @mentioned in become evidence, together with their comments; items with none of those don't

### Requirement: Only comments updated in the window become evidence
Only the comments of an item in the window updated within `[from, to)` SHALL become evidence.

#### Scenario: a comment updated before or after the window
- WHEN an item in the window has comments updated before or after the window
- THEN only the comments updated within `[from, to)` become evidence

### Requirement: Search queries name the repo and the window
The search queries a sync sends SHALL name the bound repo and window.

#### Scenario: the fixture server records the queries
- WHEN syncing
- THEN the search queries the fixture server receives name the bound repo and window

### Requirement: Each field is its own observation
Recording GitHub evidence SHALL make each title, body and comment its own observation with its URL, field path, byte span, quote, capture time and source update time, as `untrusted content` for prose and `structured` for API fields.

#### Scenario: an item's title, body and comments
- WHEN GitHub evidence is recorded
- THEN each title, body and comment is its own observation with its URL, field path, byte span, quote, capture time and source update time; prose is `untrusted content`, API fields are `structured`

### Requirement: Labels and assignees quote their own path
Recording labels or assignees SHALL quote the actual value at its own field path (`/labels/N/name`, `/assignees/N/login`), never a list the connector assembled.

#### Scenario: a label and an assignee
- WHEN labels or assignees are recorded
- THEN each observation quotes the actual value at its own field path (`/labels/N/name`, `/assignees/N/login`), never a list the connector assembled

### Requirement: Spans are UTF-8 bytes and the quote equals them
Every span SHALL be in UTF-8 bytes and its quote SHALL equal the bytes at that span, even when the prose holds multi-byte Unicode.

#### Scenario: prose with multi-byte Unicode
- WHEN prose contains multi-byte Unicode
- THEN each span is in UTF-8 bytes and the quote equals the bytes at that span

### Requirement: A rate limit mid-run keeps the finished pages
When results span several pages and a mid-run response is a rate limit, the connector SHALL keep the evidence from completed pages and coverage SHALL record "couldn't look: rate_limited", not "found".

#### Scenario: the limit arrives after some pages
- WHEN results span several pages and a mid-run response is a rate limit
- THEN the evidence from completed pages is kept and coverage records "couldn't look: rate_limited", not "found"

### Requirement: A capped or incomplete search is never found
A search that reports more than 1000 results or `incomplete_results` SHALL record coverage "couldn't look: result_cap", never "found".

#### Scenario: the search is capped
- WHEN a search reports more than 1000 results or `incomplete_results`
- THEN coverage records "couldn't look: result_cap", never "found"

### Requirement: A 403 is rate limited only on a limit signal
A 403 carrying `x-ratelimit-remaining: 0` or a secondary-limit signal (`retry-after`) SHALL record coverage `rate_limited` with its retry time, and any other 403 SHALL record `forbidden`.

#### Scenario: the limit signals are present
- WHEN GitHub answers 403 with `x-ratelimit-remaining: 0` or a secondary-limit signal (`retry-after`)
- THEN coverage records `rate_limited` with its retry time

#### Scenario: neither signal is present
- WHEN GitHub answers 403 with neither signal
- THEN coverage records `forbidden`

### Requirement: Every page of an items comments is read
When an item's comments span several pages, the connector SHALL read every page.

#### Scenario: comments over several pages
- WHEN an item's comments span several pages
- THEN every page is read

### Requirement: Instruction-like content changes no request
The connector's requests SHALL stay the fixed GET endpoints whatever instruction-like or command-like text GitHub content holds, and the fixture server SHALL record every method and path.

#### Scenario: content that reads like an instruction
- WHEN GitHub content contains instruction-like or command-like text
- THEN the connector's requests are still only the fixed GET endpoints; the fixture server records every method and path

### Requirement: Links and redirects away from the API host are refused
The connector SHALL not follow a `Link` header or redirect away from the API host, SHALL never send the token there, and SHALL record coverage "couldn't look: protocol".

#### Scenario: a link points elsewhere
- WHEN a `Link` header or a redirect points away from the API host
- THEN the connector doesn't follow it, never sends the token there, and coverage records "couldn't look: protocol"

### Requirement: A missing or rejected token is auth and never stored
A missing or rejected GitHub token SHALL record coverage "couldn't look: auth", and no token SHALL appear in evidence, the run log or protocol messages.

#### Scenario: the token is missing or refused
- WHEN the GitHub token is missing or rejected
- THEN coverage records "couldn't look: auth" and no token appears in evidence, the run log or protocol messages

### Requirement: A sync that changes nothing writes no evidence
A sync of a source over unchanged content SHALL write no new evidence, SHALL link the run to the existing evidence, and SHALL report the unchanged count.

#### Scenario: nothing changed
- WHEN a source is synced again and nothing changed
- THEN no new evidence is written, the run is linked to the existing evidence, and the unchanged count says so

### Requirement: A body that changes and returns keeps three observations
A body changing A→B→A across three syncs SHALL leave three observations, of which `search` shows only the latest and `search --history` shows all three.

#### Scenario: the body returns to its first text
- WHEN a body changes A→B→A across three syncs
- THEN there are three observations; `search` shows only the latest, and `search --history` shows all three

### Requirement: A search hit is cited with a bounded excerpt
Each search hit SHALL carry its source, scope, location, field, span start and end, capture time, trust class, coverage with freshness, and a `match` excerpt instead of `quote` or `original`; the excerpt SHALL centre on the first match, including stemming and diacritic folding, be at most 400 bytes, cut at UTF-8 character boundaries, use offsets in the span's origin, and set `clipped_before` and `clipped_after`; an unlocatable match SHALL start at the field's start with `located: false`; and search SHALL return at most `limit` hits (default 20, range 1–50) in bm25-then-id order, with `truncated: true` only when more matches exist.

#### Scenario: what a hit carries
- WHEN a search matches
- THEN each hit carries source, scope, location, field, span start and end, capture time, trust class, coverage with freshness, and a `match` excerpt instead of `quote` or `original`

#### Scenario: the excerpt is bounded and centred
- WHEN the match sits deep in the field, or a stem or a diacritic-folded form matches
- THEN the excerpt centres on the first match (including stemming and diacritic folding), is at most 400 bytes, cuts at UTF-8 character boundaries, uses offsets in the span's origin, and sets `clipped_before` and `clipped_after`

#### Scenario: the match cannot be located
- WHEN the match cannot be mapped back to the field
- THEN the excerpt starts at the field's start with `located: false`

#### Scenario: the limit bounds the hits and the order is bm25 then id
- WHEN a search finds more matches than `limit`
- THEN it returns at most `limit` hits (default 20, range 1–50) in bm25-then-id order, with `truncated: true` only when more matches exist

#### Scenario: a limit outside the range
- WHEN a limit is outside 1–50 or is not a whole number
- THEN the CLI exits 2 with the usage error envelope and MCP answers a JSON-RPC invalid-params error

### Requirement: A search that matches nothing returns no hits and the coverage
A search that matches nothing SHALL return no hits, `truncated: false`, and the coverage of each source searched, and human output SHALL say there is no match.

#### Scenario: nothing matches
- WHEN a search matches nothing
- THEN it returns no hits, `truncated: false`, and the coverage of each source searched; human output says no match

### Requirement: Stale coverage reads unknown with its last check
`rastro coverage` SHALL answer "unknown" when a source's last complete check is older than its freshness rule or its latest attempt failed, and SHALL still show the last successful check and the failure reason.

#### Scenario: the last check is too old or the last attempt failed
- WHEN a source's last complete check is older than its freshness rule, or its latest attempt failed
- THEN `rastro coverage` answers "unknown", and still shows the last successful check and the failure reason

### Requirement: A rebuilt index returns the same results
After the full-text index is dropped and `rastro index rebuild` runs, the same search SHALL return the same results with their citations intact.

#### Scenario: the index is rebuilt from scratch
- WHEN the full-text index is dropped and `rastro index rebuild` runs
- THEN the same search returns the same results with their citations intact

### Requirement: A rebuilt index behaves like a freshly migrated one
A rebuilt index SHALL behave like a freshly migrated one: the same stemming and diacritics matches, and the same latest and history results.

#### Scenario: search after a rebuild
- WHEN the index is rebuilt
- THEN it behaves like a freshly migrated one: the same stemming and diacritics matches, and the same latest and history results

### Requirement: Every json answer validates against its operation schema
A command run with `--json` SHALL print output that validates against the versioned operation schema.

#### Scenario: a command under json
- WHEN any command runs with `--json`
- THEN its output validates against the versioned operation schema

### Requirement: Arguments that do not parse give a schema valid error
A known command run with `--json` whose arguments don't parse SHALL print a schema-valid error result for that operation and exit non-zero, while `--help` and `--version` still work.

#### Scenario: a known command with arguments that don't parse
- WHEN a known command runs with `--json` but its arguments don't parse
- THEN it prints a schema-valid error result for that operation and exits non-zero; `--help` and `--version` still work

### Requirement: A non UTF-8 argument never panics
A command given an argument that isn't valid UTF-8 SHALL never panic: for a known operation under `--json` it SHALL print that operation's schema-valid error result, and either way it SHALL exit 2.

#### Scenario: an argument is not valid UTF-8
- WHEN a command gets an argument that isn't valid UTF-8
- THEN it never panics: for a known operation under `--json` it prints that operation's schema-valid error result, and either way it exits 2

### Requirement: The owners real repository syncs and a remembered phrase is found
Registering one of the owner's real repos and syncing SHALL make a phrase the owner remembers findable with a working link, and `rastro coverage` SHALL be accurate.

#### Scenario: the owner's own repo
- WHEN the owner registers one of their real repos and syncs
- THEN searching for a phrase they remember returns it with a working link, and `rastro coverage` is accurate

### Requirement: A last check on a whole second is one instant
When `rastro coverage` reports a last-checked time that falls on a whole second, its human output and `--json` SHALL name the same instant.

#### Scenario: the last check lands on a whole second
- WHEN `rastro coverage` reports a last-checked time that falls on a whole second
- THEN its human output and `--json` name the same instant

### Requirement: A half-configured live check fails naming what is missing
The live check SHALL fail naming what is missing when it runs with only one of its variables set, or one of them empty, and SHALL be skipped with neither set.

#### Scenario: only one live variable is set
- WHEN the live check runs under `--ignored` with only one of its variables set, or one empty
- THEN it fails naming what is missing

#### Scenario: neither live variable is set
- WHEN the live check runs with neither variable set
- THEN it is skipped

### Requirement: A kept original is stored beside its quote
A source added with `--keep-original` SHALL store the full original beside each new observation's quoted span, and a source without the flag SHALL store none; search SHALL return the whole quote and, when retained, the original only with `full: true` (`--full` on the CLI), and human `--full` SHALL show the whole quote with control characters escaped and no 500-character clip.

#### Scenario: a source that keeps originals syncs
- WHEN a source added with `--keep-original` syncs
- THEN each new observation stores the full original beside its quoted span; a source without the flag stores none

#### Scenario: a search asks for the full text
- WHEN search runs with `full: true` (`--full` on the CLI) for a source that keeps originals
- THEN it returns the whole quote and the original, and human `--full` shows the whole quote with control characters escaped and no 500-character clip

### Requirement: A non-object original is rejected
The core SHALL reject a submission whose original isn't a JSON object for a source that keeps originals, SHALL store no original, and `rastro search --json` SHALL stay schema-valid.

#### Scenario: the original is not an object
- WHEN a connector sends an original that isn't a JSON object for a source that keeps originals
- THEN the core rejects that submission, stores no original, and `rastro search --json` stays schema-valid

### Requirement: A null original is rejected and an omission accepted
The core SHALL reject a submission with `"original": null` for a source that keeps originals like any other non-object original, and SHALL accept a submission that omits the original.

#### Scenario: the original is explicitly null
- WHEN a connector sends `"original": null` for a source that keeps originals
- THEN the core rejects that submission like any other non-object original; an omitted original is still accepted

### Requirement: Coverage lists each source's newest windows
Coverage SHALL return each source's overall state judged over all its windows, its 10 newest windows (newest first) and `windows_total`, `all_windows: true` (`--all-windows`) SHALL return every window, and each window `[from, to)` SHALL carry its state, selection and, for "couldn't look", its retry time.

#### Scenario: the default answer
- WHEN coverage is requested
- THEN it returns each source's overall state judged over all its windows, its 10 newest windows (newest first), and `windows_total`

#### Scenario: every window is asked for
- WHEN coverage is requested with `all_windows: true` (`--all-windows`)
- THEN it returns every window, each `[from, to)` with its state, selection and, for "couldn't look", its retry time

### Requirement: A quiet window does not erase what an earlier one found
When a later sync over a quiet period finds nothing new, coverage and each search hit SHALL still answer "found" for a source whose only found window is older than the 10 newest, and the quiet window SHALL show "empty" in its own row.

#### Scenario: a quiet sync after a found one
- WHEN a later sync over a quiet period finds nothing new
- THEN coverage and each search hit still answer "found" for the source, even when its only found window is older than the 10 newest; the quiet window shows "empty" in its own row

### Requirement: A run whose submissions were all rejected is never empty
When every submission in a run is rejected, the window SHALL record "couldn't look: protocol", never "empty".

#### Scenario: every submission is rejected
- WHEN every submission in a run is rejected
- THEN the window records "couldn't look: protocol", never "empty"

### Requirement: Some rejected submissions make the window couldnt look
When some submissions in a run are rejected, the accepted evidence SHALL be stored and the window SHALL record "couldn't look: protocol", not "found".

#### Scenario: a run accepts some submissions and rejects some
- WHEN some submissions in a run are rejected
- THEN the accepted evidence is stored and the window records "couldn't look: protocol", not "found"

### Requirement: A rejected submission outranks a reported failure
When a run has a rejected submission and also a connector-reported failure or a failed finish, the window SHALL record "couldn't look: protocol" and the accepted evidence SHALL be kept.

#### Scenario: a rejection beside another failure
- WHEN a run has a rejected submission and also a connector-reported failure or a failed finish
- THEN the window records "couldn't look: protocol" and the accepted evidence is kept

### Requirement: A missing connector is couldnt look not installed
When a source's connector isn't installed, coverage SHALL record "couldn't look: not_installed", and `not_installed` SHALL be one of the published typed reasons.

#### Scenario: the connector isn't installed
- WHEN a source's connector isn't installed
- THEN coverage records "couldn't look: not_installed", and `not_installed` is one of the published typed reasons

### Requirement: A manifest that fails its check is one failed run
When a source's manifest fails its check, a failed run SHALL be recorded with "couldn't look: protocol" and any other source in the same sync SHALL still run.

#### Scenario: one source's manifest is broken
- WHEN a source's manifest fails its check
- THEN a failed run with "couldn't look: protocol" is recorded and any other source in the same sync still runs

### Requirement: A non-loopback API base URL sends no request
When the GitHub API base URL override names a host that isn't loopback, the connector SHALL send no request and coverage SHALL record "couldn't look: protocol".

#### Scenario: the override names another host
- WHEN the GitHub API base URL override names a host that isn't loopback
- THEN the connector sends no request and coverage records "couldn't look: protocol"

### Requirement: A release build ignores the test-only overrides
A release build SHALL ignore `RASTRO_NOW` and any `RASTRO_HOST_*` override set in its environment.

#### Scenario: a test-only override is set
- WHEN a release build runs with `RASTRO_NOW` or a `RASTRO_HOST_*` override set
- THEN it ignores them

### Requirement: Authors are structured evidence at each location
Reading a PR, issue or comment with an author SHALL submit `structured` evidence at `/user/id`, `/user/login` and `/user/type` at that item's location, and SHALL submit no author evidence when the author is null.

#### Scenario: an item has an author
- WHEN the GitHub connector reads a PR, issue or comment with an author
- THEN it submits `structured` evidence at `/user/id`, `/user/login` and `/user/type` at that item's location

#### Scenario: the author is null
- WHEN the author of a PR, issue or comment is null
- THEN it submits no author evidence

### Requirement: A reread starts at the registered window
`rastro sync <source> --reread` SHALL read from the registered window start, SHALL add no evidence for unchanged fields, SHALL append new fields, SHALL record coverage as for any sync, and SHALL change nothing for an item that isn't returned.

#### Scenario: the owner rereads a source
- WHEN `rastro sync <source> --reread` runs
- THEN the connector reads from the registered window start: unchanged fields add no evidence, new fields append, coverage is recorded as for any sync, and an item that isn't returned changes nothing

### Requirement: A sync result carries extraction only for a bound source
A sync's result SHALL carry an optional `extraction` object, holding the run id, extractor, version, state, reason, evidence count and claim count, for a source bound to an extractor, and SHALL omit it when no extractor is bound; the addition SHALL sit within v2 and SHALL be recorded in `docs/decisions/surfaces.md`.

#### Scenario: the source has an extractor
- WHEN a sync runs for a source bound to an extractor
- THEN its result carries an optional `extraction` object (run id, extractor, version, state, reason, evidence and claim counts)

#### Scenario: no extractor is bound
- WHEN a sync runs for a source with no extractor bound
- THEN the `extraction` object is absent

### Requirement: An undeclared or missing option is refused
`rastro source add --connector <id>` SHALL refuse an option its manifest doesn't declare, and a missing required option, with an error naming the declared options, and SHALL save nothing.

#### Scenario: an option the manifest doesn't declare
- WHEN `rastro source add --connector <id>` is given an option its manifest doesn't declare
- THEN it's refused with an error naming the declared options, and nothing is saved

#### Scenario: a required option is missing
- WHEN `rastro source add --connector <id>` omits a required option
- THEN it's refused with an error naming the declared options, and nothing is saved

### Requirement: The declared repo pattern checks the repo
`rastro source add --connector github --repo owner/name` SHALL save the source as before, and SHALL take its error from the GitHub manifest's declared option when the repo doesn't match that option's pattern.

#### Scenario: a repo that matches the pattern
- WHEN `rastro source add --connector github --repo owner/name` runs
- THEN the source is saved as before

#### Scenario: a repo that doesn't match the pattern
- WHEN the repo doesn't match the GitHub manifest's pattern
- THEN the error comes from that declared option

### Requirement: A single-source connector refuses a second source
`rastro source add --connector linear` SHALL be refused with an error naming the existing source while a Linear source exists, and SHALL save nothing.

#### Scenario: a Linear source already exists
- WHEN `rastro source add --connector linear` runs while a Linear source exists
- THEN it's refused with an error naming the existing source, and nothing is saved

### Requirement: A second source in the config fails its own sync
When a config already holds a second source for a single-source connector, that source's sync SHALL fail as `protocol` naming the other source before the connector runs, and the first source SHALL still sync.

#### Scenario: a hand-edited second source
- WHEN a config already holds a second source for a single-source connector
- THEN that source's sync fails as `protocol` naming the other source, before the connector runs, and the first source still syncs

### Requirement: The Slack check accepts only the read scopes
The Slack connector's check SHALL answer authenticated only for a token that holds every required read scope (`channels:read`, `channels:history`, `groups:read`, `groups:history`, `users:read`) and no other scope, and SHALL answer rejected naming the extra or missing scopes otherwise.

#### Scenario: a token with exactly the read scopes
- WHEN `rastro check` runs a Slack source whose token the scripted API reports with exactly the five read scopes
- THEN the check reports authenticated and names the credential source

#### Scenario: a token with a write scope
- WHEN the scripted API reports the token's scopes as the five read scopes plus `chat:write`
- THEN the check reports rejected naming `chat:write` and exits unsuccessfully

#### Scenario: a token missing a read scope
- WHEN the scripted API reports the token's scopes without `groups:history`
- THEN the check reports rejected naming `groups:history` and exits unsuccessfully

### Requirement: A Slack token comes from the environment or the keychain and is never stored
The Slack connector SHALL take its token from `SLACK_USER_TOKEN`, else from the keychain item `rastro-slack`, and the token SHALL never appear in the database, the config or any output.

#### Scenario: the keychain item is used when the variable is unset
- WHEN `SLACK_USER_TOKEN` is unset and the keychain item `rastro-slack` holds a valid token
- THEN the check reports authenticated naming the keychain item `rastro-slack`

#### Scenario: no token anywhere
- WHEN a Slack source syncs with no `SLACK_USER_TOKEN` and no keychain item
- THEN coverage records "couldn't look: auth"

#### Scenario: the token leaves no trace
- WHEN a check and a sync have run with a token
- THEN the token's bytes appear in none of the database file, the config, stdout or stderr

### Requirement: A Slack source names one channel by its id
`rastro source add --connector slack` SHALL require a `channel` option holding a Slack channel id, and SHALL refuse a channel name with an error naming the declared option, saving nothing.

#### Scenario: a channel id is saved
- WHEN `rastro source add --connector slack --option channel=C0123ABCDE` runs
- THEN the source is saved with that channel

#### Scenario: a channel name is refused
- WHEN `rastro source add --connector slack --option channel=general` runs
- THEN it is refused with an error naming the declared `channel` option, and nothing is saved

### Requirement: The Slack app template asks only for the read scopes
The Slack connector SHALL ship a Slack app manifest template whose user scopes are exactly the five read scopes, with no bot scopes, token rotation off and a display name the owner chooses.

#### Scenario: the template's scopes
- WHEN the shipped template is parsed
- THEN its user scopes equal the five read scopes, it declares no bot scopes, and token rotation is off
