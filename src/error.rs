use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),

    #[error("parse error in {path}:{line}")]
    ParseError { path: String, line: usize },

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Error, Debug)]
pub enum EmbeddingError {
    #[error("model not loaded")]
    ModelNotLoaded,

    #[error("tokenization failed: {0}")]
    Tokenization(String),

    #[error("inference failed: {0}")]
    Inference(String),
}

#[derive(Error, Debug)]
pub enum SearchError {
    #[error("index is empty — run `ca index <path>` first")]
    EmptyIndex,

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}
