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
        db.append_evidence(run, &e).unwrap();
        let evidence_id = db
            .pending_evidence(("fixture", "1"), &source)
            .unwrap()
            .into_iter()
            .find(|(_, item)| {
                item.location == e.location
                    && item.field == e.field
                    && item.quote == e.quote
                    && item.captured_at == captured
            })
            .unwrap()
            .0;
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
                scalar: predicate == "code:state" || predicate == "rastro:alias",
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
    for bad in ["me", "felippe", "o/p", "https://github.com/o p/pull/1"] {
        let (ok, error) = env.run(&["about", bad]);
        assert!(!ok, "{error}");
        let text = error["error"].as_str().unwrap();
        assert!(
            text.contains("entity id")
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
        "github:login/",
        "github:FBisca",
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

/// claims/R31: an old login still resolves to its account
#[test]
fn S1_an_old_login_resolves_to_its_account() {
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

/// claims/R31: a login held by several entities names every candidate with its mark
#[test]
fn S2_an_ambiguous_login_marks_each_candidate() {
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

/// claims/R31: resolving through an old login lists current aliases; why shows the old interval
#[test]
fn S3_resolving_through_an_old_login_lists_current_aliases_only() {
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

/// claims/R31: one entity holding a login both current and historical resolves directly
#[test]
fn S2_an_entity_that_regained_a_login_resolves_directly() {
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
