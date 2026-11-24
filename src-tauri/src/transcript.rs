//! Cleanup for raw Whisper output.
//!
//! Whisper annotates non-speech audio (`[BLANK_AUDIO]`, `(music)`, `♪`) and
//! sometimes emits empty punctuation-only segments. Those must never reach the
//! user's document, so they are stripped before pasting.

/// Annotations Whisper emits for non-speech audio. Matched case-insensitively
/// inside parentheses or asterisks; bracketed annotations are always dropped.
const ARTIFACTS: &[&str] = &[
    "applause",
    "background noise",
    "beep",
    "birds",
    "blank audio",
    "blank_audio",
    "buzzing",
    "chuckles",
    "clears throat",
    "cough",
    "crickets",
    "foreign language",
    "inaudible",
    "laughter",
    "laughs",
    "laughing",
    "music",
    "no speech",
    "noise",
    "silence",
    "sigh",
    "sighs",
    "static",
    "typing",
    "wind",
];

const MAX_ANNOTATION: usize = 48;

pub fn clean(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut index = 0;

    while index < chars.len() {
        let current = chars[index];

        if current == '[' {
            if let Some(end) = find_close(&chars, index + 1, ']') {
                index = end + 1;
                continue;
            }
        }

        if current == '(' || current == '*' {
            let close = if current == '(' { ')' } else { '*' };
            if let Some(end) = find_close(&chars, index + 1, close) {
                let inner: String = chars[index + 1..end].iter().collect();
                if is_artifact(&inner) {
                    index = end + 1;
                    continue;
                }
            }
        }

        if current == '♪' || current == '♫' {
            index += 1;
            continue;
        }

        out.push(current);
        index += 1;
    }

    let cleaned = collapse_whitespace(&out);
    if cleaned.chars().any(|c| c.is_alphanumeric()) {
        cleaned
    } else {
        String::new()
    }
}

fn find_close(chars: &[char], start: usize, close: char) -> Option<usize> {
    let limit = (start + MAX_ANNOTATION).min(chars.len());
    for (offset, c) in chars[start..limit].iter().enumerate() {
        if *c == close {
            return Some(start + offset);
        }
        // Annotations never span lines; a stray opening bracket in dictated
        // text should not swallow the rest of the sentence.
        if *c == '\n' {
            return None;
        }
    }
    None
}

fn is_artifact(inner: &str) -> bool {
    let normalized = inner.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    ARTIFACTS
        .iter()
        .any(|artifact| normalized == *artifact || normalized.contains(artifact))
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = true;
    for c in text.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_bracketed_blank_audio() {
        assert_eq!(clean("[BLANK_AUDIO]"), "");
        assert_eq!(clean(" hello [SILENCE] world "), "hello world");
    }

    #[test]
    fn drops_known_parenthetical_artifacts() {
        assert_eq!(clean("Hello (music) world"), "Hello world");
        assert_eq!(clean("(applause) Great talk."), "Great talk.");
        assert_eq!(clean("*sighs* Fine."), "Fine.");
    }

    #[test]
    fn keeps_ordinary_parentheses() {
        assert_eq!(clean("Use (beta) wisely."), "Use (beta) wisely.");
        assert_eq!(clean("The ratio (about 3) is fine."), "The ratio (about 3) is fine.");
    }

    #[test]
    fn strips_music_notes() {
        assert_eq!(clean("♪ ♫ Hello ♪"), "Hello");
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(clean("  hello \n\n  world  "), "hello world");
    }

    #[test]
    fn punctuation_only_becomes_empty() {
        assert_eq!(clean(" ... "), "");
        assert_eq!(clean("!!!"), "");
    }

    #[test]
    fn keeps_normal_text_untouched() {
        assert_eq!(
            clean(" Send the report to Ana by Friday."),
            "Send the report to Ana by Friday."
        );
    }

    #[test]
    fn stray_open_paren_does_not_swallow_text() {
        let text = "(this is a long aside that is definitely not an annotation and keeps going";
        assert_eq!(clean(text), text);
    }
}
