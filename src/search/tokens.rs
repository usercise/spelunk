/// Estimate token count using chars/4 heuristic (standard approximation for code/English).
pub fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_returns_one() {
        assert_eq!(estimate_tokens(""), 1);
    }

    #[test]
    fn four_chars_returns_one() {
        assert_eq!(estimate_tokens("abcd"), 1);
    }

    #[test]
    fn eight_chars_returns_two() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }
}
