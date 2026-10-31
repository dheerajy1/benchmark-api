use crate::features::v1::benchmark::model::BuildCommand;

// Splits a command line the way a shell would split words, without running a shell: spaces
// separate words, single and double quotes group, a backslash escapes the next character
// outside single quotes. Nothing is expanded, so no variable or glob is ever evaluated.
pub fn split(line: &str) -> Result<BuildCommand, String> {
    let words = split_words(line)?;

    let mut words = words.into_iter();

    let Some(program) = words.next() else {
        return Err("the command must not be empty".to_string());
    };

    Ok(BuildCommand {
        program,
        args: words.collect(),
    })
}

fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut characters = line.chars();

    while let Some(character) = characters.next() {
        match character {
            '\'' => {
                in_word = true;

                loop {
                    match characters.next() {
                        Some('\'') => break,
                        Some(other) => current.push(other),
                        None => return Err("unterminated single quote".to_string()),
                    }
                }
            }
            '"' => {
                in_word = true;

                loop {
                    match characters.next() {
                        Some('"') => break,
                        Some('\\') => match characters.next() {
                            Some(escaped @ ('"' | '\\')) => current.push(escaped),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err("unterminated double quote".to_string()),
                        },
                        Some(other) => current.push(other),
                        None => return Err("unterminated double quote".to_string()),
                    }
                }
            }
            '\\' => match characters.next() {
                Some(escaped) => {
                    in_word = true;
                    current.push(escaped);
                }
                None => return Err("a trailing backslash escapes nothing".to_string()),
            },
            whitespace if whitespace.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }

    if in_word {
        words.push(current);
    }

    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_program_and_arguments() {
        let command = split("bun run dev").unwrap();

        assert_eq!(command.program, "bun");
        assert_eq!(command.args, vec!["run", "dev"]);
    }

    #[test]
    fn groups_quoted_words() {
        let command = split("docker run --name 'a b' \"c d\" e\\ f").unwrap();

        assert_eq!(command.program, "docker");
        assert_eq!(command.args, vec!["run", "--name", "a b", "c d", "e f"]);
    }

    #[test]
    fn keeps_an_empty_quoted_argument() {
        let command = split("echo ''").unwrap();

        assert_eq!(command.args, vec![""]);
    }

    #[test]
    fn rejects_empty_and_unbalanced_commands() {
        assert!(split("   ").is_err());
        assert!(split("echo 'x").is_err());
        assert!(split("echo \"x").is_err());
        assert!(split("echo x\\").is_err());
    }

    #[test]
    fn does_not_expand_anything() {
        let command = split("echo $HOME *").unwrap();

        assert_eq!(command.args, vec!["$HOME", "*"]);
    }
}
