//! The checks an action proposal passes before it waits in the queue: a citation must be visible
//! in the scope of the source the action writes to, and an open observed Linear issue with the
//! proposal's exact title is named on the queue item as a possible duplicate. The provider is a
//! shell script, and the observed claims and Linear issues are seeded straight into the store,
//! so no outside system is ever called.
mod support;
use chrono::{DateTime, Utc};
use rastro_core::claims::{self, Entity, Object, ObservedClaim, Support};
use rastro_core::{Evidence, Scope, Source, Storage, TrustClass};
use rastro_storage_sqlite::SqliteStorage;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn time() -> DateTime<Utc> {
    "2026-11-18T00:00:00Z".parse().unwrap()
}

struct Env {
    home: TempDir,
}

impl Env {
    /// A home with the fake provider installed, the Linear source `linear-work` in scope `work`
    /// and `[actions.linear] default_team`.
    fn new() -> Self {
        let env = Self {
            home: tempfile::tempdir().unwrap(),
        };
        let config = env.home.path().join("config");
        support::provider::install(&config, |_| {});
        fs::write(
            config.join("config.toml"),
            "[[sources]]\nname = \"linear-work\"\nconnector = \"linear\"\nselector = {}\n\
             scope = \"work\"\nfreshness_rule = \"1h\"\nwindow_start = \"2026-08-01T00:00:00Z\"\n\
             keep_original = false\n\n[actions.linear]\ndefault_team = \"ENG\"\n",
        )
        .unwrap();
        env
    }

    fn rastro(&self, args: &[&str]) -> Output {
        Command::new(support::rastro())
            .env("RASTRO_HOME", self.home.path())
            .args(args)
            .output()
            .unwrap()
    }

    fn propose(&self, title: &str, extra: &[&str]) -> Output {
        let mut args = vec![
            "actions",
            "propose",
            "linear.create_issue",
            "--title",
            title,
            "--why",
            "owner asked in session",
        ];
        args.extend_from_slice(extra);
        self.rastro(&args)
    }

    fn queue_items(&self) -> Vec<Value> {
        let out = self.rastro(&["queue", "--json"]);
        assert!(out.status.success(), "{}", stderr(&out));
        let queue: Value = serde_json::from_slice(&out.stdout).unwrap();
        queue["result"]["items"].as_array().unwrap().clone()
    }

    fn db(&self) -> SqliteStorage {
        SqliteStorage::open(&self.home.path().join("data/rastro.sqlite3")).unwrap()
    }

    /// One observed field of `location` in `source`, as a connector run records it.
    fn observe(&self, source: &Source, location: &str, fields: &[(&str, &str)]) -> Vec<i64> {
        let mut db = self.db();
        db.project_sources(std::slice::from_ref(source)).unwrap();
        let run = db
            .start_run(source, ("fixture", "1"), time(), (time(), time()))
            .unwrap();
        fields
            .iter()
            .map(|(field, quote)| {
                let evidence = Evidence {
                    source: source.name.clone(),
                    scope: source.scope.clone(),
                    location: location.into(),
                    field: (*field).into(),
                    start: 0,
                    end: quote.len() as u64,
                    quote: (*quote).into(),
                    trust_class: TrustClass::Structured,
                    captured_at: time(),
                    source_updated_at: Some(time()),
                    original: None,
                };
                match db.append_evidence(run, &evidence).unwrap() {
                    rastro_core::Appended::New(id) | rastro_core::Appended::Unchanged(id) => id,
                }
            })
            .collect()
    }

    /// An issue Linear says exists in `linear-work`.
    fn observe_issue(&self, id: &str, identifier: &str, title: &str, state: &str) {
        let source = Source::new("linear-work".into(), "linear".into(), Scope("work".into()));
        self.observe(
            &source,
            &format!("linear:issue/{id}"),
            &[
                ("/identifier", identifier),
                ("/title", title),
                ("/state/type", state),
            ],
        );
    }

    /// A claim observed in `scope` only, and its id.
    fn claim_in(&self, scope: &str, subject: &str) -> String {
        let source = Source::github(format!("notes-{scope}"), "o/p".into(), Scope(scope.into()));
        let evidence_id = self.observe(&source, subject, &[("/state", "open")])[0];
        let object = Object::Literal {
            value: "open".into(),
            datatype: "string".into(),
        };
        let location = subject;
        let subject = Entity::new(subject.into(), "code:PullRequest".into());
        let id = claims::claim_id(&subject.id, "code:state", &object);
        let mut db = self.db();
        let run = db
            .start_run(&source, ("fixture", "1"), time(), (time(), time()))
            .unwrap();
        db.record_claims(
            run,
            ("fixture", "1"),
            &[ObservedClaim {
                id: id.clone(),
                subject,
                predicate: "code:state".into(),
                object,
                scalar: true,
                type_ancestors: Vec::new(),
                supports: BTreeSet::from([Support {
                    portable_key: claims::portable_evidence_key(&Evidence {
                        source: source.name.clone(),
                        scope: source.scope.clone(),
                        location: location.into(),
                        field: "/state".into(),
                        start: 0,
                        end: 4,
                        quote: "open".into(),
                        trust_class: TrustClass::Structured,
                        captured_at: time(),
                        source_updated_at: Some(time()),
                        original: None,
                    }),
                    evidence_id,
                    field: "/state".into(),
                    start: 0,
                    end: 4,
                    extraction_rule: "fixture-rule".into(),
                }]),
            }],
            None,
        )
        .unwrap();
        id
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// spec: An agent proposes an action
#[test]
fn a_citation_the_sources_scope_cannot_see_is_refused_and_nothing_is_recorded() {
    let env = Env::new();
    let personal = env.claim_in("personal", "https://example.test/o/p/pull/1");
    let visible = env.claim_in("work", "https://example.test/o/p/pull/2");

    let out = env.propose("Flaky sync test", &["--cite", &personal]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let message = stderr(&out);
    assert!(message.contains(&personal), "{message}");
    assert!(message.contains("work"), "{message}");
    assert!(env.queue_items().is_empty());

    // A claim the work scope does see is cited without trouble, so the refusal above was the
    // scope's and not a refusal of every citation.
    let out = env.propose("Flaky sync test", &["--cite", &visible]);
    assert!(out.status.success(), "{}", stderr(&out));
    let items = env.queue_items();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["action"]["cites"], serde_json::json!([visible]));
}

// spec: A duplicate title is flagged
#[test]
fn an_open_issue_with_the_same_title_is_named_and_the_proposal_stays_queued() {
    let env = Env::new();
    env.observe_issue("a1", "ENG-12", "Flaky sync test", "started");
    env.observe_issue("a2", "ENG-13", "Flaky sync test with a suffix", "started");
    env.observe_issue("a3", "ENG-14", "flaky sync test", "unstarted");

    let out = env.propose("Flaky sync test", &[]);
    assert!(out.status.success(), "{}", stderr(&out));

    let items = env.queue_items();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(
        items[0]["possible_duplicates"],
        serde_json::json!(["ENG-12"])
    );

    let printed = env.rastro(&["queue"]);
    let text = String::from_utf8_lossy(&printed.stdout).into_owned();
    assert!(text.contains("possible duplicate of ENG-12"), "{text}");
    assert!(!text.contains("ENG-13"), "{text}");
    assert!(!text.contains("ENG-14"), "{text}");
}

// spec: A duplicate title is flagged
#[test]
fn a_done_or_canceled_issue_with_the_same_title_carries_no_mark() {
    let env = Env::new();
    env.observe_issue("b1", "ENG-20", "Flaky sync test", "completed");
    env.observe_issue("b2", "ENG-21", "Flaky sync test", "canceled");

    let out = env.propose("Flaky sync test", &[]);
    assert!(out.status.success(), "{}", stderr(&out));

    let items = env.queue_items();
    assert_eq!(items.len(), 1, "{items:?}");
    assert!(items[0].get("possible_duplicates").is_none(), "{items:?}");
    let printed = env.rastro(&["queue"]);
    let text = String::from_utf8_lossy(&printed.stdout).into_owned();
    assert!(!text.contains("possible duplicate"), "{text}");
}
