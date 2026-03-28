/// Extract plain text from a PDF file, page by page.
/// Returns a Vec of (page_number, text) pairs.
/// Pages with no extractable text are skipped.
#[cfg(feature = "pdf")]
pub fn extract_pdf_text(path: &std::path::Path) -> anyhow::Result<Vec<(u32, String)>> {
    use lopdf::Document;
    let doc = Document::load(path)?;
    let mut pages = Vec::new();
    // get_pages() returns a BTreeMap<u32, ObjectId> where the key is the
    // 1-based page number and ObjectId is (object_number, generation).
    // extract_text() takes a slice of 1-based page numbers (u32).
    for page_num in doc.get_pages().keys().copied() {
        if let Ok(text) = doc.extract_text(&[page_num]) {
            let trimmed = text.trim().to_string();
            if !trimmed.is_empty() {
                pages.push((page_num, trimmed));
            }
        }
    }
    Ok(pages)
}
