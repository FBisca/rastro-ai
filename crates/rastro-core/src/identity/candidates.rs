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
pub fn fold_name(name: &str) -> String {
    let folded: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(unaccented)
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}
/// A lowercase Latin letter without its accent.
fn unaccented(c: char) -> char {
    match c {
        'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' | 'đ' => 'd',
        'è'..='ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ì'..='ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => 'l',
        'ñ' | 'ń' | 'ņ' | 'ň' => 'n',
        'ò'..='ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' | 'ŧ' => 't',
        'ù'..='ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ý' | 'ÿ' | 'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
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

/// The namespace of a canonical identifier: what comes before its first `:`.
fn namespace(identifier: &str) -> &str {
    identifier.split_once(':').map_or(identifier, |(ns, _)| ns)
}

/// The fuzzy candidates among `accounts`: two entities of different namespaces, each typed only
/// as an account (never a bot), with at least one signal. Sorted by the pair's identifiers.
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
            if namespace(&a.identifier) == namespace(&b.identifier) {
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

impl<S: Storage> Rastro<S> {
    /// The fuzzy candidates, computed on read over every scope: only the CLI, which sees every
    /// scope, lists them.
    pub fn identity_candidates(&self) -> Result<Vec<Candidate>> {
        self.storage.snapshot(|view| {
            let resolution = Resolution::of(&view.intents()?);
            Ok(candidate_pairs(&view.identity_accounts()?, &resolution))
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

    /// S23: a name folds case, accents (precomposed or combining) and runs of whitespace
    #[test]
    fn S23_a_name_folds_case_and_accents() {
        assert_eq!(fold_name("Ana SÍLVA"), "ana silva");
        assert_eq!(fold_name("  Ana \t  Silva "), "ana silva");
        assert_eq!(fold_name("Jose\u{301} Conceição"), "jose conceicao");
        assert_eq!(fold_name("Łukasz Øster"), "lukasz oster");
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
}
