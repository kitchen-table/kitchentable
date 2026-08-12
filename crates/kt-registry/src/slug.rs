//! Folder name to URL-safe slug.
//!
//! The slug becomes a hostname (`trip-planner.local`) and a path prefix, so it
//! has to survive DNS: lowercase, ASCII alphanumerics and hyphens, no leading
//! or trailing hyphen, non-empty.

/// Slugify a folder name. Never fails: anything unusable becomes `app`.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_was_hyphen = false;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !out.is_empty() && !last_was_hyphen {
            out.push('-');
            last_was_hyphen = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    // A hostname label is capped at 63 octets; trim on a boundary.
    if out.len() > 63 {
        out.truncate(63);
        while out.ends_with('-') {
            out.pop();
        }
    }

    if out.is_empty() {
        "app".to_string()
    } else {
        out
    }
}

/// Suffix until unique. Two folders can slugify the same way ("My App" and
/// "my-app"), and a slug is a hostname, so collisions have to resolve rather
/// than one app silently shadowing another (docs/architecture.md section 10).
pub fn deduplicate(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    // A thousand folders slugifying identically is not a real workspace.
    format!("{base}-{}", taken.len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercases_and_hyphenates() {
        assert_eq!(slugify("Trip Planner"), "trip-planner");
        assert_eq!(slugify("Chores Rota"), "chores-rota");
    }

    #[test]
    fn collapses_runs_of_punctuation() {
        assert_eq!(slugify("my   app!!!"), "my-app");
        assert_eq!(slugify("a_b_c"), "a-b-c");
    }

    #[test]
    fn trims_leading_and_trailing_separators() {
        assert_eq!(slugify("  spaced  "), "spaced");
        assert_eq!(slugify("...dots..."), "dots");
        assert_eq!(slugify("-lead-"), "lead");
    }

    #[test]
    fn drops_characters_that_cannot_live_in_a_hostname() {
        assert_eq!(slugify("café münchen"), "caf-m-nchen");
        assert_eq!(slugify("emoji 🧳 trip"), "emoji-trip");
    }

    #[test]
    fn always_produces_something_usable() {
        assert_eq!(slugify(""), "app");
        assert_eq!(slugify("🧳"), "app");
        assert_eq!(slugify("---"), "app");
    }

    #[test]
    fn respects_the_hostname_label_limit() {
        let slug = slugify(&"a".repeat(100));
        assert_eq!(slug.len(), 63);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn collisions_get_a_numeric_suffix() {
        let taken = vec!["notes".to_string()];
        assert_eq!(deduplicate("notes", &taken), "notes-2");

        let taken = vec!["notes".to_string(), "notes-2".to_string()];
        assert_eq!(deduplicate("notes", &taken), "notes-3");
    }

    #[test]
    fn a_free_slug_is_left_alone() {
        assert_eq!(deduplicate("notes", &[]), "notes");
    }
}
