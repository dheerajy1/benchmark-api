use std::collections::BTreeMap;

// What a header value looks like in debug output
pub const REDACTED: &str = "<redacted>";

// Header names with every value replaced, for a map of headers
pub fn map_names(headers: &BTreeMap<String, String>) -> BTreeMap<&str, &'static str> {
    headers
        .keys()
        .map(|name| (name.as_str(), REDACTED))
        .collect()
}

// Header names with every value replaced, for a list of headers
pub fn pair_names(headers: &[(String, String)]) -> Vec<(&str, &'static str)> {
    headers
        .iter()
        .map(|(name, _)| (name.as_str(), REDACTED))
        .collect()
}
