//! Connector for the CC-BY-4.0 "Awesome Discord Communities" list.
//!
//! The list is fetched from GitHub's public raw-content endpoint for the
//! repository. GitHub's API is documented public infrastructure, the content is
//! explicitly licensed for reuse with attribution, and one cached fetch per
//! cache window keeps us far below the unauthenticated rate limit.

use super::Discovered;

pub const SOURCE_ID: &str = "awesome-discord-communities";
pub const SOURCE_NAME: &str = "Awesome Discord Communities";
pub const REPO_URL: &str = "https://github.com/mhxion/awesome-discord-communities";
pub const README_URL: &str =
    "https://raw.githubusercontent.com/mhxion/awesome-discord-communities/main/README.md";

/// Extracts `[__Name__](url)` from one Markdown line.
fn parse_link(line: &str) -> Option<(String, String)> {
    let start = line.find("[__")?;
    let name_start = start + 3;
    let name_end = name_start + line[name_start..].find("__]")?;
    let name = line[name_start..name_end].trim();

    let after = &line[name_end..];
    let url_start = after.find("](")? + 2;
    let url_end = url_start + after[url_start..].find(')')?;
    let url = after[url_start..url_end].trim();

    if name.is_empty() || url.is_empty() {
        None
    } else {
        Some((name.to_string(), url.to_string()))
    }
}

/// Parses the generated README into listings.
///
/// Category headings (`## `) and subcategory headings (`### `) are tracked so
/// each listing carries the real section it was published under.
pub fn parse_readme(text: &str) -> Vec<Discovered> {
    let mut out: Vec<Discovered> = Vec::new();
    let mut category = String::new();
    let mut subcategory = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();

        if let Some(heading) = line.strip_prefix("### ") {
            subcategory = clean_heading(heading);
            continue;
        }
        if let Some(heading) = line.strip_prefix("## ") {
            let heading = clean_heading(heading);
            // The table of contents is a heading we never want as a category.
            if heading.eq_ignore_ascii_case("contents") {
                category.clear();
            } else {
                category = heading;
            }
            subcategory.clear();
            continue;
        }

        let Some((name, url)) = parse_link(line) else {
            continue;
        };
        let Some(code) = invite_code(&url) else {
            continue;
        };

        if category.is_empty() {
            continue;
        }

        // One row per invite code; the first section seen wins.
        if out.iter().any(|existing| existing.invite_code == code) {
            continue;
        }

        let mut tags: Vec<String> = Vec::new();
        for value in [&subcategory, &category] {
            let tag = value.to_lowercase();
            if !tag.is_empty() && !tags.contains(&tag) {
                tags.push(tag);
            }
        }

        out.push(Discovered {
            name: name.replace("\\", "").trim().to_string(),
            invite_url: format!("https://discord.gg/{code}"),
            invite_code: code,
            category: category.clone(),
            subcategory: subcategory.clone(),
            listing_url: format!("{REPO_URL}/blob/main/README.md"),
            tags,
        });
    }

    out
}

fn clean_heading(heading: &str) -> String {
    heading
        .trim()
        .trim_end_matches('#')
        .trim_matches('*')
        .trim()
        .to_string()
}

/// Accepts `discord.gg/<code>` and `discord.com/invite/<code>` forms.
pub fn invite_code(url: &str) -> Option<String> {
    let lowered = url.to_ascii_lowercase();
    let rest = lowered
        .strip_prefix("https://discord.gg/")
        .or_else(|| lowered.strip_prefix("http://discord.gg/"))
        .or_else(|| {
            lowered
                .split_once("discord.com/invite/")
                .map(|(_, rest)| rest)
        })
        .or_else(|| {
            lowered
                .split_once("discordapp.com/invite/")
                .map(|(_, rest)| rest)
        })?;

    let code = rest.split(['/', '?', '#']).next().unwrap_or("").trim();
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
## Contents

- [Programming in General](#programming-in-general)

## Programming in General

### Support Communities

[__Devcord__](https://discord.com/invite/devcord) [<img src="images/badges/official.webp">](badges.md#official) \
[__Developer's Keep__](https://discord.com/invite/vG5uukx) \

### Competitive Programming

[__CP Community__](https://discord.com/invite/cp) \

## Programming Languages

### Rust

[__Rust Lang__](https://discord.gg/rust) \
[__Rust Lang Duplicate__](https://discord.com/invite/rust) \
"#;

    #[test]
    fn parses_entries_with_categories() {
        let parsed = parse_readme(SAMPLE);
        assert_eq!(parsed.len(), 4, "duplicate should be collapsed: {parsed:?}");
        assert_eq!(parsed[0].name, "Devcord");
        assert_eq!(parsed[0].invite_code, "devcord");
        assert_eq!(parsed[0].category, "Programming in General");
        assert_eq!(parsed[0].subcategory, "Support Communities");
        assert_eq!(
            parsed[0].tags,
            vec!["support communities", "programming in general"]
        );
    }

    #[test]
    fn deduplicates_invite_codes() {
        let parsed = parse_readme(SAMPLE);
        let rust: Vec<_> = parsed.iter().filter(|d| d.invite_code == "rust").collect();
        assert_eq!(rust.len(), 1);
        assert_eq!(rust[0].name, "Rust Lang");
    }

    #[test]
    fn ignores_table_of_contents_links() {
        let parsed = parse_readme(SAMPLE);
        assert!(parsed.iter().all(|d| d.category != "Contents"));
    }

    #[test]
    fn description_uses_only_real_section_names() {
        let parsed = parse_readme(SAMPLE);
        assert_eq!(
            parsed[0].description(),
            "Listed under Programming in General / Support Communities."
        );
    }

    #[test]
    fn invite_code_accepts_both_url_forms() {
        assert_eq!(invite_code("https://discord.gg/abc"), Some("abc".into()));
        assert_eq!(
            invite_code("https://discord.com/invite/abc?event=1"),
            Some("abc".into())
        );
        assert_eq!(invite_code("https://example.com/abc"), None);
    }

    #[test]
    fn lines_without_invites_are_skipped() {
        assert!(parse_link("no link here").is_none());
        assert!(parse_link("[__Name__](https://example.com)").is_some());
        assert!(invite_code("https://example.com").is_none());
    }
}
