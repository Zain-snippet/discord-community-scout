//! Input validation and normalisation shared by the API and CSV import.

use crate::models::PromotionPolicy;

pub const MAX_NAME: usize = 200;
pub const MAX_DESCRIPTION: usize = 2000;
pub const MAX_NOTES: usize = 5000;
pub const MAX_TAG_LEN: usize = 40;
pub const MAX_TAGS: usize = 12;
pub const MAX_CHANNEL: usize = 100;
pub const MAX_EVIDENCE: usize = 2000;

/// Trims and collapses internal whitespace runs to single spaces.
pub fn clean_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Validates a required short text field.
pub fn required_text(value: &str, field: &str, max: usize) -> Result<String, String> {
    let cleaned = clean_text(value);
    if cleaned.is_empty() {
        return Err(format!("{field} is required."));
    }
    if cleaned.chars().count() > max {
        return Err(format!("{field} must be at most {max} characters."));
    }
    Ok(cleaned)
}

/// Validates an optional longer text field.
pub fn optional_text(
    value: Option<&str>,
    field: &str,
    max: usize,
) -> Result<Option<String>, String> {
    match value.map(clean_text) {
        None => Ok(None),
        Some(text) if text.is_empty() => Ok(None),
        Some(text) => {
            if text.chars().count() > max {
                return Err(format!("{field} must be at most {max} characters."));
            }
            Ok(Some(text))
        }
    }
}

/// Only http and https URLs are accepted; anything else (including
/// `javascript:` and `file:`) is rejected before it reaches the database.
pub fn validate_url(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("Only http and https URLs are allowed.".to_string());
    }
    if trimmed.len() > 2048 {
        return Err("URL is too long.".to_string());
    }
    let after_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or("");
    let host = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty() || !host.contains('.') {
        return Err("URL must include a domain name.".to_string());
    }
    Ok(trimmed.to_string())
}

pub fn optional_url(value: Option<&str>) -> Result<Option<String>, String> {
    match value.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(None),
        Some(v) => validate_url(v).map(Some),
    }
}

/// Extracts a normalised invite code from a Discord invite URL.
///
/// Returns `(normalised_url, code)` when the URL is a Discord invite.
pub fn normalise_invite(value: &str) -> (String, Option<String>) {
    let trimmed = value.trim();
    let lowered = trimmed.to_ascii_lowercase();

    let code = lowered
        .strip_prefix("https://discord.gg/")
        .or_else(|| lowered.strip_prefix("http://discord.gg/"))
        .or_else(|| lowered.strip_prefix("discord.gg/"))
        .or_else(|| {
            lowered
                .split_once("discord.com/invite/")
                .map(|(_, rest)| rest)
        })
        .or_else(|| {
            lowered
                .split_once("discordapp.com/invite/")
                .map(|(_, rest)| rest)
        });

    match code {
        Some(code) => {
            let code = code.split(['?', '#', '/']).next().unwrap_or("").to_string();
            if code.is_empty() {
                (trimmed.to_string(), None)
            } else {
                (
                    format!("https://discord.gg/{code}"),
                    Some(code.to_ascii_lowercase()),
                )
            }
        }
        None => (trimmed.to_string(), None),
    }
}

/// Cleans, deduplicates (case-insensitively) and bounds the tag list.
pub fn normalise_tags(tags: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for raw in tags {
        let tag = clean_text(raw).to_lowercase();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_LEN {
            return Err(format!(
                "Tag must be at most {MAX_TAG_LEN} characters: {tag}"
            ));
        }
        if !out.iter().any(|existing| existing == &tag) {
            out.push(tag);
        }
    }
    if out.len() > MAX_TAGS {
        return Err(format!("At most {MAX_TAGS} tags are allowed."));
    }
    Ok(out)
}

/// Parses the promotion policy, rejecting unknown values with a clear message.
pub fn parse_policy(value: &str) -> Result<PromotionPolicy, String> {
    PromotionPolicy::parse(&clean_text(value).to_ascii_lowercase())
        .ok_or_else(|| "promotion_policy must be one of: allowed, prohibited, unknown.".to_string())
}

/// Invite fields normalised together: `(invite_url, invite_code, canonical_url)`.
pub type NormalisedInviteFields = (Option<String>, Option<String>, Option<String>);

/// Affiliates any invite-looking URL and validates the rest.
pub fn normalise_invite_fields(
    invite_url: Option<&str>,
    canonical_url: Option<&str>,
) -> Result<NormalisedInviteFields, String> {
    let mut code = None;
    let invite = match invite_url.map(str::trim) {
        None | Some("") => None,
        Some(raw) => {
            let (normalised, extracted) = normalise_invite(raw);
            if extracted.is_none() {
                // Not a Discord invite: still store it if it is a safe URL.
                validate_url(&normalised)?;
            }
            code = extracted;
            Some(normalised)
        }
    };

    let canonical = optional_url(canonical_url)?;

    Ok((invite, code, canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_invite_codes_from_common_forms() {
        for url in [
            "https://discord.gg/abc123",
            "https://discord.com/invite/abc123",
            "http://discord.gg/abc123?event=1",
            "discord.gg/abc123",
        ] {
            let (normalised, code) = normalise_invite(url);
            assert_eq!(code.as_deref(), Some("abc123"), "failed for {url}");
            assert_eq!(normalised, "https://discord.gg/abc123");
        }
    }

    #[test]
    fn invite_codes_are_case_insensitive_for_dedupe() {
        let (_, a) = normalise_invite("https://discord.gg/ABC123");
        let (_, b) = normalise_invite("https://discord.com/invite/abc123");
        assert_eq!(a, b);
    }

    #[test]
    fn non_invite_urls_have_no_code() {
        let (normalised, code) = normalise_invite("https://example.com/community");
        assert_eq!(code, None);
        assert_eq!(normalised, "https://example.com/community");
    }

    #[test]
    fn rejects_dangerous_url_schemes() {
        for bad in [
            "javascript:alert(1)",
            "data:text/html,<script>",
            "file:///etc/passwd",
            "ftp://example.com",
        ] {
            assert!(validate_url(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn accepts_plain_https_urls() {
        assert!(validate_url("https://example.com/path?q=1").is_ok());
    }

    #[test]
    fn rejects_url_without_domain() {
        assert!(validate_url("https://localhost").is_err());
        assert!(validate_url("https://").is_err());
    }

    #[test]
    fn tags_are_lowercased_and_deduplicated() {
        let tags = vec![
            "Students".to_string(),
            "students".to_string(),
            "  Side  Hustle ".to_string(),
        ];
        let out = normalise_tags(&tags).unwrap();
        assert_eq!(out, vec!["students".to_string(), "side hustle".to_string()]);
    }

    #[test]
    fn too_many_tags_are_rejected() {
        let tags: Vec<String> = (0..20).map(|i| format!("tag{i}")).collect();
        assert!(normalise_tags(&tags).is_err());
    }

    #[test]
    fn empty_name_is_rejected() {
        assert!(required_text("   ", "Name", MAX_NAME).is_err());
    }

    #[test]
    fn overlong_name_is_rejected() {
        let long = "a".repeat(MAX_NAME + 1);
        assert!(required_text(&long, "Name", MAX_NAME).is_err());
    }

    #[test]
    fn policy_parsing_is_strict() {
        assert_eq!(parse_policy("allowed").unwrap(), PromotionPolicy::Allowed);
        assert_eq!(
            parse_policy("PROHIBITED").unwrap(),
            PromotionPolicy::Prohibited
        );
        assert!(parse_policy("maybe").is_err());
    }
}
