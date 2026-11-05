//! The forms an entity is named by, on the read side and on submit. Syntax only: whether a
//! namespace is installed, or a form matches anything, is not decided here.

/// What an operation error says when a form is outside the grammar.
pub const FORMS: &str = "an entity id (`e_` and 32 lowercase hex digits), an exact canonical URL \
(`<scheme>://<rest>`), or a canonical alias `<namespace>:<kind>/<value>`";

/// `Ok` when `entity` is one of the three accepted forms.
pub fn check(entity: &str) -> Result<(), String> {
    if is_entity_id(entity) || is_url(entity) || is_alias(entity) {
        Ok(())
    } else {
        Err(format!("entity {entity:?} must be {FORMS}"))
    }
}

fn is_entity_id(entity: &str) -> bool {
    entity.strip_prefix("e_").is_some_and(|id| {
        id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn is_url(entity: &str) -> bool {
    let Some((scheme, rest)) = entity.split_once("://") else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        && !rest.is_empty()
        && !entity.chars().any(char::is_whitespace)
}

/// Namespace and kind hold no whitespace; the value may hold inner whitespace, `/`, `[` and `]`.
fn is_alias(entity: &str) -> bool {
    let plain = |part: &str| !part.is_empty() && !part.chars().any(char::is_whitespace);
    entity.split_once(':').is_some_and(|(namespace, rest)| {
        plain(namespace)
            && rest
                .split_once('/')
                .is_some_and(|(kind, value)| plain(kind) && is_alias_value(value))
    })
}

/// An alias value is non-empty and has no whitespace at either end.
pub fn is_alias_value(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

#[cfg(test)]
mod tests {
    use super::*;

    /// claims/R30: the three forms and the shapes around them
    #[test]
    fn accepts_the_three_forms_and_refuses_the_rest() {
        for good in [
            "e_0123456789abcdef0123456789abcdef",
            "https://github.com/o/p/pull/1#issuecomment-5",
            "github:account/42",
            "github:login/vercel[bot]",
            "github:login/a/b",
            "slack:name/Felippe Bisca",
        ] {
            assert!(check(good).is_ok(), "{good}");
        }
        for bad in [
            "foo:bar",
            "github:FBisca",
            "github:login/",
            ":x/y",
            "github:/x",
            "e_0123456789ABCDEF0123456789abcdef",
            "e_0123",
            "git hub:login/x",
            "github:lo gin/x",
            " github:login/x",
            "github:login/x ",
            "github:login/ x",
            "1http://x",
            "http://",
            "http://a b",
            "git\thub:login/x",
            "github:log\nin/x",
            "github:login/x\u{a0}",
            "github:login/\u{a0}x",
            "e_0123456789abcdef0123456789abcde",
            "e_0123456789abcdef0123456789abcdef0",
            "github:/v",
        ] {
            let error = check(bad).unwrap_err();
            assert!(error.contains("<namespace>:<kind>/<value>"), "{error}");
        }
    }
}
