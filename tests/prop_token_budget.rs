use proptest::prelude::*;
use spelunk::search::tokens::estimate_tokens;

proptest! {
    // estimate_tokens always returns >= 1
    #[test]
    fn estimate_tokens_always_positive(s in ".*") {
        prop_assert!(estimate_tokens(&s) >= 1);
    }

    // Longer strings always produce >= token count of shorter prefix
    #[test]
    fn estimate_tokens_monotone(
        base in "[a-z]{4,100}",
        extra in "[a-z]{1,50}",
    ) {
        let combined = format!("{}{}", base, extra);
        prop_assert!(estimate_tokens(&combined) >= estimate_tokens(&base));
    }

    // Token count scales with length (roughly chars/4)
    #[test]
    fn estimate_tokens_chars_div_4(s in "[a-z]{1,200}") {
        let expected = (s.chars().count() / 4).max(1);
        prop_assert_eq!(estimate_tokens(&s), expected);
    }
}
