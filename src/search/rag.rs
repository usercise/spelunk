use anyhow::Result;
use crate::{embeddings::EmbeddingBackend, llm::LlmBackend, storage::Database};
use super::SearchResult;

/// Full RAG pipeline: embed query → vector search → assemble context → LLM.
pub struct RagPipeline<E, L> {
    pub embedder: E,
    pub llm: L,
    pub db: Database,
    pub top_k: usize,
}

impl<E: EmbeddingBackend, L: LlmBackend> RagPipeline<E, L> {
    /// Semantic vector search. Returns the top-k closest chunks.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let vecs = self.embedder.embed(&[query]).await?;
        let _query_vec = vecs.into_iter().next().unwrap();

        // Phase 4: call sqlite-vec KNN query
        // SELECT chunk_id, distance
        // FROM embeddings
        // WHERE embedding MATCH ?
        //   AND k = ?
        // ORDER BY distance
        todo!("Phase 4: sqlite-vec KNN query")
    }

    /// Ask a natural language question; streams the answer to stdout.
    pub async fn ask(&self, question: &str) -> Result<()> {
        let results = self.search(question).await?;

        let context = results
            .iter()
            .map(|r| format!("// {} ({}:{})\n{}", r.name.as_deref().unwrap_or("?"), r.file_path, r.start_line, r.content))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let prompt = build_prompt(question, &context);

        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let gen = self.llm.generate(&prompt, 512, tx);

        let print = async move {
            while let Some(token) = rx.recv().await {
                print!("{token}");
            }
            println!();
        };

        tokio::try_join!(gen, async { Ok(print.await) })?;
        Ok(())
    }
}

fn build_prompt(question: &str, context: &str) -> String {
    format!(
        "<start_of_turn>user\n\
         You are a code analysis assistant. Use the following code excerpts to answer the question.\n\n\
         Code context:\n```\n{context}\n```\n\n\
         Question: {question}<end_of_turn>\n\
         <start_of_turn>model\n"
    )
}
