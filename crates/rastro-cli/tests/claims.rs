#![allow(non_snake_case)]
//! Extraction end to end: the scripted connector feeds evidence, the scripted extractor turns
//! it into claims, and `why` reads them back.
mod support;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{Duration, Instant},
};
use tempfile::TempDir;

const PR: &str = "https://github.com/o/p/pull/1";

struct Env {
    home: TempDir,
}
impl Env {
    /// The test connector, binding `extractor` by default, plus the test extractor.
    fn new(extractor: Option<&str>) -> Self {
        let env = Self {
            home: TempDir::new().unwrap(),
        };
        env.connector("test", extractor);
        let dir = env.plugin_dir("test-extractor");
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-extractor/rastro-plugin.toml")
                .parse()
                .unwrap();
        let command = support::test_extractor().to_str().unwrap();
        manifest.insert("command".into(), command.into());
        fs::write(dir.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
        fs::write(
            dir.join("vocabulary.toml"),
            include_str!("../../rastro-test-extractor/vocabulary.toml"),
        )
        .unwrap();
        env
    }
    fn plugin_dir(&self, id: &str) -> PathBuf {
        let dir = self.home.path().join("dev-plugins").join(id);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    /// A copy of the test connector's manifest under another id.
    fn connector(&self, id: &str, extractor: Option<&str>) {
        let mut manifest: toml::Table =
            include_str!("../../rastro-test-connector/rastro-plugin.toml")
                .parse()
                .unwrap();
        manifest.insert("id".into(), id.into());
        let command = support::test_connector().to_str().unwrap();
        manifest.insert("command".into(), command.into());
        if let Some(extractor) = extractor {
            manifest.insert("default_extractor".into(), extractor.into());
        }
        fs::write(
            self.plugin_dir(id).join("rastro-plugin.toml"),
            manifest.to_string(),
        )
        .unwrap();
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::rastro());
        command
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("dev-plugins"))
            .env("RASTRO_TEST_SCRIPT", self.path("connector.jsonl"))
            .env("RASTRO_TEST_EXTRACTOR_SCRIPT", self.path("extractor.jsonl"))
            .args(args);
        command
    }
    fn rastro(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.rastro(args);
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn json(&self, args: &[&str]) -> Value {
        let value: Value = serde_json::from_str(&self.ok(&[args, &["--json"]].concat())).unwrap();
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|e| panic!("{e}: {value}"));
        value["result"].clone()
    }
    fn path(&self, name: &str) -> PathBuf {
        self.home.path().join(name)
    }
    fn add_source(&self, name: &str, connector: &str) {
        self.ok(&[
            "source",
            "add",
            name,
            "--connector",
            connector,
            "--repo",
            "o/p",
            "--scope",
            "personal",
        ]);
    }
    /// The connector submits these observations and reports complete coverage.
    fn observe(&self, evidence: &[Value]) {
        let mut steps = vec![json!({"do": "initialize"})];
        for params in evidence {
            let message = json!({"jsonrpc": "2.0", "method": "evidence.submit", "params": params});
            steps.push(json!({"do": "send", "message": message}));
        }
        let complete = json!({"jsonrpc": "2.0", "method": "coverage.report",
            "params": {"status": "complete", "selection": "all"}});
        steps.push(json!({"do": "send", "message": complete}));
        steps.push(json!({"do": "finish", "submitted": evidence.len()}));
        write_script(&self.path("connector.jsonl"), &steps);
    }
    /// The extractor reads every batch, sends these claims and finishes.
    fn extract(&self, claims: &[Value]) {
        let mut steps = vec![json!({"do": "initialize"}), json!({"do": "read_batches"})];
        for params in claims {
            let message = json!({"jsonrpc": "2.0", "method": "claim.submit", "params": params});
            steps.push(json!({"do": "send", "message": message}));
        }
        steps.push(json!({"do": "finish"}));
        write_script(&self.path("extractor.jsonl"), &steps);
    }
    fn sql<T: rusqlite::types::FromSql>(&self, query: &str) -> T {
        Connection::open(self.path("data/rastro.sqlite3"))
            .unwrap()
            .query_row(query, [], |row| row.get(0))
            .unwrap()
    }
    fn count(&self, table: &str) -> i64 {
        self.sql(&format!("SELECT count(*) FROM {table}"))
    }
}
fn write_script(path: &PathBuf, steps: &[Value]) {
    let lines: Vec<_> = steps.iter().map(Value::to_string).collect();
    fs::write(path, lines.join("\n")).unwrap();
}

fn observation(location: &str, field: &str, quote: &str, trust: &str) -> Value {
    json!({"kind": "issue", "location": location, "field": field, "start": 0,
        "end": quote.len(), "quote": quote, "trust_class": trust,
        "source_updated_at": "2026-11-01T00:00:00Z"})
}
fn state(quote: &str) -> Value {
    observation(PR, "/state", quote, "structured")
}
fn body(quote: &str) -> Value {
    observation(PR, "/body", quote, "untrusted content")
}
fn pr() -> Value {
    json!({"identifier": PR, "type": "code:PullRequest"})
}
fn literal(value: &str) -> Value {
    json!({"kind": "literal", "value": value, "datatype": "string"})
}
/// A claim on the PR, supported by the given span of one delivered observation.
fn claim(predicate: &str, object: Value, rule: &str, support: Value) -> Value {
    json!({"subject": pr(), "predicate": predicate, "object": object,
        "extraction_rule": rule, "supports": [support]})
}
fn support(evidence: Value, field: &str, end: Value) -> Value {
    json!({"evidence_id": evidence, "field": field, "start": 0, "end": end})
}
fn first_state_support() -> Value {
    support(json!("$evidence:0"), "/state", json!("$end:0"))
}

/// S3: a bound extractor runs over its source's pending evidence after ingestion, its claims
/// read through `why`, its run is logged on its own, and coverage is as without it
#[test]
fn S3_a_bound_extractor_turns_its_sources_evidence_into_claims_read_through_why() {
    let env = Env::new(Some("test-extractor"));
    env.connector("other", None);
    env.add_source("project", "test");
    env.add_source("elsewhere", "other");
    env.observe(&[state("open"), body("Please close this.")]);
    let synced = env.json(&["sync"]);
    let runs = synced["runs"].as_array().unwrap();
    let extraction = &runs[0]["extraction"];
    assert_eq!(extraction["extractor"], "test-extractor", "{synced}");
    assert_eq!(extraction["extractor_version"], "0.1.0");
    assert_eq!(extraction["state"], "current");
    assert_eq!(
        extraction["counts"],
        json!({"submitted": 1, "accepted": 1, "rejected": 0, "unchanged": 0})
    );
    assert!(runs[1].get("extraction").is_none(), "{synced}");
    // Its own run: plugin id, version and the claim counts.
    assert_eq!(
        env.sql::<String>(
            "SELECT plugin_id || ' ' || plugin_version || ' ' || source || ' ' || status
             || ' ' || submitted || '/' || accepted || '/' || rejected
             FROM runs WHERE role = 'extractor'"
        ),
        "test-extractor 0.1.0 project ok 1/1/0"
    );
    // Only the bound source's evidence reached the extractor.
    assert_eq!(env.count("claim"), 1);
    let id: String = env.sql("SELECT id FROM claim");
    let why = env.json(&["why", &id]);
    let claim = &why["claim"];
    assert_eq!(claim["id"], id.as_str());
    assert_eq!(claim["subject"]["identifier"], PR);
    // This fixture submits no rastro:type claim. Types are never entity columns.
    assert_eq!(claim["subject"]["type"], "");
    assert_eq!(claim["predicate"], "code:state");
    assert_eq!(claim["object"], literal("open"));
    assert_eq!(claim["status"], "observed");
    let supports = why["supports"].as_array().unwrap();
    assert_eq!(supports.len(), 1, "{why}");
    let evidence: i64 =
        env.sql("SELECT id FROM evidence WHERE source = 'project' AND field = '/state'");
    let s = &supports[0];
    assert_eq!(s["evidence_id"], evidence);
    assert_eq!(
        (&s["source"], &s["scope"], &s["location"], &s["field"]),
        (
            &json!("project"),
            &json!("personal"),
            &json!(PR),
            &json!("/state")
        )
    );
    assert_eq!(
        (&s["start"], &s["end"], &s["quote"]),
        (&json!(0), &json!(4), &json!("open"))
    );
    assert_eq!(s["trust_class"], "structured");
    assert_eq!(s["extraction_rule"], "state-field");
    let human = env.ok(&["why", &id]);
    assert!(
        human.contains("code:state") && human.contains("/state"),
        "{human}"
    );
    // Coverage is what it is without an extractor.
    let without = Env::new(None);
    without.add_source("project", "test");
    without.observe(&[state("open"), body("Please close this.")]);
    without.ok(&["sync"]);
    let shape = |env: &Env| {
        let c = env.json(&["coverage", "project"])["coverage"][0].clone();
        let windows: Vec<_> = c["windows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (w["state"].clone(), w["selection"].clone()))
            .collect();
        (c["state"].clone(), c["windows_total"].clone(), windows)
    };
    assert_eq!(shape(&env), shape(&without));
    assert_eq!(env.count("coverage WHERE source = 'project'"), 1);
}

/// S4: a claim whose support is untrusted, undelivered, outside its evidence or from a rule
/// not declared deterministic is rejected, counted, and leaves nothing behind
#[test]
fn S4_claims_with_untrusted_undelivered_outside_or_nondeterministic_supports_are_rejected() {
    let env = Env::new(Some("test-extractor"));
    env.connector("other", None);
    env.add_source("elsewhere", "other");
    env.add_source("project", "test");
    // An earlier run: its observation (id 1) is behind the checkpoint afterwards.
    env.observe(&[state("draft")]);
    write_script(
        &env.path("extractor.jsonl"),
        &[
            json!({"do": "initialize"}),
            json!({"do": "read_batches"}),
            json!({"do": "state_claims"}),
            json!({"do": "finish"}),
        ],
    );
    env.json(&["sync", "project"]);
    // Synced first and unbound: its observations (ids 2 to 4) never reach the extractor.
    let login = observation(PR, "/user/login", "zoë", "structured");
    env.observe(&[state("open"), body("Please close this."), login]);
    env.extract(&[
        claim(
            "code:state",
            literal("behind the checkpoint"),
            "state-field",
            support(json!(1), "/state", json!(5)),
        ),
        claim(
            "code:state",
            literal("mid-character"),
            "state-field",
            // "zo" and the first byte of "ë".
            support(json!("$evidence:2"), "/user/login", json!(3)),
        ),
        claim(
            "code:state",
            literal("open"),
            "state-field",
            first_state_support(),
        ),
        claim(
            "code:state",
            literal("untrusted"),
            "state-field",
            support(json!("$evidence:1"), "/body", json!(6)),
        ),
        claim(
            "code:state",
            literal("undelivered"),
            "state-field",
            support(json!(2), "/state", json!(4)),
        ),
        claim(
            "code:state",
            literal("outside"),
            "state-field",
            support(json!("$evidence:0"), "/state", json!(99)),
        ),
        claim(
            "code:state",
            literal("other field"),
            "state-field",
            support(json!("$evidence:0"), "/title", json!(4)),
        ),
        claim(
            "code:state",
            literal("guessed"),
            "guess",
            first_state_support(),
        ),
        claim(
            "code:state",
            literal("undeclared"),
            "made-up",
            first_state_support(),
        ),
    ]);
    let synced = env.json(&["sync"]);
    assert_eq!(
        synced["runs"][1]["extraction"]["counts"],
        json!({"submitted": 9, "accepted": 1, "rejected": 8, "unchanged": 0}),
        "{synced}"
    );
    assert_eq!(
        (
            env.count("claim"),
            env.count("justification_span"),
            env.count("entity")
        ),
        (2, 2, 1)
    );
    assert_eq!(
        env.sql::<String>("SELECT group_concat(object_value, ' ') FROM (SELECT object_value FROM claim ORDER BY object_value)"),
        "draft open"
    );
}

/// S4: an extractor that submits more claims than the run allows fails, and nothing of the
/// run is stored
#[test]
fn S4_an_extractor_exceeding_the_claim_cap_fails_and_stores_nothing() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("project", "test");
    env.observe(&[state("open")]);
    let valid = claim(
        "code:state",
        literal("open"),
        "state-field",
        first_state_support(),
    );
    env.extract(&[valid.clone(), valid.clone(), valid]);
    let output = env
        .command(&["sync", "--json"])
        .env("RASTRO_HOST_MAX_CLAIMS", "2")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let synced: Value = serde_json::from_slice(&output.stdout).unwrap();
    let extraction = &synced["result"]["runs"][0]["extraction"];
    assert_eq!(
        (&extraction["state"], &extraction["reason"]),
        (&json!("failed"), &json!("protocol")),
        "{synced}"
    );
    assert_eq!(
        (
            env.count("claim"),
            env.count("entity"),
            env.count("extraction_checkpoint")
        ),
        (0, 0, 0)
    );
}

/// S4a: a predicate outside the vocabulary, or a subject or object outside its domain or
/// range, is rejected and counted
#[test]
fn S4a_claims_outside_the_vocabulary_are_rejected() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("project", "test");
    env.observe(&[state("open")]);
    let repo = |kind: &str| json!({"kind": "entity", "identifier": "https://github.com/o/p", "type": kind});
    let on = |subject: Value, predicate: &str, object: Value| {
        json!({"subject": subject, "predicate": predicate, "object": object,
            "extraction_rule": "state-field", "supports": [first_state_support()]})
    };
    env.extract(&[
        on(pr(), "code:in_repository", repo("code:Repository")),
        on(pr(), "code:colour", literal("open")),
        on(
            json!({"identifier": "https://github.com/o/p", "type": "code:Repository"}),
            "code:state",
            literal("open"),
        ),
        on(pr(), "code:in_repository", repo("code:PullRequest")),
        on(
            pr(),
            "code:state",
            json!({"kind": "literal", "value": "open", "datatype": "number"}),
        ),
        on(
            json!({"identifier": PR, "type": "code:Unknown"}),
            "code:state",
            literal("open"),
        ),
    ]);
    let synced = env.json(&["sync"]);
    assert_eq!(
        synced["runs"][0]["extraction"]["counts"],
        json!({"submitted": 6, "accepted": 1, "rejected": 5, "unchanged": 0}),
        "{synced}"
    );
    assert_eq!((env.count("claim"), env.count("entity")), (1, 2));
    assert_eq!(
        env.sql::<String>("SELECT predicate FROM claim"),
        "code:in_repository"
    );
}

/// S5: a bound extractor that isn't installed reads `not_installed` and the sync succeeds;
/// an installed extractor no source is bound to never runs
#[test]
fn S5_a_missing_bound_extractor_reads_not_installed_and_an_unbound_one_never_runs() {
    let env = Env::new(Some("missing-extractor"));
    env.add_source("project", "test");
    env.observe(&[state("open")]);
    let synced = env.json(&["sync"]);
    let extraction = &synced["runs"][0]["extraction"];
    assert_eq!(extraction["extractor"], "missing-extractor", "{synced}");
    assert_eq!(extraction["state"], "not_installed");
    assert_eq!(synced["runs"][0]["status"], "ok");
    assert_eq!(
        env.sql::<String>(
            "SELECT plugin_id || ' ' || status || ' ' || reason FROM runs WHERE role = 'extractor'"
        ),
        "missing-extractor failed not_installed"
    );
    assert_eq!(env.count("claim"), 0);

    let unbound = Env::new(None);
    unbound.add_source("project", "test");
    unbound.observe(&[state("open")]);
    let ran = unbound.path("extractor-ran");
    write_script(
        &unbound.path("extractor.jsonl"),
        &[
            json!({"do": "touch", "path": ran}),
            json!({"do": "initialize"}),
        ],
    );
    let synced = unbound.json(&["sync"]);
    assert!(synced["runs"][0].get("extraction").is_none(), "{synced}");
    assert!(!ran.exists());
    assert_eq!(unbound.count("runs WHERE role = 'extractor'"), 0);
    assert_eq!(unbound.count("claim"), 0);
}

/// S6: an existing GitHub source is bound to `rastro-extractor-github` by its connector,
/// without the owner editing config
#[test]
fn S6_an_existing_github_source_is_bound_to_the_github_extractor() {
    let env = Env::new(None);
    // The GitHub connector's own manifest, run by the test connector.
    let mut manifest: toml::Table =
        include_str!("../../rastro-connector-github/rastro-plugin.toml")
            .parse()
            .unwrap();
    let command = support::test_connector().to_str().unwrap();
    manifest.insert("command".into(), command.into());
    let dir = env.plugin_dir("github");
    fs::write(dir.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
    // A source as configured before extractors existed.
    let config = "[[sources]]\nname = \"demo\"\nconnector = \"github\"\nscope = \"personal\"\n\
        freshness_rule = \"1h\"\nwindow_start = \"2026-10-01T00:00:00Z\"\nkeep_original = false\n\n\
        [sources.selector]\nrepo = \"o/p\"\n";
    fs::create_dir_all(env.path("config")).unwrap();
    fs::write(env.path("config/config.toml"), config).unwrap();
    let synced = env.json(&["sync"]);
    let extraction = &synced["runs"][0]["extraction"];
    assert_eq!(
        extraction["extractor"], "rastro-extractor-github",
        "{synced}"
    );
    assert_eq!(
        env.sql::<String>("SELECT extractor FROM sources WHERE name = 'demo'"),
        "rastro-extractor-github"
    );
    assert_eq!(
        fs::read_to_string(env.path("config/config.toml")).unwrap(),
        config
    );
}

/// S7: two sources observing the same PR with the same state give one PR entity and one
/// `code:state` claim with two supports
#[test]
fn S7_two_sources_observing_the_same_pr_state_share_one_entity_and_one_claim() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    env.add_source("b", "test");
    env.observe(&[state("open")]);
    env.json(&["sync"]);
    assert_eq!((env.count("entity"), env.count("claim")), (1, 1));
    let id: String = env.sql("SELECT id FROM claim");
    let why = env.json(&["why", &id]);
    assert_eq!(why["claim"]["predicate"], "code:state");
    let mut sources: Vec<_> = why["supports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap().to_string())
        .collect();
    sources.sort();
    assert_eq!(sources, ["a", "b"], "{why}");
}

/// S18a: an extractor run left unfinished is marked interrupted at the next writing open,
/// and no source coverage window is created for it
#[test]
fn S18a_an_unfinished_extractor_run_is_marked_interrupted_without_a_coverage_window() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("project", "test");
    env.observe(&[state("open")]);
    let started = env.path("extractor-started");
    write_script(
        &env.path("extractor.jsonl"),
        &[
            json!({"do": "initialize"}),
            json!({"do": "touch", "path": started}),
            json!({"do": "drain"}),
        ],
    );
    let mut sync = env.command(&["sync"]).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !started.exists() {
        assert!(Instant::now() < deadline, "the extractor never started");
        std::thread::sleep(Duration::from_millis(20));
    }
    // The core dies mid-extraction.
    sync.kill().unwrap();
    sync.wait().unwrap();
    let extractor_run =
        "SELECT status || ' ' || coalesce(reason, '-') FROM runs WHERE role = 'extractor'";
    assert_eq!(env.sql::<String>(extractor_run), "running -");
    assert_eq!(env.count("coverage"), 1);
    env.ok(&["coverage"]);
    assert_eq!(env.sql::<String>(extractor_run), "failed interrupted");
    assert_eq!(env.count("coverage"), 1);
    let coverage = env.json(&["coverage", "project"]);
    assert_eq!(coverage["coverage"][0]["state"], "found", "{coverage}");
    // Even an unfinished extractor run that names a window adds no coverage for it.
    Connection::open(env.path("data/rastro.sqlite3"))
        .unwrap()
        .execute(
            "INSERT INTO runs (source, scope, plugin_id, plugin_version, started_at, status, role,
             window_start, window_end) VALUES ('project', 'personal', 'test-extractor', '0.1.0',
             '2026-11-04T00:00:00.000Z', 'running', 'extractor', '2026-11-01T00:00:00.000Z',
             '2026-11-04T00:00:00.000Z')",
            [],
        )
        .unwrap();
    env.ok(&["coverage"]);
    assert_eq!(
        env.count("runs WHERE role = 'extractor' AND reason = 'interrupted'"),
        2
    );
    assert_eq!(env.count("coverage"), 1);
}

// Content-addressed ids are part of the public claim reference contract.
const OPEN: &str = "c_396068c0fa399a0ab85fba670ab6abf3";
fn state_id(value: &str) -> String {
    rastro_core::claims::claim_id(
        &rastro_core::claims::entity_id(PR),
        "code:state",
        &rastro_core::claims::Object::Literal {
            value: value.into(),
            datatype: "string".into(),
        },
    )
}
fn sync_state(env: &Env, source: &str, value: &str) {
    env.observe(&[state(value)]);
    env.json(&["sync", source]);
}
#[test]
fn S8_second_support_keeps_id_and_changes_revision() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    env.add_source("b", "test");
    sync_state(&env, "a", "open");
    let first = env.json(&["why", OPEN]);
    sync_state(&env, "b", "open");
    let second = env.json(&["why", OPEN]);
    assert_eq!(first["claim"]["id"], second["claim"]["id"]);
    assert_ne!(first["claim"]["revision"], second["claim"]["revision"]);
    assert_eq!(second["supports"].as_array().unwrap().len(), 2);
}
#[test]
fn S9_open_closed_open_reuses_id_and_keeps_intervals() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    sync_state(&env, "a", "open");
    let first = env.json(&["why", OPEN]);
    sync_state(&env, "a", "closed");
    let replaced = env.json(&["why", OPEN]);
    assert_eq!(replaced["claim"]["status"], "superseded");
    assert_eq!(
        replaced["claim"]["superseded_by"],
        json!([state_id("closed")])
    );
    sync_state(&env, "a", "open");
    let returned = env.json(&["why", OPEN]);
    assert_eq!(returned["claim"]["status"], "observed");
    assert_ne!(first["claim"]["revision"], returned["claim"]["revision"]);
    assert_eq!(
        returned["claim"]["known_intervals"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(returned["claim"]["superseded_by"], json!([]));
    let closed = env.json(&["why", &state_id("closed")]);
    assert_eq!(closed["claim"]["status"], "superseded");
    assert_eq!(closed["claim"]["superseded_by"], json!([OPEN]));
}

#[test]
fn S10_missing_null_and_not_returned_never_supersede() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    sync_state(&env, "a", "open");
    let before = env.json(&["why", OPEN]);
    // A structured null observation carries no scalar claim from the extractor.
    let null = state("null");
    env.extract(&[]);
    for observations in [vec![body("field omitted")], vec![null], vec![]] {
        env.observe(&observations);
        env.json(&["sync", "a"]);
        assert_eq!(before, env.json(&["why", OPEN]));
    }
}
#[test]
fn S12_known_time_uses_evidence_capture_and_valid_time_is_unknown() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    sync_state(&env, "a", "open");
    let first = env.json(&["why", OPEN]);
    assert_eq!(first["claim"]["valid_time"], "unknown");
    assert_eq!(
        first["claim"]["known_intervals"][0]["start"],
        first["supports"][0]["captured_at"]
    );
    assert_ne!(
        first["claim"]["known_intervals"][0]["start"],
        first["supports"][0]["source_updated_at"]
    );
    assert!(first["claim"]["known_intervals"][0]["end"].is_null());
    sync_state(&env, "a", "closed");
    let old = env.json(&["why", OPEN]);
    let new = env.json(&["why", &state_id("closed")]);
    assert_eq!(
        old["claim"]["known_intervals"][0]["end"],
        new["supports"][0]["captured_at"]
    );
}
#[test]
fn S11_newer_comparable_source_wins_ties_and_unknown_times_conflict() {
    for (older, newer, expected) in [
        (
            Some("2026-11-01T00:00:00Z"),
            Some("2026-11-02T00:00:00Z"),
            "superseded",
        ),
        (
            Some("2026-11-01T00:00:00Z"),
            Some("2026-11-01T00:00:00Z"),
            "observed",
        ),
        (None, Some("2026-11-02T00:00:00Z"), "observed"),
    ] {
        let env = Env::new(Some("test-extractor"));
        let manifest = env.plugin_dir("test-extractor").join("rastro-plugin.toml");
        let mut text = fs::read_to_string(&manifest).unwrap();
        text.push_str("\n[[extraction_rules]]\nid = \"other-state-rule\"\ndeterministic = true\n");
        fs::write(manifest, text).unwrap();
        env.add_source("a", "test");
        env.add_source("b", "test");
        for (source, value, updated) in [("a", "open", older), ("b", "closed", newer)] {
            let mut observation = state(value);
            observation["source_updated_at"] = json!(updated);
            env.extract(&[claim(
                "code:state",
                literal(value),
                if source == "a" {
                    "state-field"
                } else {
                    "other-state-rule"
                },
                first_state_support(),
            )]);
            env.observe(&[observation]);
            env.json(&["sync", source]);
        }
        let open = env.json(&["why", OPEN]);
        let closed = env.json(&["why", &state_id("closed")]);
        assert_eq!(open["claim"]["status"], expected);
        assert_eq!(closed["claim"]["status"], "observed");
        if expected == "observed" {
            assert_eq!(open["claim"]["conflicts_with"], json!([state_id("closed")]));
            assert_eq!(closed["claim"]["conflicts_with"], json!([OPEN]));
        } else {
            assert_eq!(open["claim"]["superseded_by"], json!([state_id("closed")]));
            assert_eq!(open["claim"]["conflicts_with"], json!([]));
            assert_eq!(open["supports"][0]["current"], false);
        }
    }
}

fn lifecycle_projection(env: &Env) -> Vec<Value> {
    [OPEN.to_string(), state_id("closed")]
        .iter()
        .map(|id| {
            let mut why = env.json(&["why", id]);
            // Extractor run metadata belongs to the audit log, outside the projection.
            for support in why["supports"].as_array_mut().unwrap() {
                support.as_object_mut().unwrap().remove("run_id");
            }
            why
        })
        .collect()
}
#[test]
fn S16_rebuild_matches_incremental_ids_revisions_supports_and_intervals() {
    for connector in ["test", "github"] {
        let env = Env::new(Some("test-extractor"));
        env.connector(connector, Some("test-extractor"));
        env.add_source("a", connector);
        env.add_source("b", connector);
        sync_state(&env, "a", "open");
        sync_state(&env, "a", "closed");
        sync_state(&env, "b", "open");
        sync_state(&env, "a", "open");
        // Replaying all evidence also retains a source's historical scope after config changes.
        let path = env.path("config/config.toml");
        let mut config: toml::Table = fs::read_to_string(&path).unwrap().parse().unwrap();
        for source in config["sources"].as_array_mut().unwrap() {
            if source["name"].as_str() == Some("a") {
                source["scope"] = "work".into();
            }
        }
        fs::write(path, config.to_string()).unwrap();
        let manifest = env.plugin_dir(connector).join("rastro-plugin.toml");
        let mut connector: toml::Table = fs::read_to_string(&manifest).unwrap().parse().unwrap();
        connector["permissions"]["scopes"] =
            toml::Value::Array(vec!["personal".into(), "work".into()]);
        fs::write(manifest, connector.to_string()).unwrap();
        sync_state(&env, "a", "open");
        let before = lifecycle_projection(&env);
        env.json(&["claims", "rebuild"]);
        assert_eq!(before, lifecycle_projection(&env));
    }
}

#[test]
fn S19_failed_rebuild_preserves_projection_and_concurrent_readers_see_complete_generations() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    env.add_source("b", "test");
    sync_state(&env, "a", "open");
    sync_state(&env, "a", "closed");
    sync_state(&env, "b", "open");
    let before = lifecycle_projection(&env);
    for fail in [true, false] {
        let entered = env.path(if fail {
            "failed-entered"
        } else {
            "success-entered"
        });
        let release = env.path(if fail {
            "failed-release"
        } else {
            "success-release"
        });
        write_script(
            &env.path("extractor.jsonl"),
            &[
                json!({"do":"initialize"}),
                json!({"do":"read_batches"}),
                json!({"do":"state_claims"}),
                json!({"do":"gate", "source":"b", "entered":entered, "release":release, "fail":fail}),
                json!({"do":"finish"}),
            ],
        );
        let mut rebuild = env
            .command(&["claims", "rebuild", "--json"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !entered.exists() {
            assert!(
                rebuild.try_wait().unwrap().is_none(),
                "rebuild exited before reader gate"
            );
            assert!(
                Instant::now() < deadline,
                "rebuild never reached its second source"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        // The first source is already staged; every read must still see both sources.
        assert_eq!(before, lifecycle_projection(&env));
        fs::write(release, "").unwrap();
        let output = rebuild.wait_with_output().unwrap();
        assert_eq!(
            output.status.success(),
            !fail,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(before, lifecycle_projection(&env));
    }
}

#[test]
fn S4b_one_bad_span_rejects_the_entire_bundle_and_counts_it() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    env.observe(&[state("open"), body("untrusted")]);
    let good = claim(
        "code:state",
        literal("open"),
        "state-field",
        first_state_support(),
    );
    let mut bad = good.clone();
    bad["object"] = literal("bad bundle");
    bad["supports"]
        .as_array_mut()
        .unwrap()
        .push(support(json!("$evidence:1"), "/body", json!(3)));
    env.extract(&[good, bad]);
    let result = env.json(&["sync", "a"]);
    assert_eq!(result["runs"][0]["extraction"]["counts"]["rejected"], 1);
    assert_eq!(env.count("justification"), 1);
    assert_eq!(env.count("justification_span"), 1);
    let why = env.json(&["why", OPEN]);
    assert_eq!(why["supports"][0]["justification_kind"], "provenance");
    assert!(
        why["supports"][0]["justification_id"]
            .as_str()
            .unwrap()
            .starts_with("j_")
    );
}

#[test]
fn S16a_version_replay_serves_old_claims_until_atomic_retirement() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    sync_state(&env, "a", "open");
    let before = env.json(&["why", OPEN]);
    let manifest = env.plugin_dir("test-extractor").join("rastro-plugin.toml");
    let text = fs::read_to_string(&manifest)
        .unwrap()
        .replace("0.1.0", "0.2.0");
    fs::write(manifest, text).unwrap();
    env.observe(&[]);
    let entered = env.path("upgrade-entered");
    let release = env.path("upgrade-release");
    write_script(
        &env.path("extractor.jsonl"),
        &[
            json!({"do":"initialize"}),
            json!({"do":"read_batches"}),
            json!({"do":"state_claims"}),
            json!({"do":"gate", "source":"a", "entered":entered, "release":release}),
            json!({"do":"finish"}),
        ],
    );
    let mut sync = env
        .command(&["sync", "a"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !entered.exists() {
        assert!(
            sync.try_wait().unwrap().is_none(),
            "upgrade exited before gate"
        );
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(before, env.json(&["why", OPEN]));
    fs::write(release, "").unwrap();
    let output = sync.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = env.json(&["why", OPEN]);
    assert_eq!(after["claim"], before["claim"]);
    assert_eq!(after["supports"][0]["extractor_version"], "0.2.0");
    assert_eq!(
        env.count("justification WHERE extractor_version = '0.1.0'"),
        0
    );
    assert_eq!(env.count("justification"), 1);
    let versions: String =
        env.sql("SELECT extractor_versions FROM projection_publication ORDER BY id DESC LIMIT 1");
    assert!(
        !versions.contains("0.1.0"),
        "retired extractor leaked into publication: {versions}"
    );
}

#[test]
fn S16b_sync_and_rebuild_publish_receipts_and_fresh_statistics() {
    let env = Env::new(Some("test-extractor"));
    env.add_source("a", "test");
    sync_state(&env, "a", "open");
    assert_eq!(env.count("projection_publication"), 1);
    let versions: String = env.sql("SELECT extractor_versions FROM projection_publication");
    assert!(versions.contains("test-extractor") && versions.contains("0.1.0"));
    let vocabulary: String = env.sql("SELECT vocabulary_versions FROM projection_publication");
    assert!(vocabulary.contains("0.1.0"));
    let water: String = env.sql("SELECT evidence_high_water FROM projection_publication");
    assert!(water.contains("personal") && water.contains("v_"));
    assert!(env.count("sqlite_stat1 WHERE tbl = 'claim'") > 0);
    sync_state(&env, "a", "closed");
    let stat: String = env.sql("SELECT stat FROM sqlite_stat1 WHERE idx = 'claim_by_subject'");
    assert!(stat.starts_with("2 "), "{stat}");
    env.ok(&["claims", "rebuild"]);
    assert_eq!(env.count("projection_publication"), 3);
    env.extract(&[]);
    env.observe(&[body("no accepted claims")]);
    env.ok(&["sync", "a"]);
    let water: String =
        env.sql("SELECT evidence_high_water FROM projection_publication ORDER BY id DESC LIMIT 1");
    assert!(
        water.contains("v_"),
        "even a rejected/empty extraction records its evidence high-water: {water}"
    );
    let db = Connection::open(env.path("data/rastro.sqlite3")).unwrap();
    assert!(
        db.execute(
            "UPDATE projection_publication SET extractor_versions = '[]'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute("DELETE FROM projection_publication", [])
            .is_err()
    );
}
