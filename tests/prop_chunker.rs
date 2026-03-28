use proptest::prelude::*;
use spelunk::indexer::chunker::sliding_window;

proptest! {
    // Every chunk's content must be a substring of the original source
    #[test]
    fn chunks_are_substrings_of_source(
        source in "([a-z ]+\n){1,50}",
        window in 5usize..=50,
        overlap in 0usize..=10,
    ) {
        let overlap = overlap.min(window - 1);
        let chunks = sliding_window(&source, "test.txt", "text", window, overlap);
        for chunk in &chunks {
            prop_assert!(
                source.contains(chunk.content.trim()),
                "chunk content not found in source"
            );
        }
    }

    // No chunk should exceed window size (in lines)
    #[test]
    fn chunks_respect_window_size(
        source in "([a-z ]+\n){1,100}",
        window in 5usize..=30,
        overlap in 0usize..=5,
    ) {
        let overlap = overlap.min(window - 1);
        let chunks = sliding_window(&source, "test.txt", "text", window, overlap);
        for chunk in &chunks {
            let line_count = chunk.content.lines().count();
            prop_assert!(
                line_count <= window,
                "chunk has {} lines, window is {}",
                line_count,
                window
            );
        }
    }

    // Empty source always yields no chunks
    #[test]
    fn empty_source_yields_no_chunks(
        window in 1usize..=50,
        overlap in 0usize..=10,
    ) {
        let overlap = overlap.min(window - 1);
        let chunks = sliding_window("", "test.txt", "text", window, overlap);
        prop_assert!(chunks.is_empty());
    }
}
