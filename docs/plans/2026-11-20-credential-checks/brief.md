# Brief: Credential checks

Scheduled jobs need the same Linear credential for reading and writing, and owners need an authentication check before the first sync. The writer uses the connector's `rastro-linear` keychain item after the environment (T1). A CLI-only source check probes authentication without changing stored state, and source setup prints its result without rolling back on failure (T2). Credentials never appear in diagnostic output. GitHub retains its existing credential resolution, including `gh auth token`.

Approved by felippe.bisca@gmail.com in session on 2026-11-20, against HEAD `134ee9d43827f6fdeaf91ac52c0600126379ab8a`.
