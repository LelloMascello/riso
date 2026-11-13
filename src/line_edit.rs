//! Frontend-agnostic line-editing helpers for the `ssh` connection prompt:
//! display-width computation, word navigation and host-token detection.
//! Shared by `riso-gui` (VTE prompt) and `riso-tui`.

pub fn char_cols(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 || u == 0x7f {
        0
    } else if matches!(
        u,
        0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F000..=0x3FFFD
    ) {
        2
    } else {
        1
    }
}

pub fn str_cols(s: &str) -> usize {
    s.chars().map(char_cols).sum()
}

pub fn chars_cols(cs: &[char]) -> usize {
    cs.iter().copied().map(char_cols).sum()
}

pub fn rows_above_origin(vis_cols: usize, cur_col: usize, width: usize) -> usize {
    if vis_cols == 0 || width == 0 {
        return 0;
    }
    if vis_cols.is_multiple_of(width) && cur_col + 1 >= width {
        // VTE pending-wrap state at right margin
        (vis_cols / width).saturating_sub(1)
    } else {
        vis_cols / width
    }
}

pub fn prev_word_boundary(input: &[char], cursor: usize) -> usize {
    let mut i = cursor.min(input.len());
    while i > 0 && input[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !input[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

pub fn next_word_boundary(input: &[char], cursor: usize) -> usize {
    let len = input.len();
    let mut i = cursor.min(len);
    while i < len && input[i].is_whitespace() {
        i += 1;
    }
    while i < len && !input[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Extracts `(start_idx, end_idx, prefix_to_cursor)` if the cursor is on a
/// destination token (rather than a flag or an option value like `-p 2222`).
pub fn active_host_token(input: &[char], cursor: usize) -> Option<(usize, usize, String)> {
    let cursor = cursor.min(input.len());
    let mut start = cursor;
    while start > 0 && !input[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = cursor;
    while end < input.len() && !input[end].is_whitespace() {
        end += 1;
    }
    if start < end && input[start] == '-' {
        return None;
    }
    // Check if immediately preceded by a 1-arg flag such as `-p` or `-i`
    if start > 0 {
        let mut pe = start;
        while pe > 0 && input[pe - 1].is_whitespace() {
            pe -= 1;
        }
        let mut ps = pe;
        while ps > 0 && !input[ps - 1].is_whitespace() {
            ps -= 1;
        }
        if ps < pe {
            let prev: String = input[ps..pe].iter().collect();
            if matches!(
                prev.as_str(),
                "-p" | "-i"
                    | "-l"
                    | "-b"
                    | "-c"
                    | "-D"
                    | "-E"
                    | "-e"
                    | "-F"
                    | "-I"
                    | "-J"
                    | "-L"
                    | "-m"
                    | "-O"
                    | "-o"
                    | "-Q"
                    | "-R"
                    | "-S"
                    | "-W"
                    | "-w"
            ) {
                return None;
            }
        }
    }
    let prefix: String = input[start..cursor].iter().collect();
    Some((start, end, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_token_extraction_skips_flags() {
        let cs: Vec<char> = "leo@arch".chars().collect();
        assert_eq!(
            active_host_token(&cs, cs.len()),
            Some((0, 8, "leo@arch".to_string()))
        );

        let cs: Vec<char> = "-p 2222".chars().collect();
        assert_eq!(active_host_token(&cs, cs.len()), None);

        let cs: Vec<char> = "-p 2222 ho".chars().collect();
        assert_eq!(
            active_host_token(&cs, cs.len()),
            Some((8, 10, "ho".to_string()))
        );
    }

    #[test]
    fn word_boundaries_stay_inside_editable_input() {
        let cs: Vec<char> = "-p 2222 leo@host".chars().collect();
        assert_eq!(prev_word_boundary(&cs, 0), 0);
        assert_eq!(prev_word_boundary(&cs, 16), 8);
        assert_eq!(prev_word_boundary(&cs, 8), 3);
        assert_eq!(prev_word_boundary(&cs, 3), 0);
    }
}
