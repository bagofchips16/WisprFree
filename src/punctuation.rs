//! Lightweight auto-punctuation post-processing.
//!
//! Whisper already does a decent job of punctuation when the model includes
//! it, but this module cleans up common issues:
//!
//! - Capitalizes the first letter of the result.
//! - Ensures a period at the end if no terminal punctuation exists.
//! - Normalizes whitespace around punctuation marks.
//! - Capitalizes after sentence-ending punctuation.

pub fn auto_punctuate(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let text = normalize_whitespace(text);
    let text = capitalize_first(&text);
    let text = capitalize_after_sentence_end(&text);
    let text = ensure_terminal_punctuation(&text);
    text
}

/// Collapse multiple spaces, remove spaces before punctuation.
fn normalize_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            if !prev_space && !result.is_empty() {
                prev_space = true;
            }
            continue;
        }

        // Remove space before punctuation that attaches to the left word.
        if is_left_attaching(ch) && prev_space {
            prev_space = false;
        }

        if prev_space {
            result.push(' ');
            prev_space = false;
        }

        result.push(ch);
    }

    result
}

/// Capitalize the first alphabetic character.
fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{}{}", upper, chars.as_str())
        }
    }
}

/// Capitalize the first letter after `.` `!` `?` followed by whitespace.
fn capitalize_after_sentence_end(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut capitalize_next = false;

    for ch in text.chars() {
        if capitalize_next && ch.is_alphabetic() {
            for u in ch.to_uppercase() {
                result.push(u);
            }
            capitalize_next = false;
        } else {
            result.push(ch);
            if ch == '.' || ch == '!' || ch == '?' {
                capitalize_next = true;
            } else if !ch.is_whitespace() {
                capitalize_next = false;
            }
        }
    }

    result
}

/// If the text doesn't end with terminal punctuation, append a period.
fn ensure_terminal_punctuation(text: &str) -> String {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }
    let last = trimmed.chars().last().unwrap();
    if is_terminal_punctuation(last) {
        trimmed.to_string()
    } else {
        format!("{}.", trimmed)
    }
}

fn is_terminal_punctuation(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '…' | '"' | '\'' | ')' | ']')
}

fn is_left_attaching(ch: char) -> bool {
    matches!(ch, '.' | ',' | '!' | '?' | ':' | ';' | ')' | ']' | '\'' | '"' | '…')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_punctuation() {
        assert_eq!(auto_punctuate("hello world"), "Hello world.");
        assert_eq!(auto_punctuate("hello world."), "Hello world.");
        assert_eq!(auto_punctuate("hello. world"), "Hello. World.");
    }

    #[test]
    fn test_whitespace_cleanup() {
        assert_eq!(auto_punctuate("hello  ,  world"), "Hello, world.");
    }

    #[test]
    fn test_empty() {
        assert_eq!(auto_punctuate(""), "");
    }
}
