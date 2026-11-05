#![allow(non_snake_case)]
//! Owner identity end to end: `rastro identity` writes intents, the projection shows the claims
//! they back, `about` and `why` read them, and a rebuild brings them back unchanged.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{collections::BTreeSet, process::Command};
use tempfile::TempDir;

/// The extractor a fixture source is bound to; it is never installed, so a rebuild carries the
/// source's pair over.
const MISSING: &str = "missing-extractor";

fn time() -> DateTime<Utc> {
    "2026-11-01T00:00:00Z".parse().unwrap()
}
struct Env {
    home: TempDir,
}
impl Env {
    fn new() -> Self {
        Self {
            home: tempfile::tempdir().unwrap(),
        }
    }
    fn run(&self, args: &[&str]) -> (bool, Value) {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .env("RASTRO_PLUGIN_PATH", self.home.path().join("no-plugins"))
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/operations-v2.schema.json"))
                .unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&value)
            .unwrap_or_else(|error| panic!("{error}: {value}"));
        (out.status.success(), value)
    }
    fn ok(&self, args: &[&str]) -> Value {
        let (ok, value) = self.run(args);
        assert!(ok, "{args:?}: {value}");
        value["result"].clone()
    }
    /// The operation error a refused command gives.
    fn refused(&self, args: &[&str]) -> String {
        let (ok, value) = self.run(args);
        assert!(!ok, "{args:?} should be refused: {value}");
        value["error"].as_str().unwrap().to_string()
    }
    fn human(&self, args: &[&str]) -> String {
        let out = Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn path(&self) -> std::path::PathBuf {
        self.home.path().join("data/rastro.sqlite3")
    }
    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.path()).unwrap()
    }
    fn sql<T: rusqlite::types::FromSql>(&self, query: &str) -> T {
        Connection::open(self.path())
            .unwrap()
            .query_row(query, [], |row| row.get(0))
            .unwrap()
    }
    fn intents(&self) -> i64 {
        self.sql("SELECT count(*) FROM intent")
    }
    /// A GitHub-shaped account `github:account/<n>`, typed `code:Account`, with a login alias,
    /// observed by source `project` (scope `personal`), bound to an extractor that isn't
    /// installed.
    fn account(&self, n: u32, login: &str) -> String {
        let source = Source::github("project".into(), "o/p".into(), Scope("personal".into()));
        let mut db = self.db();
        db.project_sources(std::slice::from_ref(&source)).unwrap();
        db.bind_extractor(&source.name, Some(MISSING)).unwrap();
        let run = db
            .start_run(&source, (MISSING, "1"), time(), (time(), time()))
            .unwrap();
        let location = format!("https://api.github.com/users/{n}");
        let subject = Entity::new(format!("github:account/{n}"), "code:Account".into());
        let mut observed = Vec::new();
        for (field, quote, predicate, object) in [
            (
                "/type",
                "User",
                claims::TYPE_PREDICATE,
                Object::Literal {
                    value: "code:Account".into(),
                    datatype: claims::TYPE_DATATYPE.into(),
                },
            ),
            (
                "/login",
                login,
                claims::ALIAS_PREDICATE,
                Object::Literal {
                    value: login.into(),
                    datatype: "login".into(),
                },
            ),
        ] {
            let e = Evidence {
                source: source.name.clone(),
                scope: source.scope.clone(),
                location: location.clone(),
                field: field.into(),
                start: 0,
                end: quote.len() as u64,
                quote: quote.into(),
                trust_class: TrustClass::Structured,
                captured_at: time(),
                source_updated_at: Some(time()),
                original: None,
            };
            db.append_evidence(run, &e).unwrap();
            let (evidence_id, _) = db
                .pending_evidence((MISSING, "1"), &source)
                .unwrap()
                .into_iter()
                .rev()
                .find(|(_, item)| item.location == location && item.field == field)
                .unwrap();
            observed.push(ObservedClaim {
                id: claims::claim_id(&subject.id, predicate, &object),
                subject: subject.clone(),
                predicate: predicate.into(),
                object,
                scalar: true,
                type_ancestors: Vec::new(),
                supports: BTreeSet::from([Support {
                    portable_key: claims::portable_evidence_key(&e),
                    evidence_id,
                    field: field.into(),
                    start: 0,
                    end: e.end,
                    extraction_rule: "fixture-rule".into(),
                }]),
            });
        }
        db.record_claims(run, (MISSING, "1"), &observed, None)
            .unwrap();
        format!("github:login/{login}")
    }
    /// A new named Person; returns its canonical identifier.
    fn person(&self, name: &str) -> String {
        let created = self.ok(&["identity", "person", "create", "--name", name]);
        created["person"]["identifier"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn about(&self, entity: &str) -> Value {
        self.ok(&["about", entity])
    }
}
/// The (predicate, subject identifier, object) of every claim an `about` shows, sorted.
fn statements(about: &Value) -> Vec<(String, String, String)> {
    let mut found: Vec<_> = about["claims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let object = match c["object"]["kind"].as_str().unwrap() {
                "entity" => c["object"]["identifier"].as_str().unwrap().to_string(),
                _ => c["object"]["value"].as_str().unwrap().to_string(),
            };
            (
                c["predicate"].as_str().unwrap().to_string(),
                c["subject"]["identifier"].as_str().unwrap().to_string(),
                object,
            )
        })
        .collect();
    found.sort();
    found
}
fn claim_id(about: &Value, predicate: &str) -> String {
    about["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["predicate"] == predicate)
        .unwrap_or_else(|| panic!("no {predicate}: {about}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}
fn kinds(history: &Value) -> Vec<(String, String)> {
    history["intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["kind"].as_str().unwrap().to_string(),
                i["status"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// S3: `identity me` mints the owner's Person, `about me` resolves to it, and a second run
/// returns the same Person and records nothing
#[test]
fn S3_identity_me_mints_one_person_that_about_me_resolves() {
    let env = Env::new();
    let first = env.ok(&["identity", "me"]);
    let me = first["person"]["identifier"].as_str().unwrap().to_string();
    assert!(me.starts_with("rastro:person/"), "{first}");
    assert_eq!(first["created"], true);
    assert_eq!(first["intents"].as_array().unwrap().len(), 1, "{first}");
    assert_eq!(first["intents"][0]["kind"], "create_person");
    assert_eq!(env.intents(), 1);
    let about = env.about("me");
    assert_eq!(
        statements(&about),
        [(
            "rastro:type".to_string(),
            me.clone(),
            "rastro:Person".to_string()
        )]
    );
    assert_eq!(about["claims"][0]["subject"]["type"], "rastro:Person");
    assert_eq!(env.about(&me)["claims"], about["claims"]);
    let second = env.ok(&["identity", "me"]);
    assert_eq!(second["person"], first["person"]);
    assert_eq!(second["created"], false);
    assert_eq!(second["intents"], json!([]));
    assert_eq!(env.intents(), 1);
    // Before any `me`, `about me` is the answer for an unknown entity.
    let fresh = Env::new();
    fresh.account(1, "octocat");
    assert_eq!(fresh.about("me")["claims"], json!([]));
}

/// S4: two Persons created with one name are two Persons, and the name is an ambiguous alias
#[test]
fn S4_two_persons_with_one_name_make_the_name_ambiguous() {
    let env = Env::new();
    let ana = env.person("Ana Silva");
    let other = env.person("Ana Silva");
    assert_ne!(ana, other);
    for person in [&ana, &other] {
        let about = env.about(person);
        assert!(
            statements(&about).contains(&(
                "rastro:alias".into(),
                person.to_string(),
                "Ana Silva".into()
            )),
            "{about}"
        );
    }
    let error = env.refused(&["about", "rastro:name/Ana Silva"]);
    assert!(error.contains("ambiguous alias"), "{error}");
    for person in [&ana, &other] {
        assert!(error.contains(person.as_str()), "{error}");
    }
    assert_eq!(env.intents(), 4);
}

/// S5: a linked account shows `rastro:held_by` the Person, and `why` on the link shows an
/// observed claim justified by the owner intent: its id, time and issuer
#[test]
fn S5_a_link_shows_held_by_and_why_cites_the_owner_intent() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let ana = env.person("Ana Silva");
    let linked = env.ok(&["identity", "link", &octocat, &ana]);
    let intent = &linked["intents"][0];
    assert_eq!(intent["kind"], "link", "{linked}");
    let about = env.about(&octocat);
    assert!(
        statements(&about).contains(&(
            "rastro:held_by".into(),
            "github:account/7".into(),
            ana.clone()
        )),
        "{about}"
    );
    let link = claim_id(&about, "rastro:held_by");
    let why = env.ok(&["why", &link]);
    assert_eq!(why["claim"]["status"], "observed");
    assert_eq!(why["claim"]["object"]["type"], "rastro:Person");
    let supports = why["supports"].as_array().unwrap();
    assert_eq!(supports.len(), 1, "{why}");
    let support = &supports[0];
    assert_eq!(support["justification_kind"], "owner_intent");
    assert_eq!(support["current"], true);
    assert_eq!(support["intent"]["id"], intent["id"]);
    assert_eq!(support["intent"]["issued_at"], intent["issued_at"]);
    assert_eq!(
        support["intent"]["issuer"],
        json!({"principal": "owner", "surface": "cli", "scopes": []})
    );
    assert_eq!(
        why["claim"]["known_intervals"][0]["start"],
        intent["issued_at"]
    );
    let human = env.human(&["why", &link]);
    assert!(
        human.contains("owner_intent") && human.contains(intent["id"].as_str().unwrap()),
        "{human}"
    );
    // Linking the same pair again records nothing.
    let again = env.ok(&["identity", "link", &octocat, &ana]);
    assert_eq!(again["intents"], json!([]));
    assert_eq!(env.intents(), 3);
}

/// S6: undoing a link removes `held_by`, records a cannot-link, and the history lists the link,
/// the undo and the cannot-link in acceptance order
#[test]
fn S6_undoing_a_link_removes_it_and_records_a_cannot_link_in_history() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let ana = env.person("Ana Silva");
    let link = env.ok(&["identity", "link", &octocat, &ana])["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let undone = env.ok(&["identity", "undo", &link]);
    let written: Vec<_> = undone["intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(written, ["undo", "cannot_link"], "{undone}");
    assert_eq!(undone["intents"][0]["undo_of"], link.as_str());
    let about = env.about(&octocat);
    assert!(
        !statements(&about)
            .iter()
            .any(|(predicate, ..)| predicate == "rastro:held_by"),
        "{about}"
    );
    assert!(
        !statements(&env.about(&ana))
            .iter()
            .any(|(p, ..)| p == "rastro:held_by")
    );
    let history = env.ok(&["identity", "history", &octocat]);
    assert_eq!(history["entity"]["identifier"], "github:account/7");
    assert_eq!(
        kinds(&history),
        [
            ("link".to_string(), "undone".to_string()),
            ("undo".into(), "effective".into()),
            ("cannot_link".into(), "effective".into()),
        ]
    );
    let order: Vec<_> = history["intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["append_seq"].as_i64().unwrap())
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{history}");
    // The Person's history holds the same three after its creation and name.
    let person = env.ok(&["identity", "history", &ana]);
    assert_eq!(person["intents"].as_array().unwrap().len(), 5, "{person}");
    // Linking the pair again is the owner's explicit link: it clears the cannot-link, which
    // stays in the history, marked reversed.
    env.ok(&["identity", "link", &octocat, &ana]);
    assert!(
        statements(&env.about(&octocat))
            .iter()
            .any(|(p, ..)| p == "rastro:held_by")
    );
    let history = env.ok(&["identity", "history", &octocat]);
    assert_eq!(
        kinds(&history)[2..],
        [
            ("cannot_link".to_string(), "reversed".to_string()),
            ("link".into(), "effective".into()),
        ]
    );
}

/// S7: a merge shows both Persons' claims on either one, equivalent claims grouped; undoing it
/// separates them and records a cannot-link
#[test]
fn S7_a_merge_groups_both_persons_claims_and_its_undo_separates_them() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let hubot = env.account(8, "hubot");
    let ana = env.person("Ana");
    let silva = env.person("Ana Silva");
    env.ok(&["identity", "link", &octocat, &ana]);
    env.ok(&["identity", "link", &hubot, &silva]);
    env.ok(&["identity", "link", &octocat, &silva]);
    let alone = statements(&env.about(&ana));
    let merged = env.ok(&["identity", "merge", &ana, &silva]);
    let merge = merged["intents"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(merged["intents"][0]["kind"], "same_as");
    let (on_ana, on_silva) = (env.about(&ana), env.about(&silva));
    assert_eq!(statements(&on_ana), statements(&on_silva));
    let shown = statements(&on_ana);
    let (a, s) = (ana.as_str(), silva.as_str());
    for (predicate, subject, object) in [
        ("rastro:type", a, "rastro:Person"),
        ("rastro:type", s, "rastro:Person"),
        ("rastro:alias", a, "Ana"),
        ("rastro:alias", s, "Ana Silva"),
        ("rastro:held_by", "github:account/7", a),
        ("rastro:held_by", "github:account/7", s),
        ("rastro:held_by", "github:account/8", s),
    ] {
        assert!(
            shown.contains(&(predicate.into(), subject.to_string(), object.to_string())),
            "{predicate} {subject} {object}: {on_ana}"
        );
    }
    assert_eq!(shown.len(), 8, "{on_ana}");
    // The two type claims are one group, and so are the two links of octocat; each member keeps
    // its own id, status and revision, and a group counts once toward `limit`.
    let groups = on_ana["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2, "{on_ana}");
    let by_id = |id: &Value| {
        on_ana["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == *id)
            .unwrap()
            .clone()
    };
    let mut grouped: Vec<_> = groups
        .iter()
        .map(|g| {
            assert_eq!(g["status"], "observed");
            assert!(g["revision"].as_str().unwrap().starts_with("g_"));
            let members = g["members"].as_array().unwrap();
            assert_eq!(members.len(), 2, "{g}");
            for m in members {
                let claim = by_id(&m["claim"]);
                assert_eq!(m["revision"], claim["revision"]);
                assert_eq!(m["status"], claim["status"]);
            }
            by_id(&members[0]["claim"])["predicate"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    grouped.sort();
    assert_eq!(grouped, ["rastro:held_by", "rastro:type"]);
    let items = |about: &Value| {
        let grouped: usize = about["groups"].as_array().map_or(0, |groups| {
            groups
                .iter()
                .map(|g| g["members"].as_array().unwrap().len() - 1)
                .sum()
        });
        about["claims"].as_array().unwrap().len() - grouped
    };
    assert_eq!(items(&on_ana), 6);
    let limited = env.ok(&["about", &ana, "--limit", "5"]);
    assert_eq!(limited["truncated"], true, "{limited}");
    assert_eq!(items(&limited), 5, "{limited}");
    let six = env.ok(&["about", &ana, "--limit", "6"]);
    assert_eq!(six["truncated"], false, "{six}");
    assert_eq!(statements(&six), shown);
    // Undo: they separate, and the pair can't be merged again by anything but the owner.
    let undone = env.ok(&["identity", "undo", &merge]);
    assert_eq!(undone["intents"][1]["kind"], "cannot_link", "{undone}");
    let after = env.about(&ana);
    assert_eq!(statements(&after), alone);
    assert!(after.get("groups").is_none(), "{after}");
    let history = env.ok(&["identity", "history", &silva]);
    assert_eq!(
        kinds(&history).last().unwrap(),
        &("cannot_link".to_string(), "effective".to_string())
    );
}

/// S8: invalid intents are refused with an operation error naming the reason, and nothing is
/// stored
#[test]
fn S8_invalid_intents_are_refused_and_store_nothing() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let hubot = env.account(8, "hubot");
    let ana = env.person("Ana");
    let silva = env.person("Ana Silva");
    let link = env.ok(&["identity", "link", &octocat, &ana])["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let undo = env.ok(&["identity", "undo", &link])["intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let before = env.intents();
    let missing_intent = format!("i_{}", "0".repeat(32));
    let (octocat, hubot, ana, silva, link) = (
        octocat.as_str(),
        hubot.as_str(),
        ana.as_str(),
        silva.as_str(),
        link.as_str(),
    );
    let (undo, cannot, missing_intent) =
        (undo[0].as_str(), undo[1].as_str(), missing_intent.as_str());
    for (args, reason) in [
        // A `held_by` from a Person.
        (vec!["identity", "link", ana, silva], "is a Person"),
        // A holder that isn't a Person.
        (vec!["identity", "link", octocat, hubot], "is not a Person"),
        // A merge of two accounts, of an account and a Person, of an entity with itself.
        (
            vec!["identity", "merge", octocat, hubot],
            "accounts never merge",
        ),
        (
            vec!["identity", "merge", octocat, ana],
            "only Persons merge",
        ),
        (vec!["identity", "merge", ana, ana], "itself"),
        // Targets that don't exist.
        (
            vec!["identity", "link", "github:login/nobody", ana],
            "unknown entity",
        ),
        (
            vec!["identity", "merge", ana, "rastro:person/missing"],
            "unknown entity",
        ),
        (vec!["identity", "link", "not a form", ana], "must be"),
        // An undo of a missing intent, of an undone one, of a cannot-link, of an undo.
        (vec!["identity", "undo", missing_intent], "unknown intent"),
        (vec!["identity", "undo", link], "already undone"),
        (
            vec!["identity", "undo", cannot],
            "cannot-link can't be undone",
        ),
        (vec!["identity", "undo", undo], "an undo can't be undone"),
        // A name that isn't one.
        (
            vec!["identity", "person", "create", "--name", " Ana"],
            "name",
        ),
    ] {
        let error = env.refused(&args);
        assert!(error.contains(reason), "{args:?}: {error}");
        assert_eq!(env.intents(), before, "{args:?}");
    }
}

/// Every projection row a rebuild must reproduce, and the intent table, as stored.
fn rows(env: &Env) -> (Vec<String>, Vec<String>) {
    let db = Connection::open(env.path()).unwrap();
    let dump = |query: &str| -> Vec<String> {
        let mut statement = db.prepare(query).unwrap();
        let width = statement.column_count();
        statement
            .query_map([], |row| {
                (0..width)
                    .map(|i| {
                        row.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(|cells| cells.join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    let mut projection = Vec::new();
    for query in [
        "SELECT 'claim', * FROM claim ORDER BY id",
        "SELECT 'intent_justification', * FROM intent_justification ORDER BY id",
        "SELECT 'justification', id, claim_id, extractor, current FROM justification ORDER BY id",
        "SELECT 'span', justification_id, evidence_id, field FROM justification_span ORDER BY 2, 3, 4",
        "SELECT 'occurrence', id, justification_id, evidence_id FROM occurrence ORDER BY 2",
        "SELECT 'link', * FROM claim_link ORDER BY 2, 3, 4",
        "SELECT 'entity', * FROM entity ORDER BY id",
    ] {
        projection.extend(dump(query));
    }
    (projection, dump("SELECT * FROM intent ORDER BY append_seq"))
}

/// S10: a rebuild, with the evidence's extractor missing so its pair is carried over, brings
/// back every intent-backed Person, name, link and merge identically, and leaves the intent
/// table as it was
#[test]
fn S10_a_rebuild_with_a_carried_pair_brings_intent_backed_claims_back_identically() {
    let env = Env::new();
    let octocat = env.account(7, "octocat");
    let hubot = env.account(8, "hubot");
    env.ok(&["identity", "me"]);
    let ana = env.person("Ana");
    let silva = env.person("Ana Silva");
    env.ok(&["identity", "link", &octocat, "me"]);
    env.ok(&["identity", "link", &hubot, &silva]);
    env.ok(&["identity", "merge", &ana, &silva]);
    let undone = env.ok(&["identity", "link", &hubot, &ana])["intents"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    env.ok(&["identity", "undo", &undone]);
    let before = rows(&env);
    assert!(
        before
            .0
            .iter()
            .filter(|r| r.starts_with("Text(\"intent_justification\")"))
            .count()
            == 8,
        "{before:?}"
    );
    let about = (env.about("me"), env.about(&ana), env.about(&hubot));
    let rebuilt = env.ok(&["claims", "rebuild"]);
    let pairs = rebuilt["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 1, "{rebuilt}");
    assert_eq!(pairs[0]["state"], "not_installed", "{rebuilt}");
    let claims: i64 = env.sql("SELECT count(*) FROM claim");
    assert_eq!(rebuilt["claims"], claims);
    assert_eq!(rows(&env), before);
    assert_eq!((env.about("me"), env.about(&ana), env.about(&hubot)), about);
    // A second rebuild changes nothing either.
    env.ok(&["claims", "rebuild"]);
    assert_eq!(rows(&env), before);
}

/// S7: in a merge chain of three Persons, each merge stays its own claim: only claims that say
/// the same thing group, never two different merges
#[test]
fn S7_a_merge_chain_groups_equivalent_claims_but_never_two_merges() {
    let env = Env::new();
    let (a, b, c) = (env.person("A"), env.person("B"), env.person("C"));
    env.ok(&["identity", "merge", &a, &b]);
    env.ok(&["identity", "merge", &b, &c]);
    let about = env.about(&a);
    let merges = statements(&about)
        .iter()
        .filter(|(p, ..)| p == "rastro:same_as")
        .count();
    assert_eq!(merges, 2, "{about}");
    let groups = about["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "{about}");
    let members = groups[0]["members"].as_array().unwrap();
    assert_eq!(members.len(), 3, "{about}");
    for member in members {
        let claim = about["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == member["claim"])
            .unwrap();
        assert_eq!(claim["predicate"], "rastro:type", "{about}");
    }
    assert_eq!(statements(&env.about(&c)), statements(&about));
}
