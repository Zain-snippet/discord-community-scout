//! Deterministic message drafting.
//!
//! No AI API and no key: the drafts are assembled from fixed templates plus the
//! community's verified metadata. Every template keeps the four mandatory
//! disclosures (remote, commission-based, $12 deposit, conditional recovery) and
//! the invitation to ask for details. Tests assert both that those disclosures
//! are present and that prohibited claims never appear.

use serde::{Deserialize, Serialize};

use crate::models::PromotionPolicy;

/// Opening line style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// Warm and personal, the default.
    Conversational,
    /// Shorter, plainer.
    Direct,
}

impl Tone {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "conversational" | "natural" | "friendly" => Some(Tone::Conversational),
            "direct" | "plain" | "concise" => Some(Tone::Direct),
            _ => None,
        }
    }
}

/// Which draft to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// Single short post, default.
    Concise,
    /// A longer version, offered as an alternative.
    Long,
}

impl Variant {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "concise" | "short" => Some(Variant::Concise),
            "long" | "detailed" => Some(Variant::Long),
            _ => None,
        }
    }
}

pub struct DraftInput<'a> {
    pub community_name: &'a str,
    pub tags: &'a [String],
    pub policy: PromotionPolicy,
    pub policy_evidence: Option<&'a str>,
    pub channel_name: Option<&'a str>,
    pub tone: Tone,
    pub variant: Variant,
    pub user_notes: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct Draft {
    pub variant: String,
    pub text: String,
    /// False when the community's rules forbid this post.
    pub can_post: bool,
    pub warnings: Vec<String>,
    pub disclosures: Vec<String>,
    /// Where to post, only when the user recorded a verified channel.
    pub posting_target: Option<String>,
}

/// The audience phrase derived from the community's own tags, if any match.
/// Only uses vocabulary present in the record; never invents an audience.
fn audience_phrase(tags: &[String], name: &str) -> Option<String> {
    let haystack = format!("{} {}", name.to_lowercase(), tags.join(" ").to_lowercase());

    const MAP: &[(&str, &str)] = &[
        ("student", "students"),
        ("universit", "students"),
        ("college", "students"),
        ("freelanc", "freelancers"),
        ("entrepreneur", "entrepreneurs and founders"),
        ("startup", "people building startups"),
        ("side hustle", "people working on side hustles"),
        ("remote", "people interested in remote work"),
        ("career", "people thinking about their careers"),
        ("programmer", "developers"),
        ("developer", "developers"),
        ("coding", "developers"),
        ("programming", "developers"),
        ("learning", "people who are learning"),
        ("beginner", "beginners"),
    ];

    for (needle, phrase) in MAP {
        if haystack.contains(needle) {
            return Some((*phrase).to_string());
        }
    }
    None
}

pub fn draft(input: &DraftInput<'_>) -> Draft {
    let audience = audience_phrase(input.tags, input.community_name);

    let topic_line = match &audience {
        Some(audience) => format!(
            "I know how tough it can be for {audience} to find something genuinely worth looking into,",
        ),
        None => "I know how tough it can be to find something genuinely worth looking into,"
            .to_string(),
    };

    let opener = match input.tone {
        Tone::Conversational => format!(
            "Hey everyone! Hope you're all doing well. I've been exploring ways to earn online as a student, \
             and {topic_line} so I wanted to share something that came up."
        ),
        Tone::Direct => format!(
            "Hi all — I've been looking into ways to earn online as a student, and {topic_line} \
             so I wanted to mention this here."
        ),
    };

    let core = "The company I've been working with has opened remote, commission-based positions. \
                Just to be upfront, there's a $12 initial deposit, which is recoverable after your \
                first payment, subject to the company's terms.";

    let closing = "If you'd like to know more, I'm happy to share the details — no pressure, and it's \
                   fine to just ask questions first. Whether it's worth pursuing really depends on your \
                   own dedication and commitment.";

    let mut text = match input.variant {
        Variant::Concise => format!("{opener} {core} {closing}"),
        Variant::Long => format!(
            "{opener}\n\nThe internet is flooded with information and opportunities, and figuring out \
             which ones are actually worth pursuing can get overwhelming.\n\n{core} I know first-hand \
             how hard it can be to find a decent way to make some money online, so I thought I'd pass \
             this along for anyone it might suit.\n\n{closing}"
        ),
    };

    // A verified channel name is only ever used as a posting target, never
    // inserted into the message body as an invented instruction.
    if let Some(notes) = input.user_notes.map(str::trim).filter(|n| !n.is_empty()) {
        text.push_str(&format!("\n\n(Your note for reference: {notes})"));
    }

    let disclosures = vec![
        "Remote".to_string(),
        "Commission-based, not salaried".to_string(),
        "$12 initial deposit disclosed".to_string(),
        "Deposit recoverable after the first payment, subject to the company's terms".to_string(),
        "Invitation to ask for details".to_string(),
    ];

    let mut warnings = Vec::new();
    let mut can_post = true;

    match input.policy {
        PromotionPolicy::Prohibited => {
            can_post = false;
            warnings.push(
                "This community's rules prohibit promotion or recruitment. Do not post this here."
                    .to_string(),
            );
        }
        PromotionPolicy::Unknown => {
            warnings.push(
                "This community's rules are not verified. Read its posted rules before posting."
                    .to_string(),
            );
        }
        PromotionPolicy::Allowed => {
            if input.policy_evidence.unwrap_or("").trim().is_empty() {
                warnings.push(
                    "Marked as allowed, but no quoted rule was recorded. Re-check before posting."
                        .to_string(),
                );
            }
        }
    }

    warnings.push(
        "This is a draft. Review it yourself, confirm it matches the community's rules, and post it \
         manually if appropriate."
            .to_string(),
    );

    let posting_target = match (input.policy, input.channel_name) {
        (PromotionPolicy::Prohibited, _) => None,
        (_, Some(channel)) if !channel.trim().is_empty() => Some(channel.trim().to_string()),
        _ => None,
    };

    Draft {
        variant: match input.variant {
            Variant::Concise => "concise".to_string(),
            Variant::Long => "long".to_string(),
        },
        text,
        can_post,
        warnings,
        disclosures,
        posting_target,
    }
}

/// Claims that must never appear in a generated draft.
pub const FORBIDDEN_PHRASES: &[&str] = &[
    "salary",
    "salaried",
    "guaranteed income",
    "guaranteed earnings",
    "guarantee you",
    "risk-free",
    "risk free",
    "guaranteed refund",
    "full-time job",
    "hiring now",
    "immediate start",
    "no risk",
    "get rich",
    "passive income",
];

/// Returns any forbidden phrase found in `text` (case-insensitive).
pub fn forbidden_claims(text: &str) -> Vec<&'static str> {
    let lowered = text.to_lowercase();
    FORBIDDEN_PHRASES
        .iter()
        .filter(|phrase| lowered.contains(*phrase))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(policy: PromotionPolicy, variant: Variant) -> DraftInput<'a> {
        DraftInput {
            community_name: "Students Hub",
            tags: &[],
            policy,
            policy_evidence: None,
            channel_name: None,
            tone: Tone::Conversational,
            variant,
            user_notes: None,
        }
    }

    fn assert_disclosures(text: &str) {
        let lowered = text.to_lowercase();
        assert!(lowered.contains("remote"), "missing 'remote': {text}");
        assert!(
            lowered.contains("commission"),
            "missing 'commission-based': {text}"
        );
        assert!(lowered.contains("$12"), "missing '$12': {text}");
        assert!(
            lowered.contains("recoverable after your first payment"),
            "missing conditional recovery: {text}"
        );
        assert!(
            lowered.contains("subject to the company's terms"),
            "missing company terms qualifier: {text}"
        );
        assert!(
            lowered.contains("ask") || lowered.contains("happy to share"),
            "missing invitation to ask: {text}"
        );
    }

    #[test]
    fn concise_draft_discloses_every_required_fact() {
        let d = draft(&input(PromotionPolicy::Unknown, Variant::Concise));
        assert_disclosures(&d.text);
        assert_eq!(d.variant, "concise");
    }

    #[test]
    fn long_draft_discloses_every_required_fact() {
        let d = draft(&input(PromotionPolicy::Unknown, Variant::Long));
        assert_disclosures(&d.text);
        assert_eq!(d.variant, "long");
    }

    #[test]
    fn neither_variant_contains_a_forbidden_claim() {
        for policy in [
            PromotionPolicy::Allowed,
            PromotionPolicy::Prohibited,
            PromotionPolicy::Unknown,
        ] {
            for variant in [Variant::Concise, Variant::Long] {
                let d = draft(&input(policy, variant));
                assert!(
                    forbidden_claims(&d.text).is_empty(),
                    "forbidden claim in {:?}/{:?}: {:?}",
                    policy,
                    variant,
                    forbidden_claims(&d.text)
                );
            }
        }
    }

    #[test]
    fn prohibited_policy_blocks_posting() {
        let d = draft(&input(PromotionPolicy::Prohibited, Variant::Concise));
        assert!(!d.can_post);
        assert!(d.warnings.iter().any(|w| w.contains("prohibit")));
        assert!(d.posting_target.is_none());
    }

    #[test]
    fn unknown_policy_warns_and_does_not_claim_permission() {
        let d = draft(&input(PromotionPolicy::Unknown, Variant::Concise));
        assert!(
            d.can_post,
            "unknown rules must not be treated as a prohibition claim"
        );
        assert!(d
            .warnings
            .iter()
            .any(|w| w.to_lowercase().contains("not verified")));
        assert!(d.posting_target.is_none());
    }

    #[test]
    fn allowed_policy_without_evidence_still_warns() {
        let d = draft(&input(PromotionPolicy::Allowed, Variant::Concise));
        assert!(d
            .warnings
            .iter()
            .any(|w| w.to_lowercase().contains("re-check")));
    }

    #[test]
    fn audience_phrase_comes_only_from_existing_tags() {
        let tags = vec!["students".to_string()];
        let d = draft(&DraftInput {
            community_name: "Students Hub",
            tags: &tags,
            policy: PromotionPolicy::Allowed,
            policy_evidence: Some("allowed"),
            channel_name: None,
            tone: Tone::Conversational,
            variant: Variant::Concise,
            user_notes: None,
        });
        assert!(d.text.contains("for students to find"));
    }

    #[test]
    fn no_audience_phrase_when_nothing_matches() {
        let tags = vec!["retro gaming".to_string()];
        let d = draft(&DraftInput {
            community_name: "Retro Games",
            tags: &tags,
            policy: PromotionPolicy::Unknown,
            policy_evidence: None,
            channel_name: None,
            tone: Tone::Conversational,
            variant: Variant::Concise,
            user_notes: None,
        });
        assert!(!d.text.contains("for students to find"));
        assert_disclosures(&d.text);
    }

    #[test]
    fn verified_channel_becomes_posting_target_not_invented_text() {
        let d = draft(&DraftInput {
            community_name: "Dev Hub",
            tags: &[],
            policy: PromotionPolicy::Allowed,
            policy_evidence: Some("Post offers in #opportunities"),
            channel_name: Some("#opportunities"),
            tone: Tone::Conversational,
            variant: Variant::Concise,
            user_notes: None,
        });
        assert_eq!(d.posting_target.as_deref(), Some("#opportunities"));
        // The channel name is metadata, not a claim inside the message body.
        assert!(!d.text.contains("#opportunities"));
    }

    #[test]
    fn direct_tone_is_shorter_than_conversational() {
        let tags: Vec<String> = Vec::new();
        let conversational = draft(&DraftInput {
            community_name: "X",
            tags: &tags,
            policy: PromotionPolicy::Unknown,
            policy_evidence: None,
            channel_name: None,
            tone: Tone::Conversational,
            variant: Variant::Concise,
            user_notes: None,
        });
        let direct = draft(&DraftInput {
            community_name: "X",
            tags: &tags,
            policy: PromotionPolicy::Unknown,
            policy_evidence: None,
            channel_name: None,
            tone: Tone::Direct,
            variant: Variant::Concise,
            user_notes: None,
        });
        assert!(direct.text.len() <= conversational.text.len());
    }

    #[test]
    fn forbidden_claim_detector_actually_detects() {
        assert!(forbidden_claims("remote, commission-based, $12 deposit").is_empty());
        assert!(forbidden_claims("this is a salaried job").contains(&"salaried"));
        assert!(forbidden_claims("Guaranteed Income").contains(&"guaranteed income"));
    }

    #[test]
    fn drafts_are_deterministic() {
        let a = draft(&input(PromotionPolicy::Unknown, Variant::Long));
        let b = draft(&input(PromotionPolicy::Unknown, Variant::Long));
        assert_eq!(a.text, b.text);
    }
}
