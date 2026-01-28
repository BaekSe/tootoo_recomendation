use crate::domain::contract::LlmRecommendationSnapshot;
use crate::domain::recommendation::RecommendationSnapshot;
use anyhow::Context;

pub fn extract_json(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.starts_with("```") {
        // Remove Markdown fences (```json ... ``` or ``` ... ```).
        let mut inner = trimmed;
        if let Some(after_first) = inner.splitn(2, '\n').nth(1) {
            inner = after_first;
        }
        if let Some(end) = inner.rfind("```") {
            inner = &inner[..end];
        }
        return Some(inner.trim().to_string());
    }

    // Best-effort extraction: first '{' to last '}'.
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(trimmed[start..=end].trim().to_string())
}

pub fn parse_snapshot(
    text: &str,
    expected_as_of_date: chrono::NaiveDate,
) -> anyhow::Result<RecommendationSnapshot> {
    let json_str = extract_json(text).unwrap_or_else(|| text.trim().to_string());
    let parsed = serde_json::from_str::<LlmRecommendationSnapshot>(&json_str)
        .with_context(|| format!("LLM output is not valid JSON for snapshot schema: {json_str}"))?;
    parsed.validate_and_into_snapshot(expected_as_of_date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};
    use serde_json::json;

    fn valid_snapshot_json(as_of: NaiveDate) -> String {
        let generated_at = Utc.with_ymd_and_hms(2026, 1, 27, 10, 0, 0).unwrap();
        let items: Vec<_> = (1..=20)
            .map(|rank| {
                json!({
                    "rank": rank,
                    "ticker": format!("KRX:{rank:06}"),
                    "name": format!("Name {rank}"),
                    "rationale": ["a", "b", "c"],
                    "risk_notes": null,
                    "confidence": 0.5,
                })
            })
            .collect();

        json!({
            "as_of_date": as_of,
            "generated_at": generated_at,
            "items": items,
        })
        .to_string()
    }

    #[test]
    fn extract_json_handles_fenced_blocks() {
        let body = "{\"a\":1}";
        let fenced = format!("```json\n{body}\n```\n");
        assert_eq!(extract_json(&fenced), Some(body.to_string()));
    }

    #[test]
    fn extract_json_falls_back_to_braces() {
        let s = "prefix {\"a\":1} suffix";
        assert_eq!(extract_json(s), Some("{\"a\":1}".to_string()));
    }

    #[test]
    fn parse_snapshot_accepts_valid_json() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 27).unwrap();
        let json = valid_snapshot_json(as_of);
        let snapshot = parse_snapshot(&json, as_of).unwrap();
        assert_eq!(snapshot.items.len(), 20);
        assert_eq!(snapshot.as_of_date, as_of);
        assert_eq!(snapshot.items[0].rank, 1);
    }

    #[test]
    fn parse_snapshot_rejects_wrong_as_of_date() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 27).unwrap();
        let other = NaiveDate::from_ymd_opt(2026, 1, 26).unwrap();
        let json = valid_snapshot_json(other);
        assert!(parse_snapshot(&json, as_of).is_err());
    }

    #[test]
    fn parse_snapshot_rejects_wrong_item_count() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 27).unwrap();
        let generated_at = Utc.with_ymd_and_hms(2026, 1, 27, 10, 0, 0).unwrap();
        let json = json!({
            "as_of_date": as_of,
            "generated_at": generated_at,
            "items": [],
        })
        .to_string();
        assert!(parse_snapshot(&json, as_of).is_err());
    }

    #[test]
    fn parse_snapshot_accepts_missing_optional_keys() {
        let as_of = NaiveDate::from_ymd_opt(2026, 1, 27).unwrap();
        let generated_at = Utc.with_ymd_and_hms(2026, 1, 27, 10, 0, 0).unwrap();
        let items: Vec<_> = (1..=20)
            .map(|rank| {
                // risk_notes and confidence are optional.
                json!({
                    "rank": rank,
                    "ticker": format!("KRX:{rank:06}"),
                    "name": format!("Name {rank}"),
                    "rationale": ["a", "b", "c"],
                })
            })
            .collect();

        let json = json!({
            "as_of_date": as_of,
            "generated_at": generated_at,
            "items": items,
        })
        .to_string();

        let snapshot = parse_snapshot(&json, as_of).unwrap();
        assert_eq!(snapshot.items.len(), 20);
    }
}
