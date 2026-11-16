//! Community discovery sources.
//!
//! Only sources whose terms permit automated access are registered as enabled.
//! Everything else is listed as unavailable so the UI can be honest about it
//! instead of pretending search works everywhere.

pub mod awesome;

/// Description of a source, including whether it can actually be used.
pub struct SourceInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub homepage: &'static str,
    pub license: &'static str,
    pub enabled: bool,
    pub notes: &'static str,
}

/// The complete list considered in the audit, with current availability.
pub fn registry() -> Vec<SourceInfo> {
    vec![
        SourceInfo {
            id: awesome::SOURCE_ID,
            name: awesome::SOURCE_NAME,
            homepage: awesome::REPO_URL,
            license: "CC-BY-4.0",
            enabled: true,
            notes: "Curated public list read through the GitHub REST API. Attribution shown on every record.",
        },
        SourceInfo {
            id: "topgg",
            name: "Top.gg",
            homepage: "https://top.gg/",
            license: "Proprietary",
            enabled: false,
            notes: "No documented server-search API; the documented endpoints cover bots and need a project token.",
        },
        SourceInfo {
            id: "disboard",
            name: "DISBOARD",
            homepage: "https://disboard.org/",
            license: "Proprietary",
            enabled: false,
            notes: "No public API; automated requests are blocked (HTTP 403). Not scraped.",
        },
        SourceInfo {
            id: "discordme",
            name: "Discord.me",
            homepage: "https://discord.me/",
            license: "Proprietary",
            enabled: false,
            notes: "No documented API and no retrievable terms.",
        },
        SourceInfo {
            id: "discordservers",
            name: "DiscordServers.com",
            homepage: "https://discordservers.com/",
            license: "Proprietary",
            enabled: false,
            notes: "No documented API was found.",
        },
        SourceInfo {
            id: "discord-discovery",
            name: "Discord Server Discovery",
            homepage: "https://discord.com/servers",
            license: "Proprietary",
            enabled: false,
            notes: "Requires an authenticated session; scraping it is prohibited by Discord's Developer Policy.",
        },
    ]
}

/// A parsed listing from any source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub name: String,
    pub invite_url: String,
    pub invite_code: String,
    pub category: String,
    pub subcategory: String,
    pub listing_url: String,
    pub tags: Vec<String>,
}

impl Discovered {
    /// Short description built only from real source metadata.
    pub fn description(&self) -> String {
        if self.subcategory.is_empty() {
            format!("Listed under {}.", self.category)
        } else {
            format!("Listed under {} / {}.", self.category, self.subcategory)
        }
    }
}
