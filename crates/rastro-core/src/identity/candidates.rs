//! Fuzzy candidates (decisions/knowledge.md §55, §56): two accounts that may belong to one
//! human, computed on read and never stored. Only the owner's answer is stored, as intents.
use super::*;

/// What the candidate rules read about one entity: its id and canonical identifier, whether
/// every current type of it is an account, its current `name`, `login` and `email` aliases (each
/// sorted), and the Persons that hold it now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountFacts {
    pub id: String,
    pub identifier: String,
    pub account: bool,
    pub names: Vec<String>,
    pub logins: Vec<String>,
    pub emails: Vec<String>,
    pub holders: BTreeSet<String>,
}

/// A name as the `same name` signal compares it: lowercased, with the accents of Latin letters
/// removed, whether precomposed or combining, and every run of whitespace made one space.
/// Canonical decomposition (NFD) splits a precomposed letter into its base and combining marks,
/// which are then dropped.
pub fn fold_name(name: &str) -> String {
    let nfd = icu_normalizer::DecomposingNormalizerBorrowed::new_nfd();
    let folded: String = nfd
        .normalize_iter(name.chars().flat_map(char::to_lowercase))
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(unaccented)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// A lowercase Latin letter whose accent canonical decomposition doesn't remove (a stroke, a
/// bar, a middle dot, a missing dot), without it.
fn unaccented(c: char) -> char {
    match c {
        'đ' => 'd',
        'ħ' => 'h',
        'ı' => 'i',
        'ŀ' | 'ł' => 'l',
        'ø' => 'o',
        'ŧ' => 't',
        other => other,
    }
}

/// Why two accounts may be one human.
#[derive(Clone, Copy, Debug, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Signal {
    /// Equal full names after case and accent folding.
    #[serde(rename = "same name")]
    SameName,
    /// An email's local part equals the other account's login.
    #[serde(rename = "email matches login")]
    EmailMatchesLogin,
}
/// One signal, with the value each account of the pair gave, in the pair's order.
#[derive(Clone, Debug, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CandidateReason {
    pub reason: Signal,
    pub values: [String; 2],
}

/// The signals that `a` and `b` may be one human, values in that order: each pair of full names
/// (two words or more) equal once folded, and each email of one whose local part equals a login of the other,
/// ASCII case-folded.
pub fn reasons(a: &AccountFacts, b: &AccountFacts) -> Vec<CandidateReason> {
    let mut found = Vec::new();
    for x in a.names.iter().filter(|x| fold_name(x).contains(' ')) {
        for y in b.names.iter().filter(|y| fold_name(x) == fold_name(y)) {
            found.push(CandidateReason {
                reason: Signal::SameName,
                values: [x.clone(), y.clone()],
            });
        }
    }
    for (one, other, flipped) in [(a, b, false), (b, a, true)] {
        for email in &one.emails {
            let Some((local, _)) = email.rsplit_once('@') else {
                continue;
            };
            for login in other
                .logins
                .iter()
                .filter(|l| l.eq_ignore_ascii_case(local))
            {
                let values = if flipped {
                    [login.clone(), email.clone()]
                } else {
                    [email.clone(), login.clone()]
                };
                found.push(CandidateReason {
                    reason: Signal::EmailMatchesLogin,
                    values,
                });
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// An account of a candidate pair.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct CandidateAccount {
    pub id: String,
    pub identifier: String,
    /// The Persons holding it now, by canonical identifier.
    pub held_by: Vec<String>,
}
/// Two accounts that may be one human, and why.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Candidate {
    /// The two accounts, in identifier order.
    pub accounts: [CandidateAccount; 2],
    pub reasons: Vec<CandidateReason>,
    /// The name a yes gives the Person it creates when neither account is held: the longest
    /// name either account has (alphabetically first among equals), else the matched login.
    pub name: String,
}

impl Resolution {
    /// The Persons merged with `person` through effective merges, itself included.
    pub fn merge_class(&self, person: &str) -> BTreeSet<String> {
        let mut class = BTreeSet::from([person.to_string()]);
        loop {
            let before = class.len();
            for (a, b) in self.merges.keys() {
                if class.contains(a) || class.contains(b) {
                    class.insert(a.clone());
                    class.insert(b.clone());
                }
            }
            if class.len() == before {
                return class;
            }
        }
    }
    /// Whether the owner has settled the pair: one Person holds both (merged Persons are one),
    /// or a cannot-link refuses the two accounts, or one account and a Person holding the other.
    fn settled(&self, a: &AccountFacts, b: &AccountFacts) -> bool {
        let held = |f: &AccountFacts| -> BTreeSet<String> {
            f.holders.iter().flat_map(|p| self.merge_class(p)).collect()
        };
        let (held_a, held_b) = (held(a), held(b));
        let refused = |f: &AccountFacts, persons: &BTreeSet<String>| {
            (persons.iter()).any(|p| self.cannot.contains_key(&sorted(&f.id, p)))
        };
        !held_a.is_disjoint(&held_b)
            || self.cannot.contains_key(&sorted(&a.id, &b.id))
            || refused(a, &held_b)
            || refused(b, &held_a)
    }
}

/// The namespace of a canonical identifier: what comes before its first `:`.
fn namespace(identifier: &str) -> &str {
    identifier.split_once(':').map_or(identifier, |(ns, _)| ns)
}

/// The fuzzy candidates among `accounts`: two entities of different namespaces, each typed only
/// as an account (never a bot), with at least one signal, that the owner hasn't settled.
/// Sorted by the pair's identifiers.
pub fn candidate_pairs(accounts: &[AccountFacts], resolution: &Resolution) -> Vec<Candidate> {
    let mut accounts: Vec<&AccountFacts> = accounts.iter().filter(|a| a.account).collect();
    accounts.sort_by(|a, b| a.identifier.cmp(&b.identifier));
    let shown = |f: &AccountFacts| CandidateAccount {
        id: f.id.clone(),
        identifier: f.identifier.clone(),
        held_by: (f.holders.iter())
            .filter_map(|p| resolution.persons.get(p))
            .map(|p| p.entity.identifier.clone())
            .collect(),
    };
    let mut found = Vec::new();
    for (i, a) in accounts.iter().enumerate() {
        for b in &accounts[i + 1..] {
            if namespace(&a.identifier) == namespace(&b.identifier) || resolution.settled(a, b) {
                continue;
            }
            let reasons = reasons(a, b);
            let Some(first) = reasons.first() else {
                continue;
            };
            let longest = (a.names.iter().chain(&b.names))
                .max_by(|x, y| x.chars().count().cmp(&y.chars().count()).then(y.cmp(x)));
            let login = first.values.iter().find(|v| !v.contains('@'));
            let name = longest.or(login).cloned().unwrap_or_default();
            found.push(Candidate {
                accounts: [shown(a), shown(b)],
                reasons,
                name,
            });
        }
    }
    found
}

/// What the owner's answer to the candidate pair `a`, `b` records, given every account's facts
/// and the intents accepted so far. A no is a cannot-link between the two accounts. A yes links
/// both to the Person holding one of them, or, when neither is held, to a new Person named
/// `name` (the candidate's name by default). Refused when the pair isn't a candidate now, or
/// when more than one Person holds its accounts.
fn plan_answer(
    a: EntityTarget,
    b: EntityTarget,
    facts: &[AccountFacts],
    resolution: &Resolution,
    yes: bool,
    name: Option<&str>,
) -> Result<(Vec<Intent>, Option<EntityTarget>, bool)> {
    let intent = |kind, targets, payload| new_intent(kind, String::new(), targets, payload, None);
    let pairs = candidate_pairs(facts, resolution);
    let ids = [&a.id, &b.id];
    let Some(candidate) = (pairs.iter()).find(|c| c.accounts.iter().all(|x| ids.contains(&&x.id)))
    else {
        bail!(
            "{} and {} are not a candidate pair; `rastro identity candidates` lists the pairs",
            a.identifier,
            b.identifier
        );
    };
    if !yes {
        let pair = Targets::pair(true, a, b);
        let refusal = intent(IntentKind::CannotLink, pair, Payload::empty())?;
        return Ok((vec![refusal], None, false));
    }
    let holders: BTreeSet<&String> = (facts.iter().filter(|f| ids.contains(&&f.id)))
        .flat_map(|f| &f.holders)
        .collect();
    ensure!(
        holders.len() <= 1,
        "{} and {} are held by more than one Person; merge them with `rastro identity merge`, \
         or link each account with `rastro identity link`",
        a.identifier,
        b.identifier
    );
    let mut intents = Vec::new();
    let holder = match holders.first() {
        Some(id) => person(resolution, id).ok_or_else(|| anyhow::anyhow!("unknown Person {id}"))?,
        None => {
            let minted = mint_person()?;
            let named = Payload {
                version: SHAPE_VERSION,
                name: Some(name.unwrap_or(&candidate.name).into()),
            };
            let entity = Targets::entity(minted.clone());
            let create = intent(IntentKind::CreatePerson, entity.clone(), Payload::empty())?;
            intents.extend([create, intent(IntentKind::Name, entity, named)?]);
            minted
        }
    };
    for account in [a, b] {
        if !(facts.iter()).any(|f| f.id == account.id && !f.holders.is_empty()) {
            let pair = Targets::pair(false, account, holder.clone());
            intents.push(intent(IntentKind::Link, pair, Payload::empty())?);
        }
    }
    Ok((intents, Some(holder), holders.is_empty()))
}

impl<S: Storage> Rastro<S> {
    /// The fuzzy candidates, computed on read over every scope: only the CLI, which sees every
    /// scope, lists them.
    pub fn identity_candidates(&self) -> Result<Vec<Candidate>> {
        self.storage.snapshot(|view| {
            let resolution = Resolution::of(&view.intents()?);
            Ok(candidate_pairs(&view.identity_accounts()?, &resolution))
        })
    }
    /// The owner's answer to the candidate pair `one`, `other`, any forms `about` accepts (see
    /// `plan_answer`). Its intents are accepted together, or none is.
    pub fn answer(
        &mut self,
        one: &str,
        other: &str,
        yes: bool,
        name: Option<&str>,
    ) -> Result<Issued> {
        self.issue(|view, resolution| {
            let a = resolve(view, resolution, one, "account")?;
            let b = resolve(view, resolution, other, "account")?;
            plan_answer(a, b, &view.identity_accounts()?, resolution, yes, name)
        })
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// Facts about an account typed only as an account, held by nobody.
    fn facts(identifier: &str, names: &[&str], logins: &[&str], emails: &[&str]) -> AccountFacts {
        let list = |values: &[&str]| values.iter().map(|v| v.to_string()).collect();
        AccountFacts {
            id: claims::entity_id(identifier),
            identifier: identifier.into(),
            account: true,
            names: list(names),
            logins: list(logins),
            emails: list(emails),
            holders: BTreeSet::new(),
        }
    }
    /// A Person `rastro:person/<n>` that `r` knows.
    fn known_person(r: &mut Resolution, n: &str) -> EntityTarget {
        let identifier = format!("rastro:person/{n}");
        let entity = EntityTarget {
            id: claims::entity_id(&identifier),
            identifier,
            role: "person".into(),
        };
        let person = Person {
            entity: entity.clone(),
            created_by: format!("i_{n}"),
            me: false,
        };
        r.persons.insert(entity.id.clone(), person);
        entity
    }
    /// `f`, held now by `person`.
    fn held(f: &AccountFacts, person: &EntityTarget) -> AccountFacts {
        AccountFacts {
            holders: BTreeSet::from([person.id.clone()]),
            ..f.clone()
        }
    }
    /// `f` as an intent names an account.
    fn target(f: &AccountFacts) -> EntityTarget {
        EntityTarget {
            id: f.id.clone(),
            identifier: f.identifier.clone(),
            role: "account".into(),
        }
    }

    /// S23: a name folds case, accents (precomposed or combining) and runs of whitespace
    #[test]
    fn S23_a_name_folds_case_and_accents() {
        assert_eq!(fold_name("Ana SÍLVA"), "ana silva");
        assert_eq!(fold_name("  Ana \t  Silva "), "ana silva");
        assert_eq!(fold_name("Jose\u{301} Conceição"), "jose conceicao");
        assert_eq!(fold_name("Łukasz Øster"), "lukasz oster");
        assert_eq!(fold_name("Ștefan Țepeș"), "stefan tepes");
        assert_eq!(fold_name("Đặng Văn Bắc"), "dang van bac");
        assert_ne!(fold_name("Ana Silva"), fold_name("Ana Silveira"));
    }

    /// S23: names equal after case and accent folding are a `same name` signal; other names aren't
    #[test]
    fn S23_names_equal_after_case_and_accent_folding_are_a_same_name_signal() {
        let github = facts("github:account/1", &["Ana Silva"], &["asilva"], &[]);
        let linear = facts("linear:user/u-1", &["ANA SÍLVA"], &[], &["ana@example.org"]);
        let same = CandidateReason {
            reason: Signal::SameName,
            values: ["Ana Silva".into(), "ANA SÍLVA".into()],
        };
        assert_eq!(reasons(&github, &linear), [same]);
        let other = facts("linear:user/u-2", &["Ana Silveira"], &[], &[]);
        assert!(reasons(&github, &other).is_empty());
        // A one-word name is never a full name: no signal even when equal.
        let one = facts("github:account/2", &["Ana"], &[], &[]);
        let two = facts("linear:user/u-3", &["ANA"], &[], &[]);
        assert!(reasons(&one, &two).is_empty());
    }

    /// S24: an email whose local part equals the other account's login (ASCII case-folded) is an
    /// `email matches login` signal, each value in its own account's place
    #[test]
    fn S24_an_email_local_part_equal_to_a_login_is_a_signal() {
        let github = facts("github:account/1", &[], &["ASilva"], &[]);
        let linear = facts("linear:user/u-1", &[], &[], &["asilva@example.org"]);
        let signal = |values: [&str; 2]| {
            vec![CandidateReason {
                reason: Signal::EmailMatchesLogin,
                values: values.map(String::from),
            }]
        };
        assert_eq!(
            reasons(&github, &linear),
            signal(["ASilva", "asilva@example.org"])
        );
        assert_eq!(
            reasons(&linear, &github),
            signal(["asilva@example.org", "ASilva"])
        );
        let other = facts("linear:user/u-2", &[], &[], &["asilva.x@example.org"]);
        assert!(reasons(&github, &other).is_empty());
    }

    /// S27: a pair within one namespace, or with an entity not typed only as an account (a bot),
    /// is never a candidate; a pair across namespaces with a signal is, in identifier order
    #[test]
    fn S27_pairs_in_one_namespace_or_with_a_bot_are_never_candidates() {
        let none = Resolution::default();
        let github = facts("github:account/1", &["Ana Silva"], &["asilva"], &[]);
        let linear = facts("linear:user/u-1", &["ana silva"], &[], &[]);
        let found = candidate_pairs(&[linear.clone(), github.clone()], &none);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].accounts[0].identifier, "github:account/1");
        assert_eq!(found[0].accounts[1].identifier, "linear:user/u-1");
        assert_eq!(found[0].reasons[0].reason, Signal::SameName);
        assert_eq!(found[0].name, "Ana Silva");
        let twin = facts("github:account/2", &["Ana Silva"], &[], &[]);
        let bot = AccountFacts {
            account: false,
            ..facts("linear:user/app", &["Ana Silva"], &[], &[])
        };
        assert!(candidate_pairs(&[github, twin, bot], &none).is_empty());
    }

    /// S25: a pair one Person holds, directly or through a merge, is no longer a candidate; with
    /// one account held, it still is, and names its holder
    #[test]
    fn S25_a_pair_one_person_holds_is_not_a_candidate() {
        let mut r = Resolution::default();
        let (p, q) = (known_person(&mut r, "p"), known_person(&mut r, "q"));
        let github = facts("github:account/1", &["Ana Silva"], &[], &[]);
        let linear = facts("linear:user/u-1", &["ana silva"], &[], &[]);
        let found = candidate_pairs(&[held(&github, &p), linear.clone()], &r);
        assert_eq!(found[0].accounts[0].held_by, ["rastro:person/p"]);
        assert!(candidate_pairs(&[held(&github, &p), held(&linear, &p)], &r).is_empty());
        let apart = [held(&github, &p), held(&linear, &q)];
        assert_eq!(candidate_pairs(&apart, &r).len(), 1);
        r.merges.insert(sorted(&p.id, &q.id), "i_merge".into());
        assert!(candidate_pairs(&apart, &r).is_empty());
    }

    /// S26: a cannot-link between the two accounts, or between one and the Person holding the
    /// other, keeps the pair out
    #[test]
    fn S26_a_refused_pair_is_not_a_candidate() {
        let github = facts("github:account/1", &["Ana Silva"], &[], &[]);
        let linear = facts("linear:user/u-1", &["ana silva"], &[], &[]);
        let mut r = Resolution::default();
        r.cannot
            .insert(sorted(&github.id, &linear.id), "i_no".into());
        assert!(candidate_pairs(&[github.clone(), linear.clone()], &r).is_empty());
        let mut r = Resolution::default();
        let p = known_person(&mut r, "p");
        r.cannot
            .insert(sorted(&linear.id, &p.id), "i_undone".into());
        assert!(candidate_pairs(&[held(&github, &p), linear], &r).is_empty());
    }

    /// S26: a no records one cannot-link between the two accounts, after which the pair is no
    /// candidate and answering it again is refused
    #[test]
    fn S26_a_no_records_a_cannot_link_between_the_two_accounts() {
        let github = facts("github:account/1", &["Ana Silva"], &[], &[]);
        let linear = facts("linear:user/u-1", &["ana silva"], &[], &[]);
        let both = [github.clone(), linear.clone()];
        let none = Resolution::default();
        let (intents, person, _) =
            plan_answer(target(&linear), target(&github), &both, &none, false, None).unwrap();
        assert!(person.is_none());
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].kind, IntentKind::CannotLink);
        let (x, y) = sorted(&github.id, &linear.id);
        assert_eq!(intents[0].domain_key, format!("cannot_link/{x}/{y}"));
        let after = Resolution::of(&intents);
        assert!(candidate_pairs(&both, &after).is_empty());
        let again = plan_answer(target(&github), target(&linear), &both, &after, false, None);
        let error = again.unwrap_err().to_string();
        assert!(error.contains("not a candidate pair"), "{error}");
    }

    /// S25: a yes to a pair neither account of which is held creates a Person named from the
    /// candidate and links both; with one account held, it links only the other, to that Person
    #[test]
    fn S25_a_yes_links_both_accounts_to_one_person() {
        use IntentKind::{CreatePerson, Link, Name};
        let github = facts("github:account/1", &["Ana Silva"], &[], &[]);
        let linear = facts("linear:user/u-1", &["ana silva"], &[], &[]);
        let both = [github.clone(), linear.clone()];
        let none = Resolution::default();
        let answer = plan_answer(target(&linear), target(&github), &both, &none, true, None);
        let (intents, person, created) = answer.unwrap();
        let kinds: Vec<_> = intents.iter().map(|i| i.kind).collect();
        assert_eq!(kinds, [CreatePerson, Name, Link, Link]);
        assert!(created);
        assert_eq!(intents[1].payload.name.as_deref(), Some("Ana Silva"));
        let person = person.unwrap();
        for link in &intents[2..] {
            assert_eq!(link.targets.entities()[1], &person);
        }
        let mut r = Resolution::default();
        let p = known_person(&mut r, "p");
        let one_held = [held(&github, &p), linear.clone()];
        let answer = plan_answer(target(&github), target(&linear), &one_held, &r, true, None);
        let (intents, person, created) = answer.unwrap();
        assert!(!created);
        assert_eq!(person, Some(p));
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].kind, Link);
        assert_eq!(intents[0].targets.entities()[0].id, linear.id);
    }
}
