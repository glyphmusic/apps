use glyph_protocol::network::Class;
use std::time::Duration;

pub fn desired(class: Class) -> &'static str {
    match class {
        Class::Slow => "low",
        Class::Standard => "standard",
        Class::Fast => "high",
    }
}
fn rank(q: &str) -> u8 {
    match q {
        "low" => 0,
        "standard" => 1,
        _ => 2,
    }
}
pub fn rewrite(
    url: &str,
    mode: Option<&str>,
    class: Class,
    since: Duration,
) -> Option<(String, bool)> {
    let mut parsed = reqwest::Url::parse(url).ok()?;
    if !parsed.path().contains("/v1/audio/youtube/") {
        return None;
    }
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let current = pairs
        .iter()
        .find(|(k, _)| k == "quality")
        .map(|(_, v)| v.as_str())
        .unwrap_or("high");
    let auto = mode
        .map(|m| m == "auto")
        .unwrap_or_else(|| pairs.iter().any(|(k, v)| k == "adaptive" && v == "1"));
    if mode.is_none() && !auto {
        return None;
    }
    let next = if auto { desired(class) } else { mode? };
    if !matches!(next, "low" | "standard" | "high") {
        return None;
    }
    let changed = current != next;
    if mode.is_none()
        && (!changed
            || since < Duration::from_secs(if rank(next) < rank(current) { 5 } else { 45 }))
    {
        return None;
    }
    parsed.set_query(None);
    parsed
        .query_pairs_mut()
        .extend_pairs(
            pairs
                .iter()
                .filter(|(k, _)| k != "quality" && k != "adaptive")
                .map(|(k, v)| (k, v)),
        )
        .append_pair("quality", next);
    if auto {
        parsed.query_pairs_mut().append_pair("adaptive", "1");
    }
    Some((parsed.into(), changed))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manual_is_binding_and_auto_has_hysteresis() {
        let high = "https://test/v1/audio/youtube/abc?quality=high";
        assert!(rewrite(high, None, Class::Slow, Duration::from_secs(90)).is_none());
        let auto = format!("{high}&adaptive=1");
        assert!(rewrite(&auto, None, Class::Slow, Duration::from_secs(4)).is_none());
        let (low, changed) = rewrite(&auto, None, Class::Slow, Duration::from_secs(5)).unwrap();
        assert!(changed && low.contains("quality=low") && low.contains("adaptive=1"));
        assert!(rewrite(&low, None, Class::Fast, Duration::from_secs(44)).is_none());
        assert!(rewrite(&low, None, Class::Fast, Duration::from_secs(45))
            .unwrap()
            .0
            .contains("quality=high"));
        let forced = rewrite(&low, Some("high"), Class::Slow, Duration::ZERO)
            .unwrap()
            .0;
        assert!(forced.contains("quality=high") && !forced.contains("adaptive="));
    }
}
