//! The standard vocabulary `std:`: the commitment type and the four predicates the list's terms
//! name. The core bundles it, so every hypothesis can carry the terms it expresses
//! (decisions/vocabulary.md §5, §10).
use anyhow::Result;
use rastro_plugin_sdk::Vocabulary;

/// The standard vocabulary's file, bundled with the core.
const STANDARD: &str = include_str!("std.toml");

/// The standard vocabulary, parsed and checked as any vocabulary file is.
pub fn standard() -> Result<Vocabulary> {
    Vocabulary::parse(STANDARD)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S5: the standard vocabulary defines the commitment type and its four predicates, each with
    /// its definition, what it is not, examples, counter-examples and the evidence it needs.
    #[test]
    #[allow(non_snake_case, reason = "the test is named by its scenario id")]
    fn S5_the_standard_vocabulary_defines_the_commitment_terms() {
        let vocabulary = standard().expect("the standard vocabulary loads");
        assert_eq!(vocabulary.namespace, "std");
        let types: Vec<&str> = vocabulary.types.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(types, ["std:Commitment"]);
        let predicates: Vec<&str> = vocabulary
            .predicates
            .iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(
            predicates,
            ["std:owes", "std:owed_to", "std:due", "std:waiting_on"]
        );
        let entries = vocabulary
            .types
            .iter()
            .map(|t| (&t.id, &t.about))
            .chain(vocabulary.predicates.iter().map(|p| (&p.id, &p.about)));
        for (id, about) in entries {
            assert!(
                !about.definition.trim().is_empty(),
                "{id} has no definition"
            );
            assert!(
                !about.not_to_confuse_with.trim().is_empty(),
                "{id} has no not-to-confuse"
            );
            assert!(!about.examples.is_empty(), "{id} has no examples");
            assert!(
                !about.counter_examples.is_empty(),
                "{id} has no counter-examples"
            );
            assert!(
                !about.evidence_requirement.trim().is_empty(),
                "{id} has no evidence"
            );
        }
    }
}
