//! Ids and refs (§5): `id` within the same repo, `repo-id/id` from anywhere.

use std::fmt;

/// Ids match `^[a-z0-9][a-z0-9-]{0,62}$`.
pub fn valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The message for an invalid id, naming the rule.
pub fn id_rule(what: &str, s: &str) -> String {
    format!(
        "{s:?} is not a valid {what}: use lowercase letters, digits and dashes, starting with a letter or digit, at most 63 characters"
    )
}

/// A pointer to something: `argocd` (same repo) or `infra-repo/argocd`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ref {
    pub repo: String,
    pub id: String,
}

impl Ref {
    /// Parse `id` or `repo/id`, resolving a bare id against `home`.
    pub fn parse(s: &str, home: &str) -> Result<Ref, String> {
        let (repo, id) = match s.split_once('/') {
            Some((r, i)) => (r, i),
            None => (home, s),
        };
        if !valid_id(repo) || !valid_id(id) {
            return Err(format!(
                "{s:?} is not a ref: write an id like `argocd` (this repo) or `infra-repo/argocd` (another repo)"
            ));
        }
        Ok(Ref {
            repo: repo.to_string(),
            id: id.to_string(),
        })
    }

    pub fn is_local(&self, home: &str) -> bool {
        self.repo == home
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.repo, self.id)
    }
}

/// GitHub-style heading slugs: lowercase, letters/digits kept, spaces and
/// dashes become dashes, everything else dropped. Duplicates get `-2`, `-3`
/// (§5.2) through [`Slugger`].
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.trim().chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if c == ' ' || c == '-' {
            out.push('-');
        }
    }
    if out.is_empty() {
        "section".to_string()
    } else {
        out
    }
}

#[derive(Default)]
pub struct Slugger {
    seen: std::collections::HashMap<String, u32>,
}

impl Slugger {
    pub fn next(&mut self, text: &str) -> String {
        let base = slug(text);
        let n = self.seen.entry(base.clone()).or_insert(0);
        *n += 1;
        if *n == 1 { base } else { format!("{base}-{n}") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_id("infra-repo"));
        assert!(valid_id("a"));
        assert!(valid_id("0day"));
        assert!(!valid_id(""));
        assert!(!valid_id("-a"));
        assert!(!valid_id("A"));
        assert!(!valid_id("a_b"));
        assert!(!valid_id(&"a".repeat(64)));
        assert!(valid_id(&"a".repeat(63)));
    }

    #[test]
    fn refs() {
        assert_eq!(
            Ref::parse("argocd", "infra").map(|r| r.to_string()),
            Ok("infra/argocd".into())
        );
        assert_eq!(
            Ref::parse("app/x", "infra").map(|r| r.to_string()),
            Ok("app/x".into())
        );
        assert!(Ref::parse("a/b/c", "infra").is_err());
        assert!(Ref::parse("Bad", "infra").is_err());
        assert!(Ref::parse("", "infra").is_err());
    }

    #[test]
    fn slugs() {
        let mut s = Slugger::default();
        assert_eq!(s.next("What it does not do"), "what-it-does-not-do");
        assert_eq!(s.next("What it does not do"), "what-it-does-not-do-2");
        assert_eq!(s.next("Ärger & Übel!"), "ärger--übel");
        assert_eq!(s.next("???"), "section");
    }
}
