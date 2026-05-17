//! Free-form Q&A over an ingested workspace.
//!
//! This module shares the embedder + repository + provider trio with
//! [`crate::pipeline`] but skips the medical-verdict scaffolding (JSON
//! schema, deidentifier, validation, case persistence). The caller asks
//! a question in natural language, the pipeline retrieves the top-k chunks
//! by cosine similarity, hands them to the configured LLM as inline
//! `[1]`, `[2]` citations, and returns the answer plus the cited sources
//! for the UI to render.

use std::sync::Arc;

use conclave_core::{Error, Result};
use conclave_providers::{CompletionRequest, LlmProvider, Message, WebCitation};
use conclave_rag::{DocumentRepository, Embedder};

pub struct QaPipeline {
    embedder: Arc<dyn Embedder>,
    repository: Arc<DocumentRepository>,
    provider: Arc<dyn LlmProvider>,
}

impl std::fmt::Debug for QaPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QaPipeline")
            .field("embedder", &self.embedder.id())
            .field("provider", &self.provider.id())
            .finish_non_exhaustive()
    }
}

/// A chunk that the LLM was given as evidence, exposed to the UI so it
/// can render the inline `[N]` citations as source cards.
#[derive(Debug, Clone)]
pub struct QaSource {
    /// 1-based index matching the `[N]` references in the answer text.
    pub index: usize,
    pub document_id: String,
    pub document_title: String,
    pub chunk_id: String,
    /// Raw chunk text — frontend truncates as needed for display.
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct QaResponse {
    pub answer: String,
    pub sources: Vec<QaSource>,
    /// URLs the LLM consulted via live web search (only populated when the
    /// caller requested `allow_web_search` AND the provider supports it).
    pub web_sources: Vec<WebCitation>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub model: String,
}

impl QaPipeline {
    pub fn new(
        embedder: Arc<dyn Embedder>,
        repository: Arc<DocumentRepository>,
        provider: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            embedder,
            repository,
            provider,
        }
    }

    pub async fn ask(
        &self,
        question: &str,
        top_k: usize,
        language: &str,
        allow_general_knowledge: bool,
    ) -> Result<QaResponse> {
        // 1) Embed the question (sync ONNX call → spawn_blocking).
        let q = question.to_string();
        let embedder = Arc::clone(&self.embedder);
        let vectors = tokio::task::spawn_blocking(move || embedder.embed(&[q]))
            .await
            .map_err(|e| Error::Rag(format!("embed task join: {e}")))??;
        let qvec = vectors
            .into_iter()
            .next()
            .ok_or_else(|| Error::Rag("empty embedding".into()))?;

        // 2) Retrieve top-k chunks by cosine similarity.
        let hits = self.repository.search(&qvec, top_k).await?;

        // 3) Resolve document titles for citations. Repository::show is
        // a SQLite lookup; we issue one per hit. With top_k ≤ 8 this is
        // fine; if we ever scale top_k we can batch.
        let mut sources = Vec::with_capacity(hits.len());
        for (i, h) in hits.iter().enumerate() {
            let title = self
                .repository
                .show(&h.document_id)?
                .map(|d| d.record.title)
                .unwrap_or_else(|| h.document_id.clone());
            sources.push(QaSource {
                index: i + 1,
                document_id: h.document_id.clone(),
                document_title: title,
                chunk_id: h.chunk_id.clone(),
                snippet: h.text.clone(),
            });
        }

        // 4) Build prompt as system instructions + user message. Some
        // providers (OpenAI OAuth → Codex Responses API) require a separate
        // `instructions` field, and Anthropic / Ollama also handle system
        // messages distinctly. Splitting here keeps the contract clean.
        let (system_text, user_text) =
            build_prompt(question, &sources, language, allow_general_knowledge);

        // 5) Call provider. No JSON schema — we want free prose with
        // citations. Low temperature so the model sticks to the context.
        let resp = self
            .provider
            .complete(CompletionRequest {
                model: String::new(),
                messages: vec![Message::system(system_text), Message::user(user_text)],
                max_output_tokens: Some(1500),
                temperature: Some(0.2),
                json_schema: None,
                allow_web_search: false,
            })
            .await
            .map_err(|e| Error::Rag(e.to_string()))?;

        Ok(QaResponse {
            answer: resp.text,
            sources,
            // Providers with native web tools (none today) would populate
            // these. Codex doesn't, so this stays empty in practice.
            web_sources: resp.web_citations,
            input_tokens: resp.usage.input_tokens,
            output_tokens: resp.usage.output_tokens,
            model: resp.model,
        })
    }
}

/// Returns `(system_instructions, user_message)`. The split lets each
/// provider place the instructions where its API expects (top-level
/// `instructions` for Codex, `system` for Anthropic, etc.).
fn build_prompt(
    question: &str,
    sources: &[QaSource],
    language: &str,
    allow_general_knowledge: bool,
) -> (String, String) {
    let system = if allow_general_knowledge {
        format!(
            "You are a research assistant for clinical documents. ALWAYS prefer the \
             document snippets the user supplies — cite them inline as [1], [2], \
             etc. If, and only if, the snippets do not cover the question, you may \
             draw on your general medical training knowledge to fill the gap. When \
             you do, you MUST open your answer with a SHORT one-sentence disclosure \
             as its own paragraph, beginning with the warning sign emoji ⚠️ followed \
             by a single space — something like \"⚠️ Esta respuesta usa conocimiento \
             general del modelo porque los documentos no la cubren — verifica con \
             fuentes primarias.\" (or the English equivalent if replying in English). \
             Keep this disclosure to ONE sentence: do not add caveats about training \
             cutoff dates, internet access, or model limitations — the UI handles \
             that separately. Never invent facts or fabricate URLs. If your \
             primary basis is documents only, omit the ⚠️ line entirely. Reply in \
             {language}.",
        )
    } else {
        format!(
            "You are a research assistant for clinical documents. Answer the user's \
             question using ONLY the snippets they provide. Cite snippets inline as \
             [1], [2], etc., matching the bracketed indices in the context. If the \
             snippets do not contain enough information to answer the question, say \
             plainly that no relevant information is available — do not invent and \
             do not draw on outside knowledge. Reply in {language}.",
        )
    };
    let mut user = String::new();
    if sources.is_empty() {
        user.push_str("CONTEXT (documents)\n===================\n(no documents available)\n\n");
    } else {
        user.push_str("CONTEXT (documents)\n===================\n");
        for s in sources {
            user.push_str(&format!(
                "[{}] from \"{}\":\n{}\n\n",
                s.index, s.document_title, s.snippet
            ));
        }
    }
    user.push_str("QUESTION\n========\n");
    user.push_str(question);
    user.push('\n');
    (system, user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use conclave_providers::MockProvider;
    use conclave_rag::{
        chunk_text, ChunkParams, DocumentRepository, IngestionPipeline, MockEmbedder,
        RepositoryLayout,
    };
    use tempfile::tempdir;

    async fn fixture_pipeline(
        tmp: &tempfile::TempDir,
    ) -> (Arc<DocumentRepository>, Arc<dyn Embedder>) {
        let layout = RepositoryLayout::new(tmp.path().join("workspace"));
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder::new());
        let repo = Arc::new(
            DocumentRepository::open(layout, embedder.dim())
                .await
                .unwrap(),
        );
        // Seed two documents so search returns real hits.
        let pipeline = IngestionPipeline::new(
            Arc::clone(&embedder),
            Arc::clone(&repo),
            ChunkParams::DEFAULT,
        )
        .unwrap();
        let file_a = tmp.path().join("a.txt");
        std::fs::write(
            &file_a,
            "El manejo de la insuficiencia cardiaca incluye diuréticos.",
        )
        .unwrap();
        let file_b = tmp.path().join("b.txt");
        std::fs::write(
            &file_b,
            "La hipertensión se trata con cambios en el estilo de vida.",
        )
        .unwrap();
        pipeline.ingest_path(&file_a, |_| {}).await.unwrap();
        pipeline.ingest_path(&file_b, |_| {}).await.unwrap();
        // Sanity: chunking is reachable from this scope without warnings.
        let _ = chunk_text("noop", "id", ChunkParams::DEFAULT).unwrap();
        (repo, embedder)
    }

    #[tokio::test]
    async fn ask_returns_sources_and_calls_provider_with_citations() {
        let tmp = tempdir().unwrap();
        let (repo, embedder) = fixture_pipeline(&tmp).await;
        let provider = Arc::new(MockProvider::with_response(
            "El manejo se basa en diuréticos según [1].",
        ));
        let qa = QaPipeline::new(embedder, repo, provider.clone() as Arc<dyn LlmProvider>);
        let resp = qa
            .ask("¿Cómo se trata la insuficiencia cardiaca?", 4, "es", false)
            .await
            .unwrap();

        // Provider should have been called with two messages: system + user.
        // The system message carries the instructions, the user message carries
        // the [N]-indexed context and the question.
        let captured = provider.captured_requests();
        assert_eq!(captured.len(), 1);
        let messages = &captured[0].messages;
        assert_eq!(messages.len(), 2, "expected system + user messages");
        let system = &messages[0].content;
        let user = &messages[1].content;
        assert!(
            system.contains("Reply in es"),
            "system missing language: {system}"
        );
        assert!(
            user.contains("[1]"),
            "user missing [1] citation marker: {user}"
        );
        assert!(
            user.contains("¿Cómo se trata la insuficiencia cardiaca?"),
            "user missing question",
        );

        // Response should expose at least one source.
        assert!(!resp.sources.is_empty(), "expected at least one source");
        assert_eq!(resp.sources[0].index, 1);
        assert!(!resp.sources[0].snippet.is_empty());
        assert_eq!(resp.answer, "El manejo se basa en diuréticos según [1].");
    }

    #[tokio::test]
    async fn ask_with_empty_workspace_still_calls_llm() {
        let tmp = tempdir().unwrap();
        let layout = RepositoryLayout::new(tmp.path().join("workspace"));
        let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder::new());
        let repo = Arc::new(
            DocumentRepository::open(layout, embedder.dim())
                .await
                .unwrap(),
        );
        let provider = Arc::new(MockProvider::with_response(
            "No relevant information is available.",
        ));
        let qa = QaPipeline::new(embedder, repo, provider.clone() as Arc<dyn LlmProvider>);
        let resp = qa.ask("anything", 4, "en", false).await.unwrap();
        assert!(resp.sources.is_empty());
        let captured = provider.captured_requests();
        assert_eq!(captured[0].messages.len(), 2);
        // System message is the instructions; user carries the empty context.
        assert!(captured[0].messages[1]
            .content
            .contains("(no documents available)"));
    }
}
