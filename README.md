# codeanalysis (ca)

`codeanalysis` (`ca`) is a local-first code understanding and search tool. It uses Retrieval-Augmented Generation (RAG) to index your source code, enabling semantic search and natural language questions about your codebase—all without sending your code to a third-party cloud.

## Features

- **Semantic Code Search**: Find relevant code by meaning, not just keywords.
- **Natural Language Q&A**: Ask "How does authentication work?" or "Explain the error handling strategy" and get a cited answer.
- **AST-based Chunking**: Uses `tree-sitter` to intelligently split code into semantic units (functions, classes, structs) rather than naive line-based splitting.
- **Local-First & Private**: Designed to work with local LLMs (via LM Studio or local inference backends).
- **Graph-Aware**: Understands relationships between symbols (calls, definitions) to enrich search context.
- **Multi-Language Support**: Supports Rust, Python, JavaScript/TypeScript, Go, Java, C/C++, SQL, HTML/CSS, and more.
- **Incremental Indexing**: Uses BLAKE3 hashing to only re-index files that have changed.

## Prerequisites

- **Rust**: Install via [rustup.rs](https://rustup.rs/).
- **Inference Backend**:
  - **LM Studio (Recommended)**: Download and run [LM Studio](https://lmstudio.ai/). Load a chat model (e.g., `google/gemma-3-4b-it`) and an embedding model (e.g., `google/embeddinggemma-300m`).
  - **Metal (macOS)**: Optional built-in support for Apple Silicon GPU inference (via `candle`).

## Installation

Clone the repository and build the binary:

```bash
git clone https://github.com/your-repo/codeanalysis.git
cd codeanalysis
cargo build --release
```

The binary will be available at `./target/release/ca`. You can move it to your `PATH`.

## Quick Start

1. **Start LM Studio**: Ensure the Local Server is running (default: `http://localhost:1234`).
2. **Index a Project**:
   ```bash
   ca index /path/to/your/project
   ```
3. **Search Your Code**:
   ```bash
   ca search "where is the database connection handled?"
   ```
4. **Ask a Question**:
   ```bash
   ca ask "How are errors propagated in the indexer module?"
   ```

## Configuration

`codeanalysis` looks for a configuration file at `~/.config/codeanalysis/config.toml`. You can customize the models and API endpoints:

```toml
# ~/.config/codeanalysis/config.toml

# Base URL for LM Studio
lmstudio_base_url = "http://127.0.0.1:1234"

# Model IDs (must match the "API Identifier" in LM Studio)
embedding_model = "text-embedding-embeddinggemma-300m-qat"
llm_model = "google/gemma-3-4b-it"

# Default batch size for embeddings
batch_size = 32
```

## Commands

- `index <path>`: Parse and embed a source tree.
- `search <query>`: Find the top-K most relevant code chunks.
- `ask <question>`: Get a natural language answer based on indexed code.
- `status`: Show indexing statistics for the current project.
- `graph <symbol>`: Explore symbol relationships (calls/definitions).
- `link/unlink <path>`: Manage cross-project dependencies (search multiple projects at once).
- `languages`: List all supported languages.

## Architecture

1. **Parser**: `tree-sitter` generates an AST; we extract semantic "chunks" (functions, structs).
2. **Embedder**: Chunks are converted into high-dimensional vectors.
3. **Storage**: `SQLite` stores file metadata and code chunks; `sqlite-vec` provides high-performance vector search.
4. **RAG Pipeline**:
   - Query is embedded.
   - Vector search finds the most relevant code chunks.
   - Graph enrichment adds neighboring symbols (callers/callees).
   - Context is formatted and sent to the LLM to generate the final answer.

## Development

### Building with Features

```bash
# Build with default features (LM Studio backend)
cargo build

# Build with Metal GPU support (macOS only)
cargo build --features backend-metal
```

### Testing

```bash
cargo test
```

## Security

For detailed security guidelines and our current action plan, see [docs/security-review-action-plan.md](docs/security-review-action-plan.md).

## License

MIT
