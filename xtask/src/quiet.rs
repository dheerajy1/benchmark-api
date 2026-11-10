use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

// Which lines of the cargo output are shown besides the one-line progress
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Show {
    // Everything except the Compiling and Checking lines, so warnings and errors stay
    All,

    // Only the summary lines of a test run: Finished, Running, test result, FAILED, panicked, Doc-tests and errors
    TestSummary,
}

// Words of cargo that start a line saying which crate is being worked on
const PROGRESS_WORDS: [&str; 4] = ["Compiling", "Checking", "Documenting", "Building"];

// Words that keep a line in the test summary
const SUMMARY_WORDS: [&str; 6] = [
    "Finished",
    "Running",
    "test result",
    "FAILED",
    "panicked",
    "Doc-tests",
];

// Removes colour codes so a line can be read as plain text
fn strip_ansi(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars();

    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            // A colour code ends at its first letter
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            plain.push(character);
        }
    }

    plain
}

// Returns the progress text such as "Compiling ring" when the line is a progress line
fn progress_text(plain: &str) -> Option<String> {
    let mut words = plain.split_whitespace();
    let first = words.next()?;
    let name = words.next()?;

    PROGRESS_WORDS
        .contains(&first)
        .then(|| format!("{first} {name}"))
}

// True when a plain line is shown in the given mode
fn keep_line(plain: &str, show: Show) -> bool {
    match show {
        Show::All => true,
        Show::TestSummary => {
            !plain.contains("SIGTERM")
                && (SUMMARY_WORDS.iter().any(|word| plain.contains(word))
                    || plain.starts_with("error"))
        }
    }
}

// Runs a cargo command and shows one overwritten progress line instead of a line per crate
// Returns whether the command succeeded
pub fn run(command: &Command, show: Show) -> io::Result<bool> {
    // Both cargo streams go to one pipe in order, colours stay on
    let mut child = Command::new("sh")
        .args(["-c", "exec \"$0\" \"$@\" 2>&1"])
        .arg(command.get_program())
        .args(command.get_args())
        .env("CARGO_TERM_COLOR", "always")
        .stdout(Stdio::piped())
        .spawn()?;

    let Some(stdout) = child.stdout.take() else {
        return Err(io::Error::other("no output pipe"));
    };

    let mut out = io::stdout().lock();
    let mut progress_shown = false;

    for line in BufReader::new(stdout).split(b'\n') {
        let line = String::from_utf8_lossy(&line?).into_owned();
        let plain = strip_ansi(&line);

        if let Some(text) = progress_text(&plain) {
            // Clears the line and writes the new text without a newline
            write!(out, "\r\x1b[2K   {text}")?;
            out.flush()?;
            progress_shown = true;
        } else if keep_line(&plain, show) {
            if progress_shown {
                write!(out, "\r\x1b[2K")?;
                progress_shown = false;
            }

            writeln!(out, "{line}")?;
        }
    }

    if progress_shown {
        write!(out, "\r\x1b[2K")?;
    }

    out.flush()?;

    Ok(child.wait()?.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Colour codes are removed and the text stays
    #[test]
    fn strips_colour_codes() {
        assert_eq!(
            strip_ansi("\u{1b}[1m\u{1b}[92m   Compiling\u{1b}[0m ring v0.17.14"),
            "   Compiling ring v0.17.14"
        );
    }

    // A Compiling line gives the progress text with only the crate name
    #[test]
    fn reads_a_compiling_line() {
        assert_eq!(
            progress_text("   Compiling ring v0.17.14 (/path)"),
            Some("Compiling ring".to_string())
        );
        assert_eq!(
            progress_text("    Checking subscriber v0.1.0"),
            Some("Checking subscriber".to_string())
        );
    }

    // Other lines are not progress lines
    #[test]
    fn other_lines_are_not_progress() {
        assert_eq!(progress_text("    Finished `dev` profile in 1s"), None);
        assert_eq!(progress_text("warning: unused variable"), None);
        assert_eq!(progress_text(""), None);
    }

    // Show::All keeps every line that is not progress
    #[test]
    fn all_keeps_everything() {
        assert!(keep_line("warning: unused variable", Show::All));
        assert!(keep_line("    Finished `dev` profile", Show::All));
    }

    // The test summary keeps the summary lines and errors only
    #[test]
    fn test_summary_keeps_summary_lines() {
        assert!(keep_line("test result: ok. 3 passed", Show::TestSummary));
        assert!(keep_line(
            "     Running unittests src/lib.rs",
            Show::TestSummary
        ));
        assert!(keep_line(
            "thread 'a' panicked at src/lib.rs:1",
            Show::TestSummary
        ));
        assert!(keep_line(
            "error[E0382]: borrow of moved value",
            Show::TestSummary
        ));
        assert!(!keep_line("test auth::works ... ok", Show::TestSummary));
        assert!(!keep_line("warning: unused variable", Show::TestSummary));
    }

    // A line with SIGTERM is hidden in the test summary
    #[test]
    fn test_summary_hides_sigterm_lines() {
        assert!(!keep_line("Running stop test SIGTERM", Show::TestSummary));
    }
}
