//! Small helpers for digging through Innertube JSON.
use serde_json::Value;

/// Extracts text from {simpleText}, {runs:[{text}]}, {content}, or a plain string.
pub fn text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(runs) = v.get("runs").and_then(Value::as_array) {
        return runs
            .iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect::<String>();
    }
    if let Some(s) = v.get("content").and_then(Value::as_str) {
        return s.to_string();
    }
    String::new()
}

pub fn opt_text(v: Option<&Value>) -> Option<String> {
    v.map(text).filter(|s| !s.is_empty())
}

pub fn s(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr).and_then(Value::as_str).map(str::to_string)
}

/// Depth-first search for the first value under `key`.
pub fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if let Some(x) = m.get(key) {
                return Some(x);
            }
            m.values().find_map(|c| find(c, key))
        }
        Value::Array(a) => a.iter().find_map(|c| find(c, key)),
        _ => None,
    }
}

pub fn find_all<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                if k == key {
                    out.push(c);
                } else {
                    find_all(c, key, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|c| find_all(c, key, out)),
        _ => {}
    }
}

/// Renderers that only ever hold ads or paid promotions. Their contents look like
/// ordinary videos (an ad's own video, usually unlisted and unplayable), so the
/// whole subtree is skipped.
pub const AD_RENDERERS: &[&str] = &[
    "adSlotRenderer",
    "inFeedAdLayoutRenderer",
    "adSlotAndLayoutMetadata",
    "promotedVideoRenderer",
    "compactPromotedVideoRenderer",
    "promotedSparklesWebRenderer",
    "promotedSparklesTextSearchRenderer",
    "searchPyvRenderer",
    "bannerPromoRenderer",
    "statementBannerRenderer",
    "brandVideoShelfRenderer",
    "brandVideoSingletonRenderer",
    "videoMastheadAdV3Renderer",
    "mastheadAdV4Renderer",
    "primetimePromoRenderer",
    "playerLegacyDesktopWatchAdsRenderer",
    "adBreakServiceRenderer",
    "displayAdRenderer",
    "actionCompanionAdRenderer",
    "feedAdMetadataViewModel",
];

/// Fields that only appear on ads, also when YouTube draws an ad with a regular
/// video renderer or lockup.
const AD_MARKERS: &[&str] = &["adSlotLoggingData", "adLayoutLoggingData", "adClientParams", "adVideoId", "promotedVideoRenderer"];

/// Whether a renderer is an ad: it carries an ad-only field or an "Ad"/"Sponsored" badge.
pub fn is_ad(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, c)| {
            AD_MARKERS.contains(&k.as_str())
                || AD_RENDERERS.contains(&k.as_str())
                || ((k == "style" || k == "badgeStyle") && matches!(c.as_str(), Some("BADGE_STYLE_TYPE_AD" | "BADGE_STYLE_TYPE_AD_INFO")))
                || (k == "iconType" && c.as_str() == Some("AD_BADGE"))
                || is_ad(c)
        }),
        Value::Array(a) => a.iter().any(is_ad),
        _ => false,
    }
}

/// Largest thumbnail url in a {thumbnails:[...]} or {sources:[...]} object.
pub fn best_thumb(v: &Value) -> Option<String> {
    let arr = v
        .get("thumbnails")
        .or_else(|| v.get("sources"))
        .and_then(Value::as_array)?;
    let w = |t: &&Value| t.get("width").and_then(Value::as_u64).unwrap_or(0);
    let h = |t: &&Value| t.get("height").and_then(Value::as_u64).unwrap_or(0);
    // big enough for a card on a high-DPI screen, and no bigger: every image is
    // kept decoded in memory (portrait Shorts covers count by height)
    let best = arr
        .iter()
        .filter(|t| w(t) >= 600 || h(t) >= 700)
        .min_by_key(|t| w(t) * h(t).max(1))
        .or_else(|| arr.iter().max_by_key(w))?;
    best.get("url").and_then(Value::as_str).map(fix_url).map(|u| limit_avatar(&u))
}

#[cfg(test)]
mod avatar_tests {
    #[test]
    fn avatar_size() {
        assert_eq!(super::limit_avatar("https://yt3.ggpht.com/abc=s900-c-k-c0x00ffffff-no-rj"), "https://yt3.ggpht.com/abc=s240-c-k-c0x00ffffff-no-rj");
        assert_eq!(super::limit_avatar("https://yt3.ggpht.com/abc=s88-c-k"), "https://yt3.ggpht.com/abc=s88-c-k");
        assert_eq!(super::limit_avatar("https://i.ytimg.com/vi/x/hq720.jpg"), "https://i.ytimg.com/vi/x/hq720.jpg");
    }
}

pub fn small_thumb(v: &Value) -> Option<String> {
    let arr = v
        .get("thumbnails")
        .or_else(|| v.get("sources"))
        .and_then(Value::as_array)?;
    arr.first()
        .and_then(|t| t.get("url"))
        .and_then(Value::as_str)
        .map(fix_url)
}

/// Channel avatars come as 900x900 but are drawn at 24-160 px: ask for 240 px.
pub fn limit_avatar(u: &str) -> String {
    if !(u.contains("yt3.ggpht.com") || u.contains("yt3.googleusercontent.com")) {
        return u.to_string();
    }
    // "...=s900-c-k-c0x00ffffff-no-rj" -> "...=s240-c-k-c0x00ffffff-no-rj"
    if let Some(i) = u.rfind("=s") {
        let rest = &u[i + 2..];
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && rest[..digits].parse::<u32>().map(|n| n > 240).unwrap_or(false) {
            return format!("{}=s240{}", &u[..i], &rest[digits..]);
        }
    }
    u.to_string()
}

pub fn fix_url(u: &str) -> String {
    if let Some(r) = u.strip_prefix("//") {
        format!("https://{r}")
    } else {
        u.to_string()
    }
}

/// "1:02:03" -> 3723
pub fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = 0u64;
    for part in s.split(':') {
        let n: u64 = part.trim().parse().ok()?;
        total = total * 60 + n;
    }
    Some(total)
}

pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Converts "3 days ago" / "3d ago" / "Streamed 2 weeks ago" into an approximate unix timestamp.
pub fn parse_relative_time(s: &str) -> Option<i64> {
    let lower = s.to_lowercase();
    let lower = lower.replace("streamed", "").replace("premiered", "");
    let mut words = lower.split_whitespace();
    let first = words.next()?;
    // split leading digits from unit (compact form like "17y", "2mo")
    let digits: String = first.chars().take_while(|c| c.is_ascii_digit()).collect();
    let (n, unit) = if digits.is_empty() {
        return None;
    } else if digits.len() < first.len() {
        (digits.parse::<i64>().ok()?, first[digits.len()..].to_string())
    } else {
        (digits.parse::<i64>().ok()?, words.next().unwrap_or("").to_string())
    };
    let secs = if unit.starts_with("mo") {
        30 * 86400
    } else if unit.starts_with("sec") || unit == "s" {
        1
    } else if unit.starts_with("min") || unit == "m" {
        60
    } else if unit.starts_with('h') {
        3600
    } else if unit.starts_with('d') {
        86400
    } else if unit.starts_with('w') {
        7 * 86400
    } else if unit.starts_with('y') {
        365 * 86400
    } else {
        return None;
    };
    Some(now() - n * secs)
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn relative_from_ts(ts: i64) -> String {
    let d = (now() - ts).max(0);
    let (n, u) = if d < 60 {
        return "just now".into();
    } else if d < 3600 {
        (d / 60, "minute")
    } else if d < 86400 {
        (d / 3600, "hour")
    } else if d < 7 * 86400 {
        (d / 86400, "day")
    } else if d < 30 * 86400 {
        (d / (7 * 86400), "week")
    } else if d < 365 * 86400 {
        (d / (30 * 86400), "month")
    } else {
        (d / (365 * 86400), "year")
    };
    format!("{n} {u}{} ago", if n == 1 { "" } else { "s" })
}

pub fn compact_number(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => trim_dec(n as f64 / 1e3, "K"),
        1_000_000..=999_999_999 => trim_dec(n as f64 / 1e6, "M"),
        _ => trim_dec(n as f64 / 1e9, "B"),
    }
}

fn trim_dec(v: f64, suf: &str) -> String {
    if v >= 100.0 { format!("{:.0}{suf}", v) } else { format!("{:.1}{suf}", v).replace(".0", "") }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rel() {
        let n = now();
        assert!((parse_relative_time("17y ago").unwrap() - (n - 17 * 365 * 86400)).abs() < 5);
        assert!((parse_relative_time("2mo ago").unwrap() - (n - 60 * 86400)).abs() < 5);
        assert!((parse_relative_time("3 days ago").unwrap() - (n - 3 * 86400)).abs() < 5);
        assert!((parse_relative_time("Streamed 1 hour ago").unwrap() - (n - 3600)).abs() < 5);
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(compact_number(1_234_567), "1.2M");
    }
}
