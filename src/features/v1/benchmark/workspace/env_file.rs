use std::collections::BTreeMap;

// Parses the body of a .env file into a variable map. Error messages carry the line number
// and the problem, never a value.
pub fn parse(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut variables = BTreeMap::new();

    // lines() already drops a trailing \r\n pair, a lone \r is trimmed below
    for (index, raw_line) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw_line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let line = strip_export(line);

        // Only the first '=' separates, the value may contain more of them
        let Some((name, value)) = line.split_once('=') else {
            return Err(format!("line {number}: expected NAME=value"));
        };

        let name = name.trim();

        if !is_valid_name(name) {
            return Err(format!(
                "line {number}: invalid variable name (letters, digits and _ only, not starting with a digit)"
            ));
        }

        let value = parse_value(value.trim_start())
            .map_err(|message| format!("line {number}: {message}"))?;

        variables.insert(name.to_string(), value);
    }

    Ok(variables)
}

pub fn is_valid_name(name: &str) -> bool {
    let mut characters = name.chars();

    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }

    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

// "export NAME=value" is accepted, "exported=1" is a plain variable
fn strip_export(line: &str) -> &str {
    match line.strip_prefix("export") {
        Some(rest) if rest.starts_with(char::is_whitespace) => rest.trim_start(),
        _ => line,
    }
}

fn parse_value(value: &str) -> Result<String, String> {
    match value.chars().next() {
        Some('"') => parse_double_quoted(&value[1..]),
        Some('\'') => parse_single_quoted(&value[1..]),
        _ => Ok(parse_unquoted(value)),
    }
}

// An unquoted value ends at " #", surrounding whitespace is dropped
fn parse_unquoted(value: &str) -> String {
    let mut end = value.len();
    let mut previous_is_space = false;

    for (position, character) in value.char_indices() {
        if character == '#' && previous_is_space {
            end = position;
            break;
        }

        previous_is_space = character.is_whitespace();
    }

    value[..end].trim().to_string()
}

fn parse_single_quoted(rest: &str) -> Result<String, String> {
    let Some(close) = rest.find('\'') else {
        return Err("unterminated single quote".to_string());
    };

    check_tail(&rest[close + 1..])?;

    Ok(rest[..close].to_string())
}

fn parse_double_quoted(rest: &str) -> Result<String, String> {
    let mut result = String::new();
    let mut characters = rest.char_indices();

    while let Some((position, character)) = characters.next() {
        match character {
            '"' => {
                check_tail(&rest[position + 1..])?;

                return Ok(result);
            }
            '\\' => match characters.next() {
                Some((_, 'n')) => result.push('\n'),
                Some((_, 'r')) => result.push('\r'),
                Some((_, 't')) => result.push('\t'),
                Some((_, '"')) => result.push('"'),
                Some((_, '\\')) => result.push('\\'),
                // Any other escape is kept as written
                Some((_, other)) => {
                    result.push('\\');
                    result.push(other);
                }
                None => return Err("unterminated double quote".to_string()),
            },
            other => result.push(other),
        }
    }

    Err("unterminated double quote".to_string())
}

// After the closing quote only whitespace or a comment may follow
fn check_tail(tail: &str) -> Result<(), String> {
    let tail = tail.trim();

    if tail.is_empty() || tail.starts_with('#') {
        Ok(())
    } else {
        Err("unexpected text after the closing quote".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(map: &BTreeMap<String, String>, key: &str) -> String {
        map.get(key).cloned().unwrap_or_default()
    }

    #[test]
    fn parses_plain_pairs_blank_lines_and_comments() {
        let map = parse("# comment\n\nA=1\n  B = 2 \nC=\n").unwrap();

        assert_eq!(get(&map, "A"), "1");
        // Spaces around '=' are not part of the name, the value is trimmed
        assert_eq!(get(&map, "B"), "2");
        assert_eq!(map.get("C"), Some(&String::new()));
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn accepts_export_prefix() {
        let map = parse("export TOKEN=abc\nexported=1\n").unwrap();

        assert_eq!(get(&map, "TOKEN"), "abc");
        assert_eq!(get(&map, "exported"), "1");
    }

    #[test]
    fn handles_windows_line_endings() {
        let map = parse("A=1\r\nB=\"two\"\r\n\r\n# c\r\nC=3\r\n").unwrap();

        assert_eq!(get(&map, "A"), "1");
        assert_eq!(get(&map, "B"), "two");
        assert_eq!(get(&map, "C"), "3");
    }

    #[test]
    fn keeps_equals_signs_inside_values() {
        let map = parse("URL=postgres://u:p@h/db?a=b&c=d\nB64=abc==\n").unwrap();

        assert_eq!(get(&map, "URL"), "postgres://u:p@h/db?a=b&c=d");
        assert_eq!(get(&map, "B64"), "abc==");
    }

    #[test]
    fn handles_quotes_and_escapes() {
        let map = parse("A=\"x y # z\"\nB='a\\nb'\nC=\"l1\\nl2\"\nD=\"say \\\"hi\\\"\"\n").unwrap();

        assert_eq!(get(&map, "A"), "x y # z");
        assert_eq!(get(&map, "B"), "a\\nb");
        assert_eq!(get(&map, "C"), "l1\nl2");
        assert_eq!(get(&map, "D"), "say \"hi\"");
    }

    #[test]
    fn strips_inline_comments_only_after_whitespace() {
        let map = parse("A=value # note\nB=a#b\nC=\"q\" # note\n").unwrap();

        assert_eq!(get(&map, "A"), "value");
        assert_eq!(get(&map, "B"), "a#b");
        assert_eq!(get(&map, "C"), "q");
    }

    #[test]
    fn later_assignment_wins() {
        let map = parse("A=1\nA=2\n").unwrap();

        assert_eq!(get(&map, "A"), "2");
    }

    #[test]
    fn rejects_bad_lines_without_echoing_values() {
        let error = parse("A=1\nnot a pair\n").unwrap_err();
        assert!(error.starts_with("line 2"));
        assert!(!error.contains("not a pair"));

        assert!(parse("1A=x\n").is_err());
        assert!(parse("=x\n").is_err());
        assert!(parse("A B=x\n").is_err());

        let error = parse("A=\"secret-value\n").unwrap_err();
        assert!(error.contains("unterminated"));
        assert!(!error.contains("secret-value"));

        assert!(parse("A='x\n").is_err());
        assert!(parse("A=\"x\" y\n").is_err());
    }

    #[test]
    fn empty_text_is_an_empty_map() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("\n# only comments\n").unwrap().is_empty());
    }
}
