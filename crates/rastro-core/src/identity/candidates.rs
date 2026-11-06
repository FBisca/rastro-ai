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

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    /// S23: a name folds case, accents (precomposed or combining) and runs of whitespace
    #[test]
    fn S23_a_name_folds_case_and_accents() {
        assert_eq!(fold_name("Ana SÍLVA"), "ana silva");
        assert_eq!(fold_name("  Ana \t  Silva "), "ana silva");
        assert_eq!(fold_name("Jose\u{301} Conceição"), "jose conceicao");
        assert_eq!(fold_name("Łukasz Øster"), "lukasz oster");
        assert_ne!(fold_name("Ana Silva"), fold_name("Ana Silveira"));
    }
}
