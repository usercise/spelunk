use super::SearchResult;
use crate::{embeddings::EmbeddingBackend, llm::LlmBackend, storage::Database};
use anyhow::Result;

/// Full RAG pipeline: embed query → vector search → assemble context → LLM.
#[allow(dead_code)]
pub struct RagPipeline<E, L> {
    pub embedder: E,
    pub llm: L,
    pub db: Database,
    pub top_k: usize,
}

#[allow(dead_code)]
impl<E: EmbeddingBackend, L: LlmBackend> RagPipeline<E, L> {
    /// Semantic vector search. Returns the top-k closest chunks.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        use crate::embeddings::vec_to_blob;

        let query_text = format!("task: code retrieval | query: {query}");
        let vecs = self.embedder.embed(&[&query_text]).await?;
        let blob = vec_to_blob(
            vecs.first()
                .ok_or_else(|| anyhow::anyhow!("no embedding"))?,
        );
        self.db.search_similar(&blob, self.top_k)
    }

    /// Ask a natural language question; streams the answer to stdout.
    pub async fn ask(&self, question: &str) -> Result<()> {
        let results = self.search(question).await?;

        let context = results
            .iter()
            .map(|r| {
                format!(
                    "// {} ({}:{})\n{}",
                    r.name.as_deref().unwrap_or("?"),
                    r.file_path,
                    r.start_line,
                    r.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let messages = vec![
            crate::llm::Message::system(
                "You are a code analysis assistant. \
                 Use the following code excerpts to answer the question.",
            ),
            crate::llm::Message::user(format!(
                "Code context:\n```\n{context}\n```\n\nQuestion: {question}"
            )),
        ];

        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let generate = self.llm.generate(&messages, 512, tx, None);

        let print_tokens = async move {
            while let Some(token) = rx.recv().await {
                print!("{token}");
            }
            println!();
        };

        tokio::try_join!(generate, async { print_tokens.await;
        Ok(()) })?;
        Ok(())
    }
}
