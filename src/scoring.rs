//! Deterministic, explainable relevance scoring.
//!
//! Pure functions only: no database, no clock, no randomness. That makes the
//! score reproducible and testable, and the breakdown explains every point.

use serde::{Deserialize, Serialize};

use crate::models::PromotionPolicy;

/// One component of a score, suitable for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreFactor {
    pub factor: String,
    pub points: i64,
    pub max: i64,
    pub reason: String,
}

/// The full result of scoring one community.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Score {
    pub total: i64,
    pub factors: Vec<ScoreFactor>,
    pub unsuitable_for_outreach: bool,
    pub outreach_note: String,
}

const TOPIC_MAX: i64 = 50;
const AUDIENCE_MAX: i64 = 20;
const DESCRIPTION_MAX: i64 = 15;
const EVIDENCE_MAX: i64 = 15;

const STOP_WORDS: &[&str] = &[
    "a",
    "an",
    "and",
    "for",
    "in",
    "of",
    "on",
    "the",
    "to",
    "with",
    "discord",
    "server",
    "servers",
    "community",
    "communities",
    "chat",
    "group",
    "groups",
];

const AUDIENCE_WORDS: &[&str] = &[
    "student",
    "students",
    "freelance",
    "freelancer",
    "freelancers",
    "entrepreneur",
    "entrepreneurs",
    "startup",
    "side hustle",
    "remote",
    "career",
    "careers",
    "job",
    "jobs",
    "internship",
    "learning",
    "beginner",
    "beginners",
];

const OPPORTUNITY_WORDS: &[&str] = &[
    "opportunit",
    "hiring",
    "job",
    "jobs",
    "career",
    "work",
    "earn",
    "income",
    "money",
    "freelanc",
    "remote",
    "internship",
    "networking",
    "mentor",
    "advice",
    "help",
    "support",
];

/// Everything scoring needs to know about a community.
pub struct ScoreInput<'a> {
    pub query: Option<&'a str>,
    pub name: &'a str,
    pub description: &'a str,
    pub tags: &'a [String],
    pub promotion_policy: PromotionPolicy,
    pub policy_evidence: Option<&'a str>,
    pub user_notes: &'a str,
}

/// Splits text into lowercase alphanumeric words, dropping stop words.
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 2)
        .filter(|w| !STOP_WORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

fn query_terms(query: Option<&str>) -> Vec<String> {
    match query {
        Some(q) => {
            let mut terms = tokenize(q);
            terms.sort();
            terms.dedup();
            terms
        }
        None => Vec::new(),
    }
}

/// Scores a community from 0 to 100 with a factor breakdown.
pub fn score(input: &ScoreInput<'_>) -> Score {
    let mut factors = Vec::with_capacity(4);
    let terms = query_terms(input.query);

    let name_tokens = tokenize(input.name);
    let tag_tokens: Vec<String> = input.tags.iter().flat_map(|t| tokenize(t)).collect();
    let description_tokens = tokenize(input.description);

    // ---- topic match (max 50) -------------------------------------------
    let topic_points = if terms.is_empty() {
        factors.push(ScoreFactor {
            factor: "topic".to_string(),
            points: 0,
            max: TOPIC_MAX,
            reason: "No search terms supplied.".to_string(),
        });
        0
    } else {
        let mut matched: Vec<&str> = Vec::new();
        let mut weighted = 0i64;
        for term in &terms {
            // A name or tag hit is worth more than a description hit.
            let in_name = name_tokens.iter().any(|t| t == term);
            let in_tags = tag_tokens.iter().any(|t| t == term);
            let in_description = description_tokens.iter().any(|t| t == term);

            if in_name {
                weighted += 3;
                matched.push(term);
            } else if in_tags {
                weighted += 2;
                matched.push(term);
            } else if in_description {
                weighted += 1;
                matched.push(term);
            }
        }
        // 3 points per term is a full name match; scale so all terms matched by
        // name reaches the maximum.
        let possible = 3 * terms.len() as i64;
        let points = if possible == 0 {
            0
        } else {
            ((weighted as f64 / possible as f64) * TOPIC_MAX as f64).round() as i64
        }
        .clamp(0, TOPIC_MAX);

        let reason = if matched.is_empty() {
            format!(
                "None of the search terms appear in the name, tags or description ({:?}).",
                terms
            )
        } else {
            format!("Matched search terms: {}.", matched.join(", "))
        };
        factors.push(ScoreFactor {
            factor: "topic".to_string(),
            points,
            max: TOPIC_MAX,
            reason,
        });
        points
    };

    // ---- audience match (max 20) ----------------------------------------
    let haystack = format!(
        "{} {} {}",
        input.name.to_lowercase(),
        input.description.to_lowercase(),
        input.tags.join(" ").to_lowercase()
    );
    let mut audience_hits: Vec<&str> = Vec::new();
    for word in AUDIENCE_WORDS {
        if haystack.contains(word) && !audience_hits.iter().any(|h| h.contains(word)) {
            audience_hits.push(word);
        }
    }
    let audience_points = (audience_hits.len() as i64 * 7).clamp(0, AUDIENCE_MAX);
    let audience_reason = if audience_hits.is_empty() {
        "No audience keywords found in the listing.".to_string()
    } else {
        format!("Audience keywords: {}.", audience_hits.join(", "))
    };
    factors.push(ScoreFactor {
        factor: "audience".to_string(),
        points: audience_points,
        max: AUDIENCE_MAX,
        reason: audience_reason,
    });

    // ---- description relevance (max 15) ---------------------------------
    let mut opportunity_hits: Vec<&str> = Vec::new();
    for word in OPPORTUNITY_WORDS {
        if haystack.contains(word) && !opportunity_hits.iter().any(|h| h.starts_with(word)) {
            opportunity_hits.push(word);
        }
    }
    let description_points = (opportunity_hits.len() as i64 * 5).clamp(0, DESCRIPTION_MAX);
    factors.push(ScoreFactor {
        factor: "description".to_string(),
        points: description_points,
        max: DESCRIPTION_MAX,
        reason: if opportunity_hits.is_empty() {
            "Description does not mention work, careers or opportunities.".to_string()
        } else {
            format!("Description mentions: {}.", opportunity_hits.join(", "))
        },
    });

    // ---- explicit evidence that promotion is allowed (max 15) -----------
    let evidence_text = input
        .policy_evidence
        .map(|e| e.trim())
        .filter(|e| !e.is_empty());
    let evidence_points = match (input.promotion_policy, evidence_text) {
        (PromotionPolicy::Allowed, Some(_)) => EVIDENCE_MAX,
        (PromotionPolicy::Allowed, None) => 0,
        _ => 0,
    };
    factors.push(ScoreFactor {
        factor: "policy_evidence".to_string(),
        points: evidence_points,
        max: EVIDENCE_MAX,
        reason: match (input.promotion_policy, evidence_text) {
            (PromotionPolicy::Allowed, Some(_)) => {
                "The community's own rules were recorded as permitting promotion.".to_string()
            }
            (PromotionPolicy::Allowed, None) => {
                "Marked as allowed, but no quoted evidence was recorded.".to_string()
            }
            (PromotionPolicy::Prohibited, _) => {
                "The community's rules prohibit promotion, so no points are awarded.".to_string()
            }
            (PromotionPolicy::Unknown, _) => {
                "Rules not verified, so permission points cannot be awarded.".to_string()
            }
        },
    });

    let subtotal = topic_points + audience_points + description_points + evidence_points;

    // ---- hard overrides -------------------------------------------------
    let (cap, unsuitable_for_outreach, outreach_note) = match input.promotion_policy {
        PromotionPolicy::Prohibited => (
            // A prohibition outranks any topical match.
            Some(20),
            true,
            "This community's rules prohibit promotion or recruitment. Do not post here."
                .to_string(),
        ),
        PromotionPolicy::Unknown => (
            None,
            false,
            "Rules are not verified. Check the community's posted rules before posting."
                .to_string(),
        ),
        PromotionPolicy::Allowed => (
            None,
            false,
            if evidence_text.is_some() {
                "Recorded evidence says promotion is allowed. Confirm before posting.".to_string()
            } else {
                "Marked allowed without quoted evidence. Re-check the rules before posting."
                    .to_string()
            },
        ),
    };

    let mut total = subtotal;
    if let Some(cap) = cap {
        if subtotal > cap {
            // Record the deduction as its own factor so the breakdown still sums
            // to the total and the cap is visible rather than a silent surprise.
            factors.push(ScoreFactor {
                factor: "policy_cap".to_string(),
                points: cap - subtotal,
                max: 0,
                reason: format!(
                    "The community prohibits promotion, so the score is capped at {cap} (subtotal {subtotal})."
                ),
            });
            total = cap;
        }
    }

    let _ = input.user_notes;

    Score {
        total: total.clamp(0, 100),
        factors,
        unsuitable_for_outreach,
        outreach_note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        query: Option<&'a str>,
        name: &'a str,
        description: &'a str,
        tags: &'a [String],
        policy: PromotionPolicy,
        evidence: Option<&'a str>,
    ) -> ScoreInput<'a> {
        ScoreInput {
            query,
            name,
            description,
            tags,
            promotion_policy: policy,
            policy_evidence: evidence,
            user_notes: "",
        }
    }

    #[test]
    fn exact_name_match_scores_high() {
        let tags = vec!["students".to_string()];
        let s = score(&input(
            Some("students"),
            "Students Hub",
            "A place for students to talk about careers and remote work.",
            &tags,
            PromotionPolicy::Allowed,
            Some("Promotion is allowed in #promos"),
        ));
        assert!(s.total >= 80, "expected a high score, got {}", s.total);
    }

    #[test]
    fn unrelated_community_scores_low() {
        let s = score(&input(
            Some("freelancing"),
            "Retro Game Collectors",
            "We talk about cartridges.",
            &[],
            PromotionPolicy::Unknown,
            None,
        ));
        assert!(s.total <= 15, "expected a low score, got {}", s.total);
    }

    #[test]
    fn unknown_policy_never_earns_evidence_points() {
        let score_without = score(&input(
            Some("students"),
            "Student Lounge",
            "students helping students",
            &[],
            PromotionPolicy::Unknown,
            None,
        ));
        assert_eq!(
            score_without
                .factors
                .iter()
                .find(|f| f.factor == "policy_evidence")
                .unwrap()
                .points,
            0
        );

        // Even if a user pastes evidence text, unknown policy earns nothing.
        let score_with_text = score(&input(
            Some("students"),
            "Student Lounge",
            "students helping students",
            &[],
            PromotionPolicy::Unknown,
            Some("some text"),
        ));
        assert_eq!(
            score_with_text
                .factors
                .iter()
                .find(|f| f.factor == "policy_evidence")
                .unwrap()
                .points,
            0
        );
    }

    #[test]
    fn prohibited_policy_forces_unsuitable_and_caps_score() {
        let s = score(&input(
            Some("students making money online"),
            "Students Making Money Online Entrepreneurship",
            "students entrepreneurship side hustle",
            &["students".to_string(), "entrepreneurship".to_string()],
            PromotionPolicy::Prohibited,
            Some("No advertising or referral links"),
        ));
        assert!(s.unsuitable_for_outreach);
        assert!(s.total <= 20, "prohibited must be capped, got {}", s.total);
        assert!(s.outreach_note.to_lowercase().contains("prohibit"));
    }

    #[test]
    fn allowed_with_evidence_earns_full_policy_points() {
        let s = score(&input(
            Some("students"),
            "Student Hub",
            "students and study",
            &[],
            PromotionPolicy::Allowed,
            Some("Promotion allowed in #promotions"),
        ));
        let f = s
            .factors
            .iter()
            .find(|f| f.factor == "policy_evidence")
            .unwrap();
        assert_eq!(f.points, EVIDENCE_MAX);
        assert!(!s.unsuitable_for_outreach);
    }

    #[test]
    fn allowed_without_evidence_earns_nothing() {
        let s = score(&input(
            Some("students"),
            "Student Hub",
            "students and study",
            &[],
            PromotionPolicy::Allowed,
            None,
        ));
        let f = s
            .factors
            .iter()
            .find(|f| f.factor == "policy_evidence")
            .unwrap();
        assert_eq!(f.points, 0);
    }

    #[test]
    fn scoring_is_deterministic() {
        let tags = vec!["students".to_string()];
        let a = score(&input(
            Some("students money"),
            "Money Students",
            "students earning",
            &tags,
            PromotionPolicy::Allowed,
            Some("ok"),
        ));
        let b = score(&input(
            Some("students money"),
            "Money Students",
            "students earning",
            &tags,
            PromotionPolicy::Allowed,
            Some("ok"),
        ));
        assert_eq!(a, b);
    }

    #[test]
    fn no_query_yields_zero_topic_points() {
        let s = score(&input(
            None,
            "Anything",
            "whatever",
            &[],
            PromotionPolicy::Unknown,
            None,
        ));
        let f = s.factors.iter().find(|f| f.factor == "topic").unwrap();
        assert_eq!(f.points, 0);
    }

    #[test]
    fn breakdown_factors_always_sum_to_total() {
        let s = score(&input(
            Some("students"),
            "Student Hub",
            "students and careers",
            &[],
            PromotionPolicy::Allowed,
            Some("allowed"),
        ));
        assert_eq!(s.factors.len(), 4);
        let sum: i64 = s.factors.iter().map(|f| f.points).sum();
        assert_eq!(sum, s.total);
    }

    #[test]
    fn prohibited_cap_appears_as_an_explicit_factor() {
        let s = score(&input(
            Some("students entrepreneurship money online"),
            "Students Entrepreneurship Money Online",
            "students money side hustle freelance remote career",
            &["students".to_string(), "entrepreneurship".to_string()],
            PromotionPolicy::Prohibited,
            Some("No promotion"),
        ));
        let cap = s
            .factors
            .iter()
            .find(|f| f.factor == "policy_cap")
            .expect("cap factor must be present when the raw score exceeds the cap");
        assert!(cap.points < 0, "cap deduction must be negative");
        let sum: i64 = s.factors.iter().map(|f| f.points).sum();
        assert_eq!(sum, s.total);
        assert_eq!(s.total, 20);
    }
}
