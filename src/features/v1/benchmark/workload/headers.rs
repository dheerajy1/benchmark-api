use std::collections::BTreeMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

// Validates and converts request headers, values are never echoed in errors
pub fn parse(headers: &BTreeMap<String, String>) -> Result<HeaderMap, String> {
    let mut map = HeaderMap::new();

    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("Invalid header name: {name}"))?;

        let header_value = HeaderValue::from_str(value)
            .map_err(|_| format!("Invalid value for header: {name}"))?;

        map.insert(header_name, header_value);
    }

    Ok(map)
}
