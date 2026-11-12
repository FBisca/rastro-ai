#![allow(non_snake_case)]
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::knowledge::WhyOptions;
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::{Value, json};
use std::{collections::BTreeSet, process::Command};
use tempfile::TempDir;

const URL: &str = "https://github.com/o/p/pull/1";
fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        env.ok(&[
            "source",
            "add",
            "project",
            "--connector",
            "github",
            "--repo",
            "o/p",
            "--scope",
            "work",
        ]);
        env
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)));
        (out.status.success(), value)
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{value}");
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|error| panic!("{error}: {value}"));
        value["result"].clone()
    }
    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }
    fn claim(&self, predicate: &str, object: Object, captured: DateTime<Utc>) -> String {
        self.claim_for(URL, predicate, object, captured)
    }
    fn claim_for(
        &self,
        identifier: &str,
        predicate: &str,
        object: Object,
        captured: DateTime<Utc>,
    ) -> String {
        let source = Source::github("project".into(), "o/p".into(), Scope("work".into()));
        let mut db = self.db();
        let run = db
            .start_run(&source, ("fixture", "1"), captured, (captured, captured))
            .unwrap();
        let quote = match &object {
            Object::Literal { value, .. } => value.clone(),
            Object::Entity { identifier, .. } => identifier.clone(),
        };
        let e = Evidence {
            source: source.name.clone(),
            scope: source.scope.clone(),
            location: identifier.into(),
            field: format!("/{predicate}"),
            start: 0,
            end: quote.len() as u64,
            quote,
            trust_class: TrustClass::Structured,
            captured_at: captured,
            source_updated_at: Some(captured),
            original: None,
        };
        let evidence_id = match db.append_evidence(run, &e).unwrap() {
            rastro_core::Appended::New(id) | rastro_core::Appended::Unchanged(id) => id,
        };
        let subject = Entity::new(identifier.into(), "code:PullRequest".into());
        let id = claims::claim_id(&subject.id, predicate, &object);
        db.record_claims(
            run,
            ("fixture", "1"),
            &[ObservedClaim {
                id: id.clone(),
                subject,
                predicate: predicate.into(),
                object,
                scalar: [
                    "code:state",
                    "tracker:state",
                    "rastro:alias",
                    claims::TYPE_PREDICATE,
                ]
                .contains(&predicate),
                type_ancestors: Vec::new(),
                supports: BTreeSet::from([Support {
                    portable_key: claims::portable_evidence_key(&e),
                    evidence_id,
                    field: e.field,
                    start: 0,
                    end: e.end,
                    extraction_rule: "fixture-rule".into(),
                }]),
            }],
            None,
        )
        .unwrap();
        id
    }
}
fn literal(value: &str) -> Object {
    Object::Literal {
        value: value.into(),
        datatype: "string".into(),
    }
}

fn alias(kind: &str, value: &str) -> Object {
    Object::Literal {
        value: value.into(),
        datatype: kind.into(),
    }
}

/// claims/R30: about resolves exact forms and returns one hop with aliases
#[test]
fn about_resolves_exact_forms_and_returns_one_hop_with_aliases() {
    let env = Env::new();
    let state = env.claim("code:state", literal("open"), time());
    env.claim("rastro:alias", alias("pr", "o/p/1"), time());
    let entity = Entity::new("github:account/42".into(), "code:Account".into());
    env.claim_for(
        &entity.identifier,
        "rastro:alias",
        alias("login", "ana"),
        time(),
    );
    let author = env.claim(
        "code:author",
        Object::Entity {
            id: entity.id.clone(),
            identifier: entity.identifier.clone(),
            entity_type: entity.entity_type.clone(),
        },
        time(),
    );
    let expected = env.ok(&["about", URL]);
    assert_eq!(expected["claims"].as_array().unwrap().len(), 3);
    assert_eq!(
        env.ok(&["about", &claims::entity_id(URL)])["claims"],
        expected["claims"]
    );
    let rows = expected["claims"].as_array().unwrap();
    let row = rows.iter().find(|r| r["id"] == state).unwrap();
    assert_eq!(row["status"], "observed");
    assert!(row["revision"].as_str().unwrap().starts_with("r_"));
    assert_eq!(row["predicate"], "code:state");
    assert_eq!(row["subject_aliases"], json!(["o/p/1"]));
    assert_eq!(row["support_count"], 1);
    let author_row = rows.iter().find(|r| r["id"] == author).unwrap();
    assert_eq!(author_row["object_aliases"], json!(["ana"]));
    let incoming = env.ok(&["about", &entity.id]);
    assert!(
        incoming["claims"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == author)
    );
    assert_eq!(incoming["claims"].as_array().unwrap().len(), 2);
    let other = "github:account/99";
    env.claim_for(other, "rastro:alias", alias("login", "ana"), time());
    let (ok, error) = env.run(&["about", "github:login/ana"]);
    assert!(!ok, "{error}");
    let error = error["error"].as_str().unwrap();
    assert!(
        error.contains("ambiguous alias")
            && error.contains(&entity.id)
            && error.contains(&entity.identifier)
            && error.contains(&claims::entity_id(other))
            && error.contains(other),
        "{error}"
    );
    // Aliases cannot shadow an exact identifier or entity id.
    for exact in [URL.to_string(), claims::entity_id(URL)] {
        env.claim_for(&entity.identifier, "rastro:alias", literal(&exact), time());
        let resolved = env.ok(&["about", &exact]);
        assert_eq!(resolved["claims"].as_array().unwrap().len(), 3);
        assert!(
            resolved["claims"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == state)
        );
    }
    // `me` is the reserved fourth form (surfaces §38); any other bare word stays an error that
    // names the four.
    for bad in ["felippe", "Me", "o/p", "https://github.com/o p/pull/1"] {
        let (ok, error) = env.run(&["about", bad]);
        assert!(!ok, "{error}");
        let text = error["error"].as_str().unwrap();
        assert!(
            text.contains("`me`")
                && text.contains("entity id")
                && text.contains("canonical alias")
                && text.contains("exact canonical URL"),
            "{text}"
        );
    }
}

/// claims/R32: about filters known time orders limits and includes history
#[test]
fn about_filters_known_time_orders_limits_and_includes_history() {
    let env = Env::new();
    let open = env.claim("code:state", literal("open"), time());
    let later = time() + chrono::Duration::days(1);
    let closed = env.claim("code:state", literal("closed"), later);
    let a = env.claim("code:a", literal("a"), later);
    let b = env.claim("code:b", literal("b"), later);
    let current = env.ok(&[
        "about",
        URL,
        "--since",
        "2026-11-02T00:00:00Z",
        "--limit",
        "1",
    ]);
    let mut expected = [closed, a, b];
    expected.sort();
    assert_eq!(current["claims"][0]["id"], expected[0]);
    assert_eq!(current["claims"].as_array().unwrap().len(), 1);
    assert_eq!(current["truncated"], true);
    let history = env.ok(&["about", URL, "--history", "--limit", "50"]);
    assert_eq!(history["claims"][3]["id"], open);
    assert_eq!(history["claims"][3]["status"], "superseded");
    assert_eq!(history["truncated"], false);
    assert_eq!(
        env.ok(&["about", URL])["claims"].as_array().unwrap().len(),
        3
    );
    assert!(
        env.ok(&["about", URL, "--since", "2026-11-03T00:00:00Z"])["claims"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for limit in ["0", "51", "1.5", "-1"] {
        assert!(!env.run(&["about", URL, "--limit", limit]).0);
    }
    assert!(!env.run(&["about", URL, "--since", "yesterday"]).0);
    for n in 0..21 {
        env.claim(&format!("code:n{n}"), literal("x"), later);
    }
    let default = env.ok(&["about", URL]);
    assert_eq!(default["claims"].as_array().unwrap().len(), 20);
    assert_eq!(default["truncated"], true);
    // A later corroborating observation does not reopen an already-known claim interval.
    let corroborated = Env::new();
    let id = corroborated.claim("code:state", literal("open"), time());
    let source = Source::github("second".into(), "o/p".into(), Scope("work".into()));
    let original_source = Source::github("project".into(), "o/p".into(), Scope("work".into()));
    let mut db = corroborated.db();
    db.project_sources(&[original_source, source.clone()])
        .unwrap();
    let run = db
        .start_run(&source, ("fixture", "1"), later, (later, later))
        .unwrap();
    let evidence = Evidence {
        source: source.name.clone(),
        scope: source.scope.clone(),
        location: URL.into(),
        field: "/code:state".into(),
        start: 0,
        end: 4,
        quote: "open".into(),
        trust_class: TrustClass::Structured,
        captured_at: later,
        source_updated_at: Some(later),
        original: None,
    };
    db.append_evidence(run, &evidence).unwrap();
    let evidence_id = db.pending_evidence(("fixture", "1"), &source).unwrap()[0].0;
    let subject = Entity::new(URL.into(), "code:PullRequest".into());
    db.record_claims(
        run,
        ("fixture", "1"),
        &[ObservedClaim {
            id,
            subject,
            predicate: "code:state".into(),
            object: literal("open"),
            scalar: true,
            type_ancestors: Vec::new(),
            supports: BTreeSet::from([Support {
                portable_key: claims::portable_evidence_key(&evidence),
                evidence_id,
                field: evidence.field,
                start: 0,
                end: 4,
                extraction_rule: "fixture-rule".into(),
            }]),
        }],
        None,
    )
    .unwrap();
    drop(db);
    assert_eq!(
        corroborated.ok(&["about", URL, "--since", "2026-11-02T00:00:00Z"])["claims"],
        json!([])
    );
}

/// claims/R35: why bounds complete justifications and clips utf8 quotes
#[test]
fn why_bounds_complete_justifications_and_clips_utf8_quotes() {
    let env = Env::new();
    let quote = "é🦀".repeat(100);
    let id = env.claim("code:description", literal(&quote), time());
    let mut db = env.db();
    let stored = db
        .snapshot(|view| {
            view.explain(
                &id,
                &WhyOptions {
                    limit: 50,
                    full: true,
                },
            )
        })
        .unwrap()
        .unwrap();
    let original = &stored.supports[0].spans[0];
    for n in 0..21 {
        let rule = format!("rule-{n:02}");
        let spans = [(0, quote.len() as u64), (2, 6)]
            .into_iter()
            .map(|(start, end)| Support {
                portable_key: original.portable_key.clone(),
                evidence_id: original.evidence_id,
                field: original.field.clone(),
                start,
                end,
                extraction_rule: rule.clone(),
            })
            .collect();
        db.record_claims(
            1,
            ("fixture", "1"),
            &[ObservedClaim {
                id: id.clone(),
                subject: stored.claim.claim.subject.clone(),
                predicate: stored.claim.claim.predicate.clone(),
                object: stored.claim.claim.object.clone(),
                scalar: false,
                type_ancestors: Vec::new(),
                supports: spans,
            }],
            None,
        )
        .unwrap();
    }
    // Published-state fixture: retain historical justifications whose hashes sort before
    // the one live alternative, so a hash-only page would hide current evidence.
    let connection =
        rusqlite::Connection::open(env.home.path().join("data/rastro.sqlite3")).unwrap();
    connection.execute("UPDATE justification SET current = (id = (SELECT max(id) FROM justification WHERE claim_id = ?1)) WHERE claim_id = ?1", [&id]).unwrap();
    drop(connection);
    drop(db);
    let why = env.ok(&["why", &id]);
    assert_eq!(why["claim"]["support_count"], 1);
    assert_eq!(why["supports"][0]["current"], true);
    assert!(
        why["supports"]
            .as_array()
            .unwrap()
            .iter()
            .skip(1)
            .all(|s| s["current"] == false)
    );
    assert_eq!(why["supports"].as_array().unwrap().len(), 20);
    assert_eq!(why["truncated"], true);
    assert_eq!(why["claim"]["valid_time"], "unknown");
    assert!(
        !why["claim"]["known_intervals"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(why["claim"]["revision"].as_str().unwrap().starts_with("r_"));
    let limited = env.ok(&["why", &id, "--limit", "1"]);
    assert_eq!(limited["supports"].as_array().unwrap().len(), 1);
    assert_eq!(limited["truncated"], true);
    assert_eq!(limited["claim"]["support_count"], 1);
    assert_eq!(limited["supports"][0]["current"], true);
    let all = env.ok(&["why", &id, "--limit", "50", "--full"]);
    assert_eq!(all["supports"].as_array().unwrap().len(), 22);
    assert_eq!(all["truncated"], false);
    assert!(
        all["supports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["spans"].as_array().unwrap().len() == 2)
    );
    for support in why["supports"].as_array().unwrap() {
        assert_eq!(support["justification_kind"], "provenance");
        assert_eq!(support["extractor"], "fixture");
        assert_eq!(support["extractor_version"], "1");
        assert!(support.get("vocabulary_version").is_some());
        assert!(support["extraction_rule"].is_string());
        for span in support["spans"].as_array().unwrap() {
            assert!(span.get("quote").is_none());
            assert_eq!(span["evidence_id"], original.evidence_id);
            assert_eq!(span["source"], "project");
            assert_eq!(span["scope"], "work");
            assert_eq!(span["location"], URL);
            assert_eq!(span["field"], "/code:description");
            assert_eq!(span["trust_class"], "structured");
            assert_eq!(span["captured_at"], "2026-11-01T00:00:00Z");
            assert_eq!(span["source_updated_at"], "2026-11-01T00:00:00Z");
            let excerpt = &span["excerpt"];
            assert!(excerpt.get("located").is_none());
            let start = excerpt["start"].as_u64().unwrap() as usize;
            let end = excerpt["end"].as_u64().unwrap() as usize;
            assert!(quote.is_char_boundary(start) && quote.is_char_boundary(end));
            assert!(end - start <= 400);
            assert_eq!(excerpt["text"], &quote[start..end]);
            assert_eq!(excerpt["clipped_before"], false);
            assert_eq!(
                excerpt["clipped_after"],
                end < span["end"].as_u64().unwrap() as usize
            );
        }
    }
    let human = Command::new(support::rastro())
        .env("RASTRO_HOME", env.home.path())
        .args(["why", &id, "--limit", "50"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(
        human.contains("Current: true") && human.contains("Current: false"),
        "{human}"
    );
    let full = all["supports"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["spans"].as_array().unwrap())
        .find(|s| s["start"] == 0)
        .unwrap();
    assert_eq!(full["quote"], quote);
    for limit in ["0", "51", "1.5", "-1"] {
        assert!(!env.run(&["why", &id, "--limit", limit]).0);
    }
}

/// claims/R33: unknown entity returns empty claims with coverage
#[test]
fn unknown_entity_returns_empty_claims_with_coverage() {
    let env = Env::new();
    env.claim("code:state", literal("open"), time());
    let result = env.ok(&["about", "github:account/999"]);
    assert_eq!(result["claims"], json!([]));
    assert_eq!(result["truncated"], false);
    assert_eq!(result["coverage"][0]["source"], "project");
    assert_eq!(
        env.ok(&["about", "e_00000000000000000000000000000000"])["claims"],
        json!([])
    );
}

/// claims/R36: unknown claim is an operation error on catalog and cli
#[test]
fn unknown_claim_is_an_operation_error_on_catalog_and_cli() {
    let env = Env::new();
    let (ok, error) = env.run(&["why", "c_unknown"]);
    assert!(!ok);
    assert_eq!(error["operation"], "why");
    assert_eq!(error["error"], "unknown claim: c_unknown");
    let mut reader = rastro_core::Rastro::open_read_only(
        rastro_core::Locations {
            data: env.home.path().join("data"),
            config: env.home.path().join("config"),
        },
        SqliteStorage::open_read_only,
    )
    .unwrap();
    let request = rastro_operations::catalog::find("why")
        .unwrap()
        .decode(json!({"claim":"c_unknown"}))
        .unwrap();
    assert_eq!(
        request.run_read(&mut reader).unwrap_err().to_string(),
        "unknown claim: c_unknown"
    );
}

/// surfaces/R30: claim reads leave rows unchanged and missing database uncreated
#[test]
fn claim_reads_leave_rows_unchanged_and_missing_database_uncreated() {
    let env = Env::new();
    let id = env.claim("code:state", literal("open"), time());
    let database = env.home.path().join("data/rastro.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    let before: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let dump = || {
        before
            .iter()
            .map(|table| {
                let mut statement = connection
                    .prepare(&format!("SELECT * FROM \"{table}\" ORDER BY rowid"))
                    .or_else(|_| connection.prepare(&format!("SELECT * FROM \"{table}\"")))
                    .unwrap();
                let count = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..count)
                            .map(|i| row.get::<_, rusqlite::types::Value>(i))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                format!("{table}: {rows:?}")
            })
            .collect::<Vec<_>>()
    };
    let original = dump();
    env.ok(&["about", URL]);
    env.ok(&["why", &id]);
    assert_eq!(dump(), original);
    drop(connection);
    for name in ["rastro.sqlite3", "rastro.sqlite3-wal", "rastro.sqlite3-shm"] {
        let _ = std::fs::remove_file(env.home.path().join("data").join(name));
    }
    let result = env.ok(&["about", URL]);
    assert_eq!(result["claims"], json!([]));
    assert_eq!(result["coverage"][0]["state"], "never looked");
    assert_eq!(result["coverage"][0]["source"], "project");
    assert!(!database.exists());
    assert!(!env.run(&["why", &id]).0);
    assert!(!database.exists());
}

/// claims/R34: claim answers include coverage and extraction state for every pair
#[test]
fn claim_answers_include_coverage_and_extraction_state_for_every_pair() {
    let env = Env::new();
    let id = env.claim("code:state", literal("open"), time());
    env.ok(&[
        "source",
        "add",
        "other",
        "--connector",
        "github",
        "--repo",
        "o/q",
        "--scope",
        "personal",
    ]);
    let plugin_dir = env.home.path().join("config/plugins/test-extractor");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    let mut manifest: toml::Table = include_str!("../../rastro-test-extractor/rastro-plugin.toml")
        .parse()
        .unwrap();
    manifest.insert(
        "command".into(),
        support::test_extractor().to_str().unwrap().into(),
    );
    std::fs::write(plugin_dir.join("rastro-plugin.toml"), manifest.to_string()).unwrap();
    std::fs::write(
        plugin_dir.join("vocabulary.toml"),
        include_str!("../../rastro-test-extractor/vocabulary.toml"),
    )
    .unwrap();
    env.db()
        .bind_extractor("project", Some("test-extractor"))
        .unwrap();
    // A pair retained only by coverage still uses the source's bound extractor.
    let mut db = env.db();
    let archived = Source::github("project".into(), "o/p".into(), Scope("archive".into()));
    let run = db
        .start_run(&archived, ("fixture", "1"), time(), (time(), time()))
        .unwrap();
    db.record_coverage(&rastro_core::CoverageRecord {
        run_id: run,
        source: archived.name,
        scope: archived.scope,
        window: (time(), time()),
        selection: "all".into(),
        state: rastro_core::CoverageState::Empty,
        reason: None,
        retry_at: None,
        checked_at: time(),
    })
    .unwrap();
    drop(db);
    for args in [vec!["about", URL], vec!["why", &id]] {
        let answer = env.ok(&args);
        assert_eq!(answer["coverage"].as_array().unwrap().len(), 3);
        let states = answer["extraction"].as_array().unwrap();
        assert_eq!(states.len(), 3);
        let project = states
            .iter()
            .find(|s| s["source"] == "project" && s["scope"] == "work")
            .unwrap();
        let archive = states
            .iter()
            .find(|s| s["source"] == "project" && s["scope"] == "archive")
            .unwrap();
        assert_eq!(archive["extractor"], "test-extractor");
        assert_eq!(archive["extractor_version"], "0.1.0");
        assert_eq!(archive["state"], "current");
        assert_eq!(project["scope"], "work");
        assert_eq!(project["extractor"], "test-extractor");
        assert_eq!(project["extractor_version"], "0.1.0");
        assert_eq!(project["state"], "behind");
        let other = states.iter().find(|s| s["source"] == "other").unwrap();
        assert_eq!(other["scope"], "personal");
        assert_eq!(other["state"], "not_installed");
    }
}

/// claims/R31: about resolves a canonical alias by namespace kind and exact value
#[test]
fn about_resolves_a_canonical_alias_by_namespace_kind_and_exact_value() {
    let env = Env::new();
    let account = "github:account/42";
    env.claim_for(account, "rastro:alias", alias("login", "FBisca"), time());
    env.claim_for(account, "code:state", literal("active"), time());
    let expected = env.ok(&["about", account]);
    assert_eq!(expected["claims"].as_array().unwrap().len(), 2);
    assert_eq!(
        env.ok(&["about", "github:login/FBisca"])["claims"],
        expected["claims"]
    );
    for miss in [
        "gitlab:login/FBisca",
        "github:email/FBisca",
        "github:login/fbisca",
        "github:login/FBisca/x",
    ] {
        let empty = env.ok(&["about", miss]);
        assert_eq!(empty["claims"], json!([]), "{miss}");
        assert!(empty["coverage"].is_array(), "{miss}");
    }
    env.claim_for(
        "github:account/99",
        "rastro:alias",
        alias("login", "FBisca"),
        time(),
    );
    let (ok, error) = env.run(&["about", "github:login/FBisca"]);
    assert!(!ok);
    assert!(error["error"].as_str().unwrap().contains("ambiguous alias"));
}

fn later(days: i64) -> DateTime<Utc> {
    time() + chrono::Duration::days(days)
}
/// An account whose login changed from `old` to `new`; returns the old alias claim id.
fn renamed(env: &Env, account: &str, old: &str, new: &str) -> String {
    let old_claim = env.claim_for(account, "rastro:alias", alias("login", old), later(0));
    env.claim_for(account, "code:state", literal("active"), later(0));
    env.claim_for(account, "rastro:alias", alias("login", new), later(1));
    old_claim
}

/// claims/R38: an old login still resolves to its account
#[test]
fn an_old_login_resolves_to_its_account() {
    let env = Env::new();
    let account = "github:account/42";
    let old = renamed(&env, account, "oldname", "newname");
    let history = env.ok(&["about", account, "--history"]);
    let status = history["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == old.as_str())
        .unwrap()["status"]
        .clone();
    assert_eq!(status, "superseded");
    let expected = env.ok(&["about", account]);
    assert!(!expected["claims"].as_array().unwrap().is_empty());
    assert_eq!(
        env.ok(&["about", "github:login/oldname"])["claims"],
        expected["claims"]
    );
    assert_eq!(
        env.ok(&["about", "github:login/newname"])["claims"],
        expected["claims"]
    );
}

/// claims/R39: a login held by several entities names every candidate with its mark
#[test]
fn an_ambiguous_login_marks_each_candidate() {
    let env = Env::new();
    renamed(&env, "github:account/1", "shared", "other");
    env.claim_for(
        "github:account/2",
        "rastro:alias",
        alias("login", "shared"),
        later(2),
    );
    let (ok, error) = env.run(&["about", "github:login/shared"]);
    assert!(!ok);
    let text = error["error"].as_str().unwrap();
    assert!(
        text.contains("ambiguous alias github:login/shared"),
        "{text}"
    );
    assert!(text.contains("(github:account/1): historical"), "{text}");
    assert!(text.contains("(github:account/2): current"), "{text}");
}

/// claims/R40: resolving through an old login lists current aliases; why shows the old interval
#[test]
fn resolving_through_an_old_login_lists_current_aliases_only() {
    let env = Env::new();
    let account = "github:account/42";
    let old = renamed(&env, account, "oldname", "newname");
    let about = env.ok(&["about", "github:login/oldname"]);
    let rows = about["claims"].as_array().unwrap();
    assert!(!rows.is_empty());
    for row in rows {
        for key in ["subject_aliases", "object_aliases"] {
            assert!(!row[key].to_string().contains("oldname"), "{key}: {row}");
        }
    }
    assert!(
        rows.iter()
            .any(|r| r["subject_aliases"] == json!(["newname"]))
    );
    let why = env.ok(&["why", &old]);
    assert_eq!(why["claim"]["status"], "superseded");
    let intervals = why["claim"]["known_intervals"].as_array().unwrap();
    assert_eq!(intervals.len(), 1);
    assert_ne!(intervals[0]["end"], Value::Null);
}

/// claims/R39: one entity holding a login both current and historical resolves directly
#[test]
fn an_entity_that_regained_a_login_resolves_directly() {
    let env = Env::new();
    let account = "github:account/7";
    env.claim_for(account, "rastro:alias", alias("login", "a"), later(0));
    env.claim_for(account, "rastro:alias", alias("login", "b"), later(1));
    env.claim_for(account, "rastro:alias", alias("login", "a"), later(2));
    let expected = env.ok(&["about", account]);
    assert_eq!(
        env.ok(&["about", "github:login/a"])["claims"],
        expected["claims"]
    );
    assert_eq!(
        env.ok(&["about", "github:login/b"])["claims"],
        expected["claims"]
    );
}

const FORMS: &str = "entity id";
const BAD_FORMS: [&str; 10] = [
    "foo:bar",
    "github:FBisca",
    "github:login/",
    ":x/y",
    "e_0000000000000000000000000000000A",
    "git hub:login/x",
    "github:lo gin/x",
    " github:login/x",
    "github:login/x ",
    "github:login/ x",
];

/// claims/R53: a malformed entity form is an operation error naming the three forms
#[test]
fn a_malformed_entity_form_is_an_operation_error_naming_the_three_forms() {
    let env = Env::new();
    for bad in BAD_FORMS {
        let (ok, error) = env.run(&["about", bad]);
        assert!(!ok, "{bad}");
        let message = error["error"].as_str().unwrap();
        for part in [FORMS, "URL", "<namespace>:<kind>/<value>"] {
            assert!(message.contains(part), "{bad}: {message}");
        }
    }
}

/// claims/R54: a well-formed form that matches nothing is the empty result with coverage
#[test]
fn a_well_formed_form_that_matches_nothing_is_empty_with_coverage() {
    let env = Env::new();
    env.claim("code:state", literal("open"), time());
    for miss in [
        "gitlab:login/x",
        "github:login/FBisca/x",
        "github:login/a b",
    ] {
        let empty = env.ok(&["about", miss]);
        assert_eq!(empty["claims"], json!([]), "{miss}");
        assert!(empty["coverage"].is_array(), "{miss}");
    }
}

/// claims/R55: real identifiers (fragment URLs, accounts, bot logins) are accepted
#[test]
fn real_identifiers_are_accepted() {
    let env = Env::new();
    let url = "https://github.com/o/p/pull/1#issuecomment-5";
    env.claim_for(url, "code:state", literal("open"), time());
    let account = "github:account/42";
    env.claim_for(
        account,
        "rastro:alias",
        alias("login", "vercel[bot]"),
        time(),
    );
    assert_eq!(
        env.ok(&["about", url])["claims"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        env.ok(&["about", account])["claims"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        env.ok(&["about", "github:login/vercel[bot]"])["claims"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

// ---- Labels: a cited title or name next to each entity of an answer ----

const PR: &str = "https://github.com/o/p/pull/1";
const ISSUE: &str = "https://github.com/o/p/issues/2";
const REPO: &str = "https://github.com/o/p";
const COMMENT: &str = "https://github.com/o/p/pull/1#issuecomment-5";
const ACCOUNT: &str = "github:account/42";
const LINEAR_ISSUE: &str = "linear:issue/0b5a8c1e-0000-4000-8000-0000000000a1";
const LINEAR_USER: &str = "linear:user/0b5a8c1e-0000-4000-8000-000000000001";

fn entity(identifier: &str, entity_type: &str) -> Object {
    let e = Entity::new(identifier.into(), entity_type.into());
    Object::Entity {
        id: e.id,
        identifier: e.identifier,
        entity_type: e.entity_type,
    }
}
impl Env {
    /// Appends one observation of `field` at `location` (no claim cites it); its evidence id.
    fn observe(
        &self,
        location: &str,
        field: &str,
        quote: &str,
        trust_class: TrustClass,
        captured: DateTime<Utc>,
    ) -> i64 {
        self.observe_from("project", location, field, quote, trust_class, captured)
    }
    /// The same, read by the source `name` (an evidence's `captured_at` never goes below the
    /// newest of its own source and scope, so the order of two captures needs two sources).
    fn observe_from(
        &self,
        name: &str,
        location: &str,
        field: &str,
        quote: &str,
        trust_class: TrustClass,
        captured: DateTime<Utc>,
    ) -> i64 {
        let source = Source::github(name.into(), "o/p".into(), Scope("work".into()));
        let mut db = self.db();
        db.project_sources(&[
            Source::github("project".into(), "o/p".into(), Scope("work".into())),
            source.clone(),
        ])
        .unwrap();
        let run = db
            .start_run(&source, ("fixture", "1"), captured, (captured, captured))
            .unwrap();
        match db
            .append_evidence(
                run,
                &Evidence {
                    source: source.name.clone(),
                    scope: source.scope.clone(),
                    location: location.into(),
                    field: field.into(),
                    start: 0,
                    end: quote.len() as u64,
                    quote: quote.into(),
                    trust_class,
                    captured_at: captured,
                    source_updated_at: Some(captured),
                    original: None,
                },
            )
            .unwrap()
        {
            rastro_core::Appended::New(id) | rastro_core::Appended::Unchanged(id) => id,
        }
    }
    fn title(&self, location: &str, text: &str, captured: DateTime<Utc>) -> i64 {
        self.observe(
            location,
            "/title",
            text,
            TrustClass::UntrustedContent,
            captured,
        )
    }
    /// Types `identifier` with a `rastro:type` claim, as an extractor does.
    fn typed(&self, identifier: &str, entity_type: &str) {
        let object = Object::Literal {
            value: entity_type.into(),
            datatype: claims::TYPE_DATATYPE.into(),
        };
        self.claim_for(identifier, claims::TYPE_PREDICATE, object, time());
    }
    fn about(&self, entity: &str) -> Vec<Value> {
        self.ok(&["about", entity, "--limit", "50"])["claims"]
            .as_array()
            .unwrap_or_else(|| panic!("no claims about {entity}"))
            .clone()
    }
    fn text(&self, args: &[&str]) -> String {
        let output = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("claim {id} missing from {rows:?}"))
}
/// A side's label: the key must be present (null or an object), not merely absent.
fn label<'a>(row: &'a Value, side: &str) -> &'a Value {
    row.get(format!("{side}_label"))
        .unwrap_or_else(|| panic!("claim row has no `{side}_label` field: {row}"))
}

/// A PR with a title, two states, a repository, an author and a comment, plus an issue with a
/// title and no state. The repository, the comment and the account carry title- or name-shaped
/// evidence at their own locations, which must still give no label.
struct World {
    title_id: i64,
    issue_title_id: i64,
    state: String,
    in_repo: String,
    author: String,
    comment_author: String,
    issue_author: String,
}
fn world(env: &Env) -> World {
    for (identifier, entity_type) in [
        (PR, "code:PullRequest"),
        (ISSUE, "code:Issue"),
        (REPO, "code:Repository"),
        (COMMENT, "code:Comment"),
        (ACCOUNT, "code:Account"),
    ] {
        env.typed(identifier, entity_type);
    }
    let title_id = env.title(PR, "Fix the login flow", time());
    let issue_title_id = env.title(ISSUE, "Docs are stale", time());
    env.title(REPO, "Repository title", time());
    env.title(COMMENT, "Comment title", time());
    env.observe(
        ACCOUNT,
        "/name",
        "Ana Silva",
        TrustClass::Structured,
        time(),
    );
    env.claim_for(PR, "code:state", literal("open"), time());
    // The current state is the later observation; the earlier one is superseded.
    let state = env.claim_for(PR, "code:state", literal("closed"), later(1));
    let account = entity(ACCOUNT, "code:Account");
    let in_repo = env.claim_for(
        PR,
        "code:in_repository",
        entity(REPO, "code:Repository"),
        time(),
    );
    let author = env.claim_for(PR, "code:author", account.clone(), time());
    let comment_author = env.claim_for(COMMENT, "code:author", account.clone(), time());
    let issue_author = env.claim_for(ISSUE, "code:author", account, time());
    World {
        title_id,
        issue_title_id,
        state,
        in_repo,
        author,
        comment_author,
        issue_author,
    }
}

/// labels/S1: a GitHub issue or PR side carries its `/title` label with every cited field
#[test]
fn S1_a_github_issue_or_pr_side_carries_its_cited_title_label() {
    let env = Env::new();
    let w = world(&env);
    let rows = env.about(PR);
    for id in [&w.state, &w.in_repo, &w.author] {
        let row = row(&rows, id);
        let label = label(row, "subject");
        assert!(label.is_object(), "subject_label is not an object: {row}");
        assert_eq!(label["text"], "Fix the login flow", "{row}");
        assert_eq!(label["url"], PR, "{row}");
        assert_eq!(label["evidence_id"], json!(w.title_id), "{row}");
        assert_eq!(label["field"], "/title", "{row}");
        assert_eq!(
            label["captured_at"]
                .as_str()
                .unwrap()
                .parse::<DateTime<Utc>>()
                .unwrap(),
            time()
        );
        assert_eq!(label["trust_class"], "untrusted content", "{row}");
        assert_eq!(label["clipped"], false, "{row}");
    }
    // The object side of an incoming claim is labelled too.
    let incoming = env.about(ACCOUNT);
    let side = label(row(&incoming, &w.issue_author), "subject");
    assert_eq!(side["text"], "Docs are stale", "{side}");
    assert_eq!(side["evidence_id"], json!(w.issue_title_id));
    assert_eq!(side["url"], ISSUE);
    // A literal object has no label.
    assert_eq!(*label(row(&rows, &w.state), "object"), Value::Null);
}

/// labels/S2: why gives its subject and object the labels about gives
#[test]
fn S2_why_carries_the_same_labels_as_about() {
    let env = Env::new();
    let w = world(&env);
    let rows = env.about(PR);
    let in_repo = row(&rows, &w.in_repo);
    let author = row(&rows, &w.author);
    for claim in [&w.state, &w.in_repo, &w.author] {
        let why = env.ok(&["why", claim]);
        let about = row(&rows, claim);
        assert_eq!(label(&why["claim"], "subject"), label(about, "subject"));
        assert_eq!(label(&why["claim"], "object"), label(about, "object"));
        assert!(label(&why["claim"], "subject").is_object(), "{why}");
    }
    assert_eq!(*label(in_repo, "object"), Value::Null);
    assert_eq!(*label(author, "object"), Value::Null);
    // An issue as the object of a claim is labelled in why too.
    let account_rows = env.about(ACCOUNT);
    let why = env.ok(&["why", &w.issue_author]);
    assert_eq!(
        label(&why["claim"], "subject"),
        label(row(&account_rows, &w.issue_author), "subject")
    );
    assert_eq!(label(&why["claim"], "subject")["text"], "Docs are stale");
}

/// labels/S3: a Linear issue's label is its `/title` with no url; a Linear user's is its `/name`
#[test]
fn S3_a_linear_issue_and_a_linear_user_are_labelled_from_title_and_name() {
    let env = Env::new();
    env.typed(LINEAR_ISSUE, "tracker:Issue");
    env.typed(LINEAR_USER, "tracker:Account");
    let title_id = env.title(LINEAR_ISSUE, "Ship the labels", time());
    let name_id = env.observe(
        LINEAR_USER,
        "/name",
        "Ana Silva",
        TrustClass::Structured,
        time(),
    );
    env.observe(
        LINEAR_USER,
        "/displayName",
        "ana",
        TrustClass::Structured,
        time(),
    );
    let creator = env.claim_for(
        LINEAR_ISSUE,
        "tracker:creator",
        entity(LINEAR_USER, "tracker:Account"),
        time(),
    );
    let rows = env.about(LINEAR_ISSUE);
    let row = row(&rows, &creator);
    let issue = label(row, "subject");
    assert_eq!(issue["text"], "Ship the labels", "{row}");
    assert_eq!(issue["field"], "/title");
    assert_eq!(issue["evidence_id"], json!(title_id));
    assert_eq!(issue["url"], Value::Null, "{issue}");
    assert_eq!(issue["trust_class"], "untrusted content");
    let user = label(row, "object");
    assert_eq!(user["text"], "Ana Silva", "{row}");
    assert_eq!(user["field"], "/name");
    assert_eq!(user["evidence_id"], json!(name_id));
    assert_eq!(user["url"], Value::Null);
    assert_eq!(user["trust_class"], "structured");
}

/// labels/S4: a repository, a comment, a GitHub account and a Person have a null label
#[test]
fn S4_repository_comment_account_and_person_sides_have_a_null_label() {
    let env = Env::new();
    let w = world(&env);
    let rows = env.about(PR);
    assert_eq!(*label(row(&rows, &w.in_repo), "object"), Value::Null);
    assert_eq!(*label(row(&rows, &w.author), "object"), Value::Null);
    let incoming = env.about(ACCOUNT);
    let comment = row(&incoming, &w.comment_author);
    assert_eq!(*label(comment, "subject"), Value::Null, "{comment}");
    assert_eq!(*label(comment, "object"), Value::Null, "{comment}");
    // Not a blanket null: the issue side of the same answer is labelled.
    assert!(label(row(&incoming, &w.issue_author), "subject").is_object());
    // A Person, even with title- and name-shaped evidence at its location.
    let person = env.ok(&["identity", "person", "create", "--name", "Ana"])["person"].clone();
    let identifier = person["identifier"].as_str().unwrap();
    env.title(identifier, "A title", time());
    env.observe(identifier, "/name", "Ana", TrustClass::Structured, time());
    let own = env.about(identifier);
    assert!(!own.is_empty());
    for claim in &own {
        assert_eq!(*label(claim, "subject"), Value::Null, "{claim}");
        assert_eq!(*label(claim, "object"), Value::Null, "{claim}");
    }
}

/// labels/S5: a long title is cut to 200 whole chars across a multibyte char, and flagged
#[test]
fn S5_a_title_over_200_chars_is_cut_on_a_char_boundary_and_flagged() {
    let env = Env::new();
    env.typed(PR, "code:PullRequest");
    // 199 one-byte chars, then 3-byte chars: byte 200 falls inside the first of them.
    let title = format!("{}{}", "a".repeat(199), "€".repeat(50));
    env.title(PR, &title, time());
    env.claim_for(PR, "code:state", literal("open"), time());
    let rows = env.about(PR);
    let label = label(&rows[0], "subject");
    let text = label["text"].as_str().unwrap();
    assert_eq!(text.chars().count(), 200, "{label}");
    assert_eq!(text, title.chars().take(200).collect::<String>());
    assert_eq!(label["clipped"], true);
    // Exactly 200 chars is not clipped.
    let exact = "é".repeat(200);
    env.title(ISSUE, &exact, time());
    env.typed(ISSUE, "code:Issue");
    env.claim_for(ISSUE, "code:state", literal("open"), time());
    let rows = env.about(ISSUE);
    let label = self::label(&rows[0], "subject");
    assert_eq!(label["text"], exact.as_str());
    assert_eq!(label["clipped"], false);
}

/// labels/S6: the latest `captured_at` wins; a tie goes to the higher evidence id
#[test]
fn S6_the_latest_observation_wins_and_a_tie_goes_to_the_higher_evidence_id() {
    let env = Env::new();
    env.typed(PR, "code:PullRequest");
    env.claim_for(PR, "code:state", literal("open"), time());
    let title = |source: &str, text: &str, captured| {
        env.observe_from(
            source,
            PR,
            "/title",
            text,
            TrustClass::UntrustedContent,
            captured,
        )
    };
    let newest = title("project", "Newest by time", later(1));
    // Stored after, but captured earlier: it must not win.
    title("second", "Older but stored last", time());
    let rows = env.about(PR);
    let found = label(&rows[0], "subject");
    assert_eq!(found["text"], "Newest by time");
    assert_eq!(found["evidence_id"], json!(newest));
    // Two observations captured at the same instant: the higher evidence id.
    let tie = later(5);
    title("project", "Tie, first", tie);
    let second = title("second", "Tie, second", tie);
    let rows = env.about(PR);
    let found = label(&rows[0], "subject");
    assert_eq!(found["text"], "Tie, second");
    assert_eq!(found["evidence_id"], json!(second));
    // And within one source, a later capture is the higher id and the later time.
    let third = title("project", "Tie, third", tie);
    let rows = env.about(PR);
    assert_eq!(label(&rows[0], "subject")["evidence_id"], json!(third));
}

/// labels/S8: adding labels leaves claims, supports and revisions as they were, and a title's
/// text is not an alias of the entity
#[test]
fn S8_labels_change_no_claim_support_or_revision_and_a_title_is_not_an_alias() {
    let env = Env::new();
    env.typed(PR, "code:PullRequest");
    let state = env.claim_for(PR, "code:state", literal("open"), time());
    let strip = |rows: Vec<Value>| -> Vec<Value> {
        rows.into_iter()
            .map(|mut r| {
                let r_object = r.as_object_mut().unwrap();
                r_object.remove("subject_label");
                r_object.remove("object_label");
                r
            })
            .collect()
    };
    let before = strip(env.about(PR));
    // The claim's own answer: its supports and revision, without the labels.
    let why = |env: &Env| {
        let mut why = env.ok(&["why", &state, "--full"]);
        let claim = why["claim"].as_object_mut().unwrap();
        claim.remove("subject_label");
        claim.remove("object_label");
        json!([why["claim"], why["supports"], why["truncated"]])
    };
    let before_why = why(&env);
    // A title shaped like a canonical alias.
    env.title(PR, "github:title/Fix", time());
    let after_rows = env.about(PR);
    assert!(label(&after_rows[0], "subject").is_object());
    assert_eq!(strip(after_rows), before);
    assert_eq!(why(&env), before_why);
    assert_eq!(
        env.ok(&["why", &state])["claim"]["subject_label"]["text"],
        "github:title/Fix"
    );
    let resolved = env.ok(&["about", "github:title/Fix"]);
    assert_eq!(resolved["resolved"], false, "{resolved}");
    assert_eq!(resolved["claims"], json!([]));
}

/// labels/S9: `label.state` holds the current observed state, `code:state` or `tracker:state`,
/// and is null with none
#[test]
fn S9_label_state_holds_the_current_observed_state_or_null() {
    let env = Env::new();
    let w = world(&env);
    env.typed(LINEAR_ISSUE, "tracker:Issue");
    env.title(LINEAR_ISSUE, "Ship the labels", time());
    env.claim_for(LINEAR_ISSUE, "tracker:state", literal("Todo"), time());
    env.claim_for(
        LINEAR_ISSUE,
        "tracker:state",
        literal("In Progress"),
        later(1),
    );
    let rows = env.about(PR);
    let pr = label(row(&rows, &w.state), "subject");
    assert_eq!(
        pr["state"], "closed",
        "the current state, not the superseded one: {pr}"
    );
    let incoming = env.about(ACCOUNT);
    let issue = label(row(&incoming, &w.issue_author), "subject");
    assert!(issue.is_object());
    assert_eq!(issue.get("state"), Some(&Value::Null), "{issue}");
    let linear = env.about(LINEAR_ISSUE);
    let current = linear
        .iter()
        .find(|r| r["predicate"] == "tracker:state" && r["status"] == "observed")
        .unwrap();
    assert_eq!(label(current, "subject")["state"], "In Progress");
}

/// labels/S10: the CLI text shows the label after the id with control chars escaped
#[test]
fn S10_cli_text_shows_the_escaped_label_after_the_id_for_about_and_why() {
    let env = Env::new();
    env.title(PR, "Login \u{1b}[31mfails\nbadly", time());
    env.typed(PR, "code:PullRequest");
    let state = env.claim_for(PR, "code:state", literal("open"), time());
    let alias = env.claim_for(PR, "rastro:alias", alias("pr", "o/p/1"), time());
    for text in [env.text(&["about", PR]), env.text(&["why", &state])] {
        assert!(
            !text.contains('\u{1b}'),
            "a raw escape reached the terminal: {text:?}"
        );
        let line = text
            .lines()
            .find(|l| l.contains(&state))
            .unwrap_or_else(|| panic!("no line for claim {state}: {text}"));
        let id_at = line.find(PR).unwrap_or_else(|| panic!("no id: {line}"));
        let label_at = line
            .find("Login ")
            .unwrap_or_else(|| panic!("no label text on the claim line: {line}"));
        assert!(label_at > id_at, "the label is not after the id: {line}");
        for part in ["fails", "badly", "\\u{1b}", "[open]"] {
            assert!(line.contains(part), "missing {part:?} on the line: {line}");
        }
    }
    // The alias list stays between the id and the label.
    let text = env.text(&["about", PR]);
    let line = text.lines().find(|l| l.contains(&alias)).unwrap();
    assert!(
        line.find("o/p/1").unwrap() < line.find("Login ").unwrap(),
        "{line}"
    );
}

/// claims: `about --state` keeps only claims with a side whose label state matches, without
/// case, and filters before `limit`
#[test]
fn about_state_keeps_claims_whose_side_has_that_state_before_the_limit() {
    let env = Env::new();
    let w = world(&env);
    env.claim_for(ISSUE, "code:state", literal("open"), time());
    let ids = |result: &Value| -> Vec<String> {
        result["claims"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    };
    let open = env.ok(&["about", ACCOUNT, "--state", "OPEN"]);
    assert_eq!(ids(&open), vec![w.issue_author.clone()], "{open}");
    let either = env.ok(&["about", ACCOUNT, "--state", "open", "--state", "Closed"]);
    let mut both = ids(&either);
    both.sort();
    let mut expected = vec![w.issue_author.clone(), w.author.clone()];
    expected.sort();
    assert_eq!(both, expected, "{either}");
    // One claim matches, and it fits the limit of one: nothing else was dropped for it.
    let closed = env.ok(&["about", ACCOUNT, "--state", "closed", "--limit", "1"]);
    assert_eq!(ids(&closed), vec![w.author.clone()], "{closed}");
    assert_eq!(closed["truncated"], false, "{closed}");
    let none = env.ok(&["about", ACCOUNT, "--state", "merged"]);
    assert!(ids(&none).is_empty(), "{none}");
    // Without the filter every claim stays.
    assert!(
        env.ok(&["about", ACCOUNT])["claims"]
            .as_array()
            .unwrap()
            .len()
            >= 3
    );
}
