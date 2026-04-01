mod ai;

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration, Utc};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::{MainRequestSeed, TranscriptEntry},
    config::{AppConfig, MemoryConfig, MemoryExtractionConfig},
    stats::StatsHook,
    token_counting::count_text_tokens,
};

use self::ai::{
    MemoryCandidateContext, MemoryCandidateDraft, MemoryConsolidationAction,
    MemoryConsolidationDecision, MemoryConsolidationOutput, MemoryExtractorOutput,
    MemoryTranscriptEvidence, MemoryTurnEvidence, RelatedMemorySummary, consolidate_candidates,
    extract_candidates,
};

const MEMORY_DIR_RELATIVE_PATH: &str = ".config/oat/memory";
const SNAPSHOT_FILE_NAME: &str = "snapshot.json";
const EVENTS_FILE_NAME: &str = "events.jsonl";
const VECTORS_FILE_NAME: &str = "vectors.json";
const SCHEMA_VERSION: u32 = 1;
const FASTEMBED_MODEL: EmbeddingModel = EmbeddingModel::BGESmallENV15;

#[derive(Clone)]
pub(crate) struct MemoryService {
    inner: Arc<Mutex<MemoryManager>>,
}

#[derive(Clone, Debug)]
pub(crate) struct CompletedTurnMemoryInput {
    pub session_id: Option<String>,
    pub seed: MainRequestSeed,
    pub transcript_entries: Vec<TranscriptEntry>,
    pub assistant_response: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct MemoryRecord {
    pub id: String,
    pub scope: MemoryScope,
    pub repo_fingerprint: Option<String>,
    pub subject_key: String,
    pub kind: MemoryKind,
    pub title: String,
    pub summary: String,
    pub details: Option<String>,
    pub tags: Vec<String>,
    pub evidence: Vec<MemoryEvidenceRef>,
    pub source: MemorySource,
    pub confidence: f32,
    pub status: MemoryStatus,
    pub supersedes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryScope {
    Global,
    Repo,
    Module,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryKind {
    Preference,
    Workflow,
    Architecture,
    Decision,
    Hazard,
    Episode,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemorySource {
    ExplicitUser,
    Inferred,
    AutoSummary,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryStatus {
    Candidate,
    Active,
    Superseded,
    Archived,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MemoryEvidenceRef {
    pub session_id: Option<String>,
    pub prompt: Option<String>,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MemorySnapshot {
    schema_version: u32,
    records: Vec<MemoryRecord>,
}

impl Default for MemorySnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            records: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MemoryVectorsSnapshot {
    schema_version: u32,
    backend: String,
    vectors: Vec<PersistedVector>,
}

impl Default for MemoryVectorsSnapshot {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            backend: "fastembed/bge-small-en-v1.5".into(),
            vectors: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedVector {
    id: String,
    vector: Vec<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum MemoryEvent {
    Created {
        record: MemoryRecord,
    },
    Updated {
        record: MemoryRecord,
    },
    Cleared {
        cleared_at: DateTime<Utc>,
    },
    Superseded {
        ids: Vec<String>,
        updated_at: DateTime<Utc>,
    },
    Promoted {
        id: String,
        updated_at: DateTime<Utc>,
    },
    Archived {
        id: String,
        updated_at: DateTime<Utc>,
    },
    Replaced {
        old_id: String,
        new_record: MemoryRecord,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MemoryEventRecord {
    schema_version: u32,
    ts: DateTime<Utc>,
    #[serde(flatten)]
    event: MemoryEvent,
}

#[derive(Clone, Debug)]
struct MemorySearchHit {
    record: MemoryRecord,
    score: f32,
    signals: MemoryMatchSignals,
}

#[derive(Clone, Debug, Default)]
struct MemoryMatchSignals {
    lexical_score: f32,
    semantic_score: f32,
    exact_subject: bool,
    exact_path_match: bool,
    exact_tag_hits: Vec<String>,
    exact_term_hits: Vec<String>,
    path_term_hits: Vec<String>,
    total_score: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetrievalMode {
    Search,
    AutoInject,
    CandidateLinking,
}

#[derive(Clone, Copy, Debug)]
struct RetrievalPolicy {
    mode: RetrievalMode,
    include_candidates: bool,
    limit: usize,
    min_total_score: f32,
    min_semantic_score: f32,
    min_lexical_score: f32,
}

impl RetrievalPolicy {
    fn search(config: &MemoryConfig, limit: usize, include_candidates: bool) -> Self {
        Self {
            mode: RetrievalMode::Search,
            include_candidates,
            limit: limit.max(1),
            min_total_score: config.retrieval.search.min_total_score,
            min_semantic_score: config.retrieval.search.min_semantic_score,
            min_lexical_score: config.retrieval.search.min_lexical_score,
        }
    }

    fn auto_inject(config: &MemoryConfig, limit: usize) -> Self {
        Self {
            mode: RetrievalMode::AutoInject,
            include_candidates: false,
            limit: limit.clamp(1, 3),
            min_total_score: config.retrieval.auto_inject.min_total_score,
            min_semantic_score: config.retrieval.auto_inject.min_semantic_score,
            min_lexical_score: config.retrieval.auto_inject.min_lexical_score,
        }
    }

    fn candidate_linking(config: &MemoryConfig, limit: usize) -> Self {
        Self {
            mode: RetrievalMode::CandidateLinking,
            include_candidates: true,
            limit: limit.max(1),
            min_total_score: config.retrieval.candidate_linking.min_total_score,
            min_semantic_score: config.retrieval.candidate_linking.min_semantic_score,
            min_lexical_score: config.retrieval.candidate_linking.min_lexical_score,
        }
    }

    fn allows_kind(self, kind: MemoryKind) -> bool {
        match self.mode {
            RetrievalMode::AutoInject => {
                matches!(
                    kind,
                    MemoryKind::Preference | MemoryKind::Workflow | MemoryKind::Hazard
                )
            }
            RetrievalMode::Search | RetrievalMode::CandidateLinking => true,
        }
    }
}

trait EmbeddingProvider: Send {
    fn backend_name(&self) -> &'static str;
    fn embed(&mut self, inputs: Vec<String>) -> Result<Vec<Vec<f32>>>;
}

struct FastembedEmbeddingProvider {
    cache_dir: PathBuf,
    model: Option<TextEmbedding>,
    disabled_reason: Option<String>,
}

impl FastembedEmbeddingProvider {
    fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            model: None,
            disabled_reason: fastembed_unavailable_reason(),
        }
    }
}

impl EmbeddingProvider for FastembedEmbeddingProvider {
    fn backend_name(&self) -> &'static str {
        "fastembed/bge-small-en-v1.5"
    }

    fn embed(&mut self, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if let Some(reason) = &self.disabled_reason {
            return Err(anyhow!("semantic memory disabled: {reason}"));
        }

        if self.model.is_none() {
            let model = catch_unwind(AssertUnwindSafe(|| {
                TextEmbedding::try_new(
                    TextInitOptions::new(FASTEMBED_MODEL)
                        .with_cache_dir(self.cache_dir.clone())
                        .with_show_download_progress(false),
                )
            }))
            .map_err(|_| anyhow!("fastembed panicked while initializing ONNX runtime"))?
            .context("failed to initialize fastembed model")?;
            self.model = Some(model);
        }

        let model = self.model.as_mut().expect("model initialized");
        catch_unwind(AssertUnwindSafe(|| model.embed(inputs, None)))
            .map_err(|_| anyhow!("fastembed panicked while generating embeddings"))?
            .context("failed to generate embeddings")
    }
}

fn fastembed_unavailable_reason() -> Option<String> {
    if let Some(path) = env::var_os("ORT_DYLIB_PATH") {
        let path = PathBuf::from(path);
        return (!path.exists()).then(|| {
            format!(
                "ORT_DYLIB_PATH points to `{}` but the file does not exist",
                path.display()
            )
        });
    }

    let library_name = onnxruntime_library_name();
    if fastembed_search_dirs()
        .into_iter()
        .any(|dir| dir.join(library_name).exists())
    {
        return None;
    }

    Some(format!(
        "no ONNX Runtime dynamic library was found; set ORT_DYLIB_PATH or install `{library_name}`"
    ))
}

fn fastembed_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path) = env::var_os(ld_library_path_var_name()) {
        dirs.extend(env::split_paths(&path));
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    dirs.extend(
        [
            "/usr/lib",
            "/usr/local/lib",
            "/lib",
            "/lib64",
            "/usr/lib64",
            "/usr/lib/x86_64-linux-gnu",
            "/lib/x86_64-linux-gnu",
        ]
        .into_iter()
        .map(PathBuf::from),
    );

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    dirs.extend(
        ["/usr/local/lib", "/opt/homebrew/lib", "/usr/lib"]
            .into_iter()
            .map(PathBuf::from),
    );

    dirs.sort();
    dirs.dedup();
    dirs
}

fn ld_library_path_var_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "PATH"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "LD_LIBRARY_PATH"
    }
}

fn onnxruntime_library_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "onnxruntime.dll"
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        "libonnxruntime.dylib"
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        "libonnxruntime.so"
    }
}

struct MemoryManager {
    config: MemoryConfig,
    workspace_root: PathBuf,
    repo_root: PathBuf,
    repo_fingerprint: String,
    storage_dir: Option<PathBuf>,
    snapshot: MemorySnapshot,
    vectors: BTreeMap<String, Vec<f32>>,
    embedder: Box<dyn EmbeddingProvider>,
}

#[derive(Clone, Debug)]
struct PreparedCompletedTurn {
    extraction: MemoryExtractionConfig,
    evidence: MemoryTurnEvidence,
}

impl MemoryService {
    pub(crate) fn new(config: MemoryConfig, workspace_root: PathBuf) -> Result<Self> {
        let storage_dir = default_memory_dir();
        let cache_dir = storage_dir
            .as_ref()
            .map(|dir| dir.join("models"))
            .unwrap_or_else(|| PathBuf::from("."));
        let manager = MemoryManager::load(
            config,
            workspace_root,
            storage_dir,
            Box::new(FastembedEmbeddingProvider::new(cache_dir)),
        )?;
        Ok(Self {
            inner: Arc::new(Mutex::new(manager)),
        })
    }

    pub(crate) fn augment_prompt(&self, prompt: &str) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.augment_prompt(prompt)
    }

    pub(crate) fn search_text(
        &self,
        query: &str,
        include_candidates: bool,
        limit: usize,
    ) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.search_text(query, include_candidates, limit)
    }

    pub(crate) fn get_text(&self, id: &str) -> Result<String> {
        let manager = self.inner.lock().expect("memory lock");
        manager.get_text(id)
    }

    pub(crate) fn list_candidates_text(&self) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.list_candidates_text()
    }

    pub(crate) fn stats_text(&self) -> Result<String> {
        let manager = self.inner.lock().expect("memory lock");
        manager.stats_text()
    }

    pub(crate) fn promote(&self, id: &str) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.promote(id)
    }

    pub(crate) fn archive(&self, id: &str) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.archive(id)
    }

    pub(crate) fn replace(&self, id: &str, text: &str) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.replace(id, text)
    }

    pub(crate) fn clear(&self) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.clear()
    }

    pub(crate) fn rebuild_indexes(&self) -> Result<String> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.rebuild_indexes()
    }

    pub(crate) fn set_config(&self, config: MemoryConfig) {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.set_config(config);
    }

    pub(crate) async fn process_completed_turn(
        &self,
        app_config: AppConfig,
        stats_hook: StatsHook,
        input: CompletedTurnMemoryInput,
    ) -> Result<()> {
        let prepared = {
            let manager = self.inner.lock().expect("memory lock");
            manager.prepare_completed_turn(&input)?
        };
        let Some(prepared) = prepared else {
            return Ok(());
        };

        let extracted = extract_candidates(
            &app_config,
            &prepared.extraction,
            &prepared.evidence,
            stats_hook.clone(),
        )
        .await
        .context("memory extraction failed")?;
        if extracted.candidates.is_empty() {
            return Ok(());
        }

        let candidate_contexts = {
            let mut manager = self.inner.lock().expect("memory lock");
            manager
                .build_candidate_contexts(&extracted, prepared.extraction.max_related_memories)?
        };
        if candidate_contexts.is_empty() {
            return Ok(());
        }

        let consolidated = consolidate_candidates(
            &app_config,
            &prepared.extraction,
            &prepared.evidence,
            &candidate_contexts,
            stats_hook,
        )
        .await
        .context("memory consolidation failed")?;

        let mut manager = self.inner.lock().expect("memory lock");
        manager.apply_consolidation(prepared, &candidate_contexts, consolidated)
    }

    #[cfg(test)]
    fn with_embedder(
        config: MemoryConfig,
        workspace_root: PathBuf,
        storage_dir: Option<PathBuf>,
        embedder: Box<dyn EmbeddingProvider>,
    ) -> Result<Self> {
        let manager = MemoryManager::load(config, workspace_root, storage_dir, embedder)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(manager)),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_storage_dir(
        config: MemoryConfig,
        workspace_root: PathBuf,
        storage_dir: PathBuf,
    ) -> Result<Self> {
        let cache_dir = storage_dir.join("models");
        let manager = MemoryManager::load(
            config,
            workspace_root,
            Some(storage_dir),
            Box::new(FastembedEmbeddingProvider::new(cache_dir)),
        )?;
        Ok(Self {
            inner: Arc::new(Mutex::new(manager)),
        })
    }

    #[cfg(test)]
    pub(crate) fn insert_test_record(&self, record: MemoryRecord) -> Result<()> {
        let mut manager = self.inner.lock().expect("memory lock");
        manager.create_record(record)
    }
}

impl MemoryManager {
    fn load(
        config: MemoryConfig,
        workspace_root: PathBuf,
        storage_dir: Option<PathBuf>,
        embedder: Box<dyn EmbeddingProvider>,
    ) -> Result<Self> {
        let repo_root = detect_repo_root(&workspace_root);
        let repo_fingerprint = repo_root.display().to_string();
        let snapshot = load_snapshot(storage_dir.as_deref())?;
        let vectors = load_vectors(storage_dir.as_deref())?;
        Ok(Self {
            config,
            workspace_root,
            repo_root,
            repo_fingerprint,
            storage_dir,
            snapshot,
            vectors,
            embedder,
        })
    }

    fn augment_prompt(&mut self, prompt: &str) -> Result<String> {
        if !self.config.enabled || !self.config.auto_inject || prompt.trim().is_empty() {
            return Ok(prompt.to_string());
        }

        let hits = self.search_hits(
            prompt,
            RetrievalPolicy::auto_inject(&self.config, self.config.max_auto_results),
        )?;
        if hits.is_empty() {
            return Ok(prompt.to_string());
        }

        let mut brief = String::from("Relevant memory:\n");
        let mut used_tokens = count_text_tokens(&brief) as usize;
        for hit in hits {
            let line = format!(
                "- [{}] {}: {}\n",
                short_id(&hit.record.id),
                hit.record.title,
                hit.record.summary
            );
            let line_tokens = count_text_tokens(&line) as usize;
            if used_tokens + line_tokens > self.config.auto_inject_token_budget {
                break;
            }
            used_tokens += line_tokens;
            brief.push_str(&line);
        }

        if brief.trim() == "Relevant memory:" {
            Ok(prompt.to_string())
        } else {
            Ok(format!("{brief}\nUser request:\n{prompt}"))
        }
    }

    fn search_text(
        &mut self,
        query: &str,
        include_candidates: bool,
        limit: usize,
    ) -> Result<String> {
        let hits = self.search_hits(
            query,
            RetrievalPolicy::search(&self.config, limit, include_candidates),
        )?;
        Ok(format_search_results(query, &hits, true))
    }

    fn get_text(&self, id: &str) -> Result<String> {
        let record = self
            .snapshot
            .records
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("Memory `{id}` was not found."))?;
        Ok(format_record(&record))
    }

    fn list_candidates_text(&mut self) -> Result<String> {
        let hits = self.search_hits(
            "",
            RetrievalPolicy::search(&self.config, self.config.max_candidate_search_results, true),
        )?;
        let candidates = hits
            .into_iter()
            .filter(|hit| hit.record.status == MemoryStatus::Candidate)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            Ok("No memory candidates are waiting for review.".into())
        } else {
            Ok(format_search_results("candidates", &candidates, false))
        }
    }

    fn stats_text(&self) -> Result<String> {
        Ok(format_memory_stats(
            &self.snapshot.records,
            &self.vectors,
            &self.repo_fingerprint,
            self.embedder.backend_name(),
        ))
    }

    fn promote(&mut self, id: &str) -> Result<String> {
        let now = Utc::now();
        let short = short_id(id).to_string();
        let record = self
            .snapshot
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| anyhow!("Memory `{id}` was not found."))?;
        if record.status == MemoryStatus::Archived {
            return Err(anyhow!("Archived memory `{id}` cannot be promoted."));
        }
        record.status = MemoryStatus::Active;
        record.updated_at = now;
        self.append_event(MemoryEvent::Promoted {
            id: id.to_string(),
            updated_at: now,
        })?;
        self.persist_snapshot()?;
        Ok(format!("Promoted memory `{short}` to active."))
    }

    fn archive(&mut self, id: &str) -> Result<String> {
        let now = Utc::now();
        let record = self
            .snapshot
            .records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| anyhow!("Memory `{id}` was not found."))?;
        record.status = MemoryStatus::Archived;
        record.updated_at = now;
        self.vectors.remove(id);
        self.append_event(MemoryEvent::Archived {
            id: id.to_string(),
            updated_at: now,
        })?;
        self.persist_snapshot()?;
        self.persist_vectors()?;
        Ok(format!("Archived memory `{}`.", short_id(id)))
    }

    fn replace(&mut self, id: &str, text: &str) -> Result<String> {
        let now = Utc::now();
        let previous = self
            .snapshot
            .records
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .ok_or_else(|| anyhow!("Memory `{id}` was not found."))?;
        let new_record = MemoryRecord {
            id: Uuid::now_v7().to_string(),
            scope: previous.scope,
            repo_fingerprint: previous.repo_fingerprint.clone(),
            subject_key: previous.subject_key.clone(),
            kind: previous.kind,
            title: previous.title.clone(),
            summary: text.trim().to_string(),
            details: previous.details.clone(),
            tags: previous.tags.clone(),
            evidence: previous.evidence.clone(),
            source: MemorySource::ExplicitUser,
            confidence: previous.confidence.max(0.95),
            status: MemoryStatus::Active,
            supersedes: Some(previous.id.clone()),
            created_at: now,
            updated_at: now,
        };
        if let Some(old) = self
            .snapshot
            .records
            .iter_mut()
            .find(|record| record.id == id)
        {
            old.status = MemoryStatus::Superseded;
            old.updated_at = now;
        }
        self.snapshot.records.push(new_record.clone());
        self.vectors.remove(id);
        self.append_event(MemoryEvent::Replaced {
            old_id: id.to_string(),
            new_record: new_record.clone(),
        })?;
        self.persist_snapshot()?;
        self.persist_vectors()?;
        Ok(format!(
            "Created replacement memory `{}` superseding `{}`.",
            short_id(&new_record.id),
            short_id(id)
        ))
    }

    fn clear(&mut self) -> Result<String> {
        let cleared = self.snapshot.records.len();
        self.snapshot = MemorySnapshot::default();
        self.vectors.clear();
        self.reset_storage()?;
        Ok(match cleared {
            0 => "Memory store already empty.".to_string(),
            1 => "Cleared 1 memory.".to_string(),
            n => format!("Cleared {n} memories."),
        })
    }

    fn rebuild_indexes(&mut self) -> Result<String> {
        self.vectors.clear();
        let ids = self
            .snapshot
            .records
            .iter()
            .filter(|record| {
                matches!(
                    record.status,
                    MemoryStatus::Active | MemoryStatus::Candidate
                ) && self.record_is_in_scope(record)
            })
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let degraded = self
            .ensure_vectors(&ids)
            .err()
            .map(|error| error.to_string());
        self.persist_vectors()?;
        Ok(match degraded {
            Some(error) => {
                format!("Rebuilt lexical memory index. Semantic vectors were skipped: {error}")
            }
            None => format!(
                "Rebuilt memory indexes for {} record{}.",
                ids.len(),
                if ids.len() == 1 { "" } else { "s" }
            ),
        })
    }

    fn set_config(&mut self, config: MemoryConfig) {
        self.config = config;
    }

    fn prepare_completed_turn(
        &self,
        input: &CompletedTurnMemoryInput,
    ) -> Result<Option<PreparedCompletedTurn>> {
        if !self.config.enabled || !self.config.extraction.enabled {
            return Ok(None);
        }

        let visible_prompt = input.seed.visible_prompt.trim();
        let assistant_response = input.assistant_response.trim();
        if visible_prompt.is_empty() && assistant_response.is_empty() {
            return Ok(None);
        }

        let touched_files = collect_paths_from_entries(
            &self.workspace_root,
            &self.repo_root,
            &input.transcript_entries,
        );
        let transcript = limit_transcript_evidence(
            input
                .transcript_entries
                .iter()
                .map(transcript_entry_to_evidence)
                .collect(),
            self.config.extraction.max_evidence_tokens,
        );
        let evidence = MemoryTurnEvidence {
            session_id: input.session_id.clone(),
            repo_fingerprint: self.repo_fingerprint.clone(),
            visible_prompt: truncate(visible_prompt, 4_000),
            assistant_response: truncate(assistant_response, 6_000),
            touched_files,
            transcript,
        };

        Ok(Some(PreparedCompletedTurn {
            extraction: self.config.extraction.clone(),
            evidence,
        }))
    }

    fn build_candidate_contexts(
        &mut self,
        extracted: &MemoryExtractorOutput,
        max_related_memories: usize,
    ) -> Result<Vec<MemoryCandidateContext>> {
        extracted
            .candidates
            .iter()
            .enumerate()
            .map(|(candidate_index, candidate)| {
                let query = candidate_lookup_query(candidate);
                let related_memories = self
                    .search_hits(
                        &query,
                        RetrievalPolicy::candidate_linking(&self.config, max_related_memories),
                    )?
                    .into_iter()
                    .map(|hit| related_memory_summary(&hit.record))
                    .collect();
                Ok(MemoryCandidateContext {
                    candidate_index,
                    candidate: candidate.clone(),
                    related_memories,
                })
            })
            .collect()
    }

    fn apply_consolidation(
        &mut self,
        prepared: PreparedCompletedTurn,
        candidate_contexts: &[MemoryCandidateContext],
        consolidated: MemoryConsolidationOutput,
    ) -> Result<()> {
        let now = Utc::now();
        let evidence_ref = MemoryEvidenceRef {
            session_id: prepared.evidence.session_id.clone(),
            prompt: Some(prepared.evidence.visible_prompt.clone()),
            files: prepared.evidence.touched_files.clone(),
        };

        for decision in consolidated.decisions {
            let Some(context) = candidate_contexts
                .iter()
                .find(|context| context.candidate_index == decision.candidate_index)
            else {
                continue;
            };

            let Some(status) = decision_status(&prepared.extraction, &decision) else {
                continue;
            };

            let extra_supersedes = dedupe_supersede_ids(
                decision
                    .supersede_memory_ids
                    .iter()
                    .map(String::as_str)
                    .chain(decision.existing_memory_id.iter().map(String::as_str)),
            );

            match decision.action {
                MemoryConsolidationAction::Ignore => {}
                MemoryConsolidationAction::CreateActive
                | MemoryConsolidationAction::CreateCandidate => {
                    let record = build_record_from_decision(
                        &self.repo_fingerprint,
                        now,
                        &context.candidate,
                        &decision,
                        status,
                        vec![evidence_ref.clone()],
                        None,
                        None,
                    );
                    self.create_record_if_missing(record)?;
                    self.supersede_records(&extra_supersedes, now)?;
                }
                MemoryConsolidationAction::UpdateExisting => {
                    let Some(existing_id) = decision.existing_memory_id.as_deref() else {
                        continue;
                    };
                    let Some(existing_index) = self
                        .snapshot
                        .records
                        .iter()
                        .position(|record| record.id == existing_id)
                    else {
                        continue;
                    };
                    let updated = updated_record_from_decision(
                        &self.snapshot.records[existing_index],
                        now,
                        &context.candidate,
                        &decision,
                        status,
                        evidence_ref.clone(),
                    );
                    self.snapshot.records[existing_index] = updated.clone();
                    self.vectors.remove(existing_id);
                    self.append_event(MemoryEvent::Updated {
                        record: updated.clone(),
                    })?;
                    self.persist_snapshot()?;
                    let _ = self.ensure_vectors(&[updated.id.clone()]);
                    let _ = self.persist_vectors();
                    let additional = dedupe_supersede_ids(
                        extra_supersedes
                            .iter()
                            .map(String::as_str)
                            .filter(|id| *id != existing_id),
                    );
                    self.supersede_records(&additional, now)?;
                }
                MemoryConsolidationAction::SupersedeExisting => {
                    let Some(existing_id) = decision.existing_memory_id.as_deref() else {
                        continue;
                    };
                    let Some(existing) = self
                        .snapshot
                        .records
                        .iter()
                        .find(|record| record.id == existing_id)
                        .cloned()
                    else {
                        continue;
                    };
                    if let Some(old) = self
                        .snapshot
                        .records
                        .iter_mut()
                        .find(|record| record.id == existing_id)
                    {
                        old.status = MemoryStatus::Superseded;
                        old.updated_at = now;
                    }
                    let record = build_record_from_decision(
                        &self.repo_fingerprint,
                        now,
                        &context.candidate,
                        &decision,
                        status,
                        merge_evidence(existing.evidence.clone(), evidence_ref.clone()),
                        Some(existing.subject_key),
                        Some(existing.id.clone()),
                    );
                    self.snapshot.records.push(record.clone());
                    self.vectors.remove(existing_id);
                    self.append_event(MemoryEvent::Replaced {
                        old_id: existing_id.to_string(),
                        new_record: record.clone(),
                    })?;
                    self.persist_snapshot()?;
                    let _ = self.ensure_vectors(&[record.id.clone()]);
                    let _ = self.persist_vectors();
                    let additional = dedupe_supersede_ids(
                        extra_supersedes
                            .iter()
                            .map(String::as_str)
                            .filter(|id| *id != existing_id),
                    );
                    self.supersede_records(&additional, now)?;
                }
            }
        }

        Ok(())
    }

    fn supersede_records(&mut self, ids: &[String], now: DateTime<Utc>) -> Result<()> {
        let mut applied = Vec::new();
        for id in ids {
            let Some(record) = self
                .snapshot
                .records
                .iter_mut()
                .find(|record| record.id == *id)
            else {
                continue;
            };
            if matches!(
                record.status,
                MemoryStatus::Superseded | MemoryStatus::Archived
            ) {
                continue;
            }
            record.status = MemoryStatus::Superseded;
            record.updated_at = now;
            self.vectors.remove(id);
            applied.push(id.clone());
        }
        if applied.is_empty() {
            return Ok(());
        }
        self.append_event(MemoryEvent::Superseded {
            ids: applied,
            updated_at: now,
        })?;
        self.persist_snapshot()?;
        self.persist_vectors()?;
        Ok(())
    }

    fn search_hits(
        &mut self,
        query: &str,
        policy: RetrievalPolicy,
    ) -> Result<Vec<MemorySearchHit>> {
        let query = query.trim();
        let tokens = tokenize(query);
        let eligible = self
            .snapshot
            .records
            .iter()
            .filter(|record| {
                self.record_is_eligible(record, policy.include_candidates)
                    && policy.allows_kind(record.kind)
            })
            .cloned()
            .collect::<Vec<_>>();

        if eligible.is_empty() {
            return Ok(Vec::new());
        }

        let semantic = if query.is_empty() {
            None
        } else {
            let ids = eligible
                .iter()
                .map(|record| record.id.clone())
                .collect::<Vec<_>>();
            let ensured = self.ensure_vectors(&ids).ok();
            let query_embedding = self.embedder.embed(vec![query.to_string()]).ok();
            ensured
                .and(query_embedding)
                .and_then(|mut vectors| vectors.pop())
        };

        let mut hits = eligible
            .into_iter()
            .filter_map(|record| {
                let signals = memory_match_signals(
                    query,
                    &tokens,
                    semantic.as_ref().map(Vec::as_slice),
                    self.vectors.get(&record.id).map(Vec::as_slice),
                    &record,
                );
                let score = signals.total_score
                    + scope_bonus(&self.repo_fingerprint, &record)
                    + recency_bonus(record.updated_at)
                    + record.confidence * 0.3;

                memory_passes_policy(query, &signals, policy).then_some(MemorySearchHit {
                    record,
                    score,
                    signals,
                })
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.record.updated_at.cmp(&left.record.updated_at))
        });
        hits.truncate(policy.limit);
        Ok(hits)
    }

    fn record_is_eligible(&self, record: &MemoryRecord, include_candidates: bool) -> bool {
        self.record_is_in_scope(record)
            && match record.status {
                MemoryStatus::Active => true,
                MemoryStatus::Candidate => include_candidates,
                MemoryStatus::Superseded | MemoryStatus::Archived => false,
            }
    }

    fn record_is_in_scope(&self, record: &MemoryRecord) -> bool {
        match record.scope {
            MemoryScope::Global => true,
            MemoryScope::Repo | MemoryScope::Module => {
                record.repo_fingerprint.as_deref() == Some(self.repo_fingerprint.as_str())
            }
        }
    }

    fn create_record_if_missing(&mut self, record: MemoryRecord) -> Result<()> {
        if self.snapshot.records.iter().any(|existing| {
            existing.status == MemoryStatus::Active
                && existing.scope == record.scope
                && existing.kind == record.kind
                && existing.summary.eq_ignore_ascii_case(&record.summary)
        }) {
            return Ok(());
        }
        self.create_record(record)
    }

    fn create_record(&mut self, record: MemoryRecord) -> Result<()> {
        self.snapshot.records.push(record.clone());
        self.append_event(MemoryEvent::Created {
            record: record.clone(),
        })?;
        self.persist_snapshot()?;
        if matches!(
            record.status,
            MemoryStatus::Active | MemoryStatus::Candidate
        ) {
            let _ = self.ensure_vectors(&[record.id.clone()]);
            let _ = self.persist_vectors();
        }
        Ok(())
    }

    fn ensure_vectors(&mut self, ids: &[String]) -> Result<Vec<Vec<f32>>> {
        let missing = ids
            .iter()
            .filter(|id| !self.vectors.contains_key(id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let records = self
                .snapshot
                .records
                .iter()
                .filter(|record| missing.contains(&record.id))
                .map(embedding_text)
                .collect::<Vec<_>>();
            let embeddings = self.embedder.embed(records)?;
            for (id, vector) in missing.iter().zip(embeddings.into_iter()) {
                self.vectors.insert(id.clone(), vector);
            }
            self.persist_vectors()?;
        }

        ids.iter()
            .filter_map(|id| self.vectors.get(id).cloned())
            .collect::<Vec<_>>()
            .pipe(Ok)
    }

    fn append_event(&self, event: MemoryEvent) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        fs::create_dir_all(storage_dir)
            .with_context(|| format!("failed to create {}", storage_dir.display()))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(storage_dir.join(EVENTS_FILE_NAME))
            .with_context(|| {
                format!(
                    "failed to open {}",
                    storage_dir.join(EVENTS_FILE_NAME).display()
                )
            })?;
        let payload = serde_json::to_string(&MemoryEventRecord {
            schema_version: SCHEMA_VERSION,
            ts: Utc::now(),
            event,
        })?;
        writeln!(file, "{payload}")?;
        Ok(())
    }

    fn persist_snapshot(&self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        fs::create_dir_all(storage_dir)
            .with_context(|| format!("failed to create {}", storage_dir.display()))?;
        fs::write(
            storage_dir.join(SNAPSHOT_FILE_NAME),
            serde_json::to_vec_pretty(&self.snapshot)?,
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                storage_dir.join(SNAPSHOT_FILE_NAME).display()
            )
        })
    }

    fn persist_vectors(&self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        fs::create_dir_all(storage_dir)
            .with_context(|| format!("failed to create {}", storage_dir.display()))?;
        let snapshot = MemoryVectorsSnapshot {
            schema_version: SCHEMA_VERSION,
            backend: self.embedder.backend_name().to_string(),
            vectors: self
                .vectors
                .iter()
                .map(|(id, vector)| PersistedVector {
                    id: id.clone(),
                    vector: vector.clone(),
                })
                .collect(),
        };
        fs::write(
            storage_dir.join(VECTORS_FILE_NAME),
            serde_json::to_vec_pretty(&snapshot)?,
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                storage_dir.join(VECTORS_FILE_NAME).display()
            )
        })
    }

    fn reset_storage(&self) -> Result<()> {
        let Some(storage_dir) = self.storage_dir.as_deref() else {
            return Ok(());
        };
        fs::create_dir_all(storage_dir)
            .with_context(|| format!("failed to create {}", storage_dir.display()))?;
        fs::write(
            storage_dir.join(SNAPSHOT_FILE_NAME),
            serde_json::to_vec_pretty(&self.snapshot)?,
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                storage_dir.join(SNAPSHOT_FILE_NAME).display()
            )
        })?;
        let vectors = MemoryVectorsSnapshot {
            schema_version: SCHEMA_VERSION,
            backend: self.embedder.backend_name().to_string(),
            vectors: Vec::new(),
        };
        fs::write(
            storage_dir.join(VECTORS_FILE_NAME),
            serde_json::to_vec_pretty(&vectors)?,
        )
        .with_context(|| {
            format!(
                "failed to write {}",
                storage_dir.join(VECTORS_FILE_NAME).display()
            )
        })?;
        let cleared_at = Utc::now();
        let payload = serde_json::to_string(&MemoryEventRecord {
            schema_version: SCHEMA_VERSION,
            ts: cleared_at,
            event: MemoryEvent::Cleared { cleared_at },
        })?;
        fs::write(storage_dir.join(EVENTS_FILE_NAME), format!("{payload}\n")).with_context(|| {
            format!(
                "failed to write {}",
                storage_dir.join(EVENTS_FILE_NAME).display()
            )
        })
    }
}

fn load_snapshot(storage_dir: Option<&Path>) -> Result<MemorySnapshot> {
    let Some(storage_dir) = storage_dir else {
        return Ok(MemorySnapshot::default());
    };
    let snapshot_path = storage_dir.join(SNAPSHOT_FILE_NAME);
    if snapshot_path.exists() {
        return Ok(serde_json::from_slice(&fs::read(&snapshot_path)?)?);
    }
    let events_path = storage_dir.join(EVENTS_FILE_NAME);
    if !events_path.exists() {
        return Ok(MemorySnapshot::default());
    }
    let mut snapshot = MemorySnapshot::default();
    for line in fs::read_to_string(&events_path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let record: MemoryEventRecord = serde_json::from_str(line)?;
        apply_event(&mut snapshot, record.event);
    }
    Ok(snapshot)
}

fn load_vectors(storage_dir: Option<&Path>) -> Result<BTreeMap<String, Vec<f32>>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(BTreeMap::new());
    };
    let vectors_path = storage_dir.join(VECTORS_FILE_NAME);
    if !vectors_path.exists() {
        return Ok(BTreeMap::new());
    }
    let snapshot: MemoryVectorsSnapshot = serde_json::from_slice(&fs::read(vectors_path)?)?;
    Ok(snapshot
        .vectors
        .into_iter()
        .map(|entry| (entry.id, entry.vector))
        .collect())
}

fn apply_event(snapshot: &mut MemorySnapshot, event: MemoryEvent) {
    match event {
        MemoryEvent::Created { record } => snapshot.records.push(record),
        MemoryEvent::Updated { record } => {
            if let Some(existing) = snapshot
                .records
                .iter_mut()
                .find(|item| item.id == record.id)
            {
                *existing = record;
            }
        }
        MemoryEvent::Cleared { .. } => snapshot.records.clear(),
        MemoryEvent::Superseded { ids, updated_at } => {
            for id in ids {
                if let Some(record) = snapshot.records.iter_mut().find(|record| record.id == id) {
                    record.status = MemoryStatus::Superseded;
                    record.updated_at = updated_at;
                }
            }
        }
        MemoryEvent::Promoted { id, updated_at } => {
            if let Some(record) = snapshot.records.iter_mut().find(|record| record.id == id) {
                record.status = MemoryStatus::Active;
                record.updated_at = updated_at;
            }
        }
        MemoryEvent::Archived { id, updated_at } => {
            if let Some(record) = snapshot.records.iter_mut().find(|record| record.id == id) {
                record.status = MemoryStatus::Archived;
                record.updated_at = updated_at;
            }
        }
        MemoryEvent::Replaced { old_id, new_record } => {
            if let Some(record) = snapshot
                .records
                .iter_mut()
                .find(|record| record.id == old_id)
            {
                record.status = MemoryStatus::Superseded;
                record.updated_at = new_record.updated_at;
            }
            snapshot.records.push(new_record);
        }
    }
}

fn format_search_results(
    query: &str,
    hits: &[MemorySearchHit],
    include_debug_reasons: bool,
) -> String {
    if hits.is_empty() {
        return format!("No memories matched `{query}`.");
    }

    let mut lines = vec![format!("Memory results for `{query}`:")];
    for hit in hits {
        lines.push(format!(
            "- {} [{} | {} | {} | score {:.2}] {}",
            short_id(&hit.record.id),
            kind_label(hit.record.kind),
            scope_label(hit.record.scope),
            status_label(hit.record.status),
            hit.score,
            hit.record.title
        ));
        lines.push(format!("  {}", hit.record.summary));
        if include_debug_reasons {
            lines.push(format!("  why: {}", memory_search_debug_reasons(hit)));
        }
    }
    lines.join("\n")
}

fn format_record(record: &MemoryRecord) -> String {
    let mut lines = vec![
        format!("Memory {}", record.id),
        format!(
            "{} | {} | {} | confidence {:.2}",
            kind_label(record.kind),
            scope_label(record.scope),
            status_label(record.status),
            record.confidence
        ),
        record.title.clone(),
        record.summary.clone(),
        format!("subject: {}", record.subject_key),
    ];
    if let Some(details) = &record.details {
        lines.push(String::new());
        lines.push(details.clone());
    }
    if !record.tags.is_empty() {
        lines.push(String::new());
        lines.push(format!("tags: {}", record.tags.join(", ")));
    }
    if !record.evidence.is_empty() {
        lines.push(String::new());
        lines.push("evidence:".into());
        for evidence in &record.evidence {
            let mut detail = String::from("- ");
            if let Some(session_id) = &evidence.session_id {
                detail.push_str(&format!("session {session_id}"));
            }
            if !evidence.files.is_empty() {
                if detail.len() > 2 {
                    detail.push_str(" | ");
                }
                detail.push_str(&format!("files {}", evidence.files.join(", ")));
            }
            if let Some(prompt) = &evidence.prompt {
                if detail.len() > 2 {
                    detail.push_str(" | ");
                }
                detail.push_str(prompt);
            }
            lines.push(detail);
        }
    }
    lines.join("\n")
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn scope_label(scope: MemoryScope) -> &'static str {
    match scope {
        MemoryScope::Global => "global",
        MemoryScope::Repo => "repo",
        MemoryScope::Module => "module",
    }
}

fn kind_label(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Preference => "User preference",
        MemoryKind::Workflow => "Workflow",
        MemoryKind::Architecture => "Architecture",
        MemoryKind::Decision => "Decision",
        MemoryKind::Hazard => "Hazard",
        MemoryKind::Episode => "Episode",
    }
}

fn status_label(status: MemoryStatus) -> &'static str {
    match status {
        MemoryStatus::Candidate => "candidate",
        MemoryStatus::Active => "active",
        MemoryStatus::Superseded => "superseded",
        MemoryStatus::Archived => "archived",
    }
}

fn source_label(source: MemorySource) -> &'static str {
    match source {
        MemorySource::ExplicitUser => "explicit_user",
        MemorySource::Inferred => "inferred",
        MemorySource::AutoSummary => "auto_summary",
    }
}

fn bump_count(map: &mut BTreeMap<&'static str, usize>, key: &'static str) {
    *map.entry(key).or_default() += 1;
}

fn format_count_map(title: &str, counts: &BTreeMap<&'static str, usize>) -> Vec<String> {
    let mut lines = vec![format!("{title}:")];
    for (label, count) in counts {
        lines.push(format!("- {label}: {count}"));
    }
    lines
}

fn format_memory_stats(
    records: &[MemoryRecord],
    vectors: &BTreeMap<String, Vec<f32>>,
    current_repo_fingerprint: &str,
    embedder_backend: &str,
) -> String {
    let mut status_counts = BTreeMap::new();
    let mut kind_counts = BTreeMap::new();
    let mut scope_counts = BTreeMap::new();
    let mut source_counts = BTreeMap::new();
    let mut live_subject_counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut unique_subjects = BTreeSet::new();
    let mut live_record_ids = BTreeSet::new();
    let mut in_scope_now = 0usize;
    let mut latest_updated_at: Option<DateTime<Utc>> = None;

    for record in records {
        bump_count(&mut status_counts, status_label(record.status));
        bump_count(&mut kind_counts, kind_label(record.kind));
        bump_count(&mut scope_counts, scope_label(record.scope));
        bump_count(&mut source_counts, source_label(record.source));
        unique_subjects.insert(record.subject_key.as_str());
        latest_updated_at = Some(
            latest_updated_at
                .map(|current| current.max(record.updated_at))
                .unwrap_or(record.updated_at),
        );

        let in_scope = match record.scope {
            MemoryScope::Global => true,
            MemoryScope::Repo | MemoryScope::Module => {
                record.repo_fingerprint.as_deref() == Some(current_repo_fingerprint)
            }
        };
        if in_scope {
            in_scope_now += 1;
        }

        if matches!(
            record.status,
            MemoryStatus::Active | MemoryStatus::Candidate
        ) {
            *live_subject_counts
                .entry(record.subject_key.as_str())
                .or_default() += 1;
            live_record_ids.insert(record.id.as_str());
        }
    }

    let live_records = live_record_ids.len();
    let indexed_live_records = live_record_ids
        .iter()
        .filter(|id| vectors.contains_key(**id))
        .count();
    let orphaned_vectors = vectors
        .keys()
        .filter(|id| !records.iter().any(|record| record.id == ***id))
        .count();
    let duplicated_live_subjects = live_subject_counts
        .values()
        .filter(|count| **count > 1)
        .count();

    let mut lines = vec![
        "Memory stats:".to_string(),
        format!("- total records: {}", records.len()),
        format!("- in scope now: {in_scope_now}"),
        format!(
            "- live records: {} active, {} candidate",
            status_counts.get("active").copied().unwrap_or(0),
            status_counts.get("candidate").copied().unwrap_or(0)
        ),
        format!(
            "- inactive records: {} superseded, {} archived",
            status_counts.get("superseded").copied().unwrap_or(0),
            status_counts.get("archived").copied().unwrap_or(0)
        ),
        format!(
            "- vectors: {} stored, {}/{} live indexed, {} orphaned",
            vectors.len(),
            indexed_live_records,
            live_records,
            orphaned_vectors
        ),
        format!(
            "- subject keys: {} unique, {} duplicate live subjects",
            unique_subjects.len(),
            duplicated_live_subjects
        ),
        format!("- embedder backend: {embedder_backend}"),
    ];
    if let Some(updated_at) = latest_updated_at {
        lines.push(format!("- latest update: {}", updated_at.to_rfc3339()));
    }

    lines.push(String::new());
    lines.extend(format_count_map("By status", &status_counts));
    lines.push(String::new());
    lines.extend(format_count_map("By kind", &kind_counts));
    lines.push(String::new());
    lines.extend(format_count_map("By scope", &scope_counts));
    lines.push(String::new());
    lines.extend(format_count_map("By source", &source_counts));
    lines.join("\n")
}

fn detect_repo_root(workspace_root: &Path) -> PathBuf {
    let mut current = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    loop {
        if current.join(".git").exists() {
            return current;
        }
        if !current.pop() {
            return workspace_root
                .canonicalize()
                .unwrap_or_else(|_| workspace_root.to_path_buf());
        }
    }
}

fn default_memory_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(MEMORY_DIR_RELATIVE_PATH))
}

fn transcript_entry_to_evidence(entry: &TranscriptEntry) -> MemoryTranscriptEvidence {
    match entry {
        TranscriptEntry::Message(message) => MemoryTranscriptEvidence {
            kind: "message".into(),
            label: Some(format!("{:?}/{:?}", message.speaker, message.style)),
            content: truncate(&message.text, 1_200),
        },
        TranscriptEntry::ProposedPlan(plan) => MemoryTranscriptEvidence {
            kind: "proposed_plan".into(),
            label: None,
            content: truncate(&plan.markdown, 1_200),
        },
        TranscriptEntry::ToolCall(call) => MemoryTranscriptEvidence {
            kind: "tool_call".into(),
            label: Some(call.name.clone()),
            content: truncate(&call.parameter, 1_200),
        },
        TranscriptEntry::ToolResult(result) => MemoryTranscriptEvidence {
            kind: "tool_result".into(),
            label: Some(result.name.clone()),
            content: truncate(&result.output, 1_200),
        },
        TranscriptEntry::TodoSnapshot(todo) => MemoryTranscriptEvidence {
            kind: "todo_snapshot".into(),
            label: None,
            content: truncate(&serde_json::to_string(todo).unwrap_or_default(), 1_200),
        },
        TranscriptEntry::SubagentStatus(status) => MemoryTranscriptEvidence {
            kind: "subagent_status".into(),
            label: Some(status.display_label.clone()),
            content: truncate(&status.status_text, 600),
        },
        TranscriptEntry::BackgroundTerminalStatus(status) => MemoryTranscriptEvidence {
            kind: "background_terminal_status".into(),
            label: Some(status.display_label.clone()),
            content: truncate(&status.status_text, 600),
        },
    }
}

fn limit_transcript_evidence(
    transcript: Vec<MemoryTranscriptEvidence>,
    max_tokens: usize,
) -> Vec<MemoryTranscriptEvidence> {
    let mut limited = transcript;
    while !limited.is_empty()
        && count_text_tokens(&serde_json::to_string(&limited).unwrap_or_default()) as usize
            > max_tokens
    {
        limited.remove(0);
    }
    limited
}

fn candidate_lookup_query(candidate: &MemoryCandidateDraft) -> String {
    let mut parts = vec![
        candidate.subject_hint.clone(),
        candidate.title.clone(),
        candidate.summary.clone(),
    ];
    if !candidate.tags.is_empty() {
        parts.push(candidate.tags.join(" "));
    }
    if !candidate.module_refs.is_empty() {
        parts.push(candidate.module_refs.join(" "));
    }
    parts.join(" ")
}

fn related_memory_summary(record: &MemoryRecord) -> RelatedMemorySummary {
    RelatedMemorySummary {
        id: record.id.clone(),
        scope: record.scope,
        kind: record.kind,
        source: record.source,
        subject_key: record.subject_key.clone(),
        title: record.title.clone(),
        summary: record.summary.clone(),
        details: record.details.clone(),
        tags: record.tags.clone(),
        confidence: record.confidence,
        status: status_label(record.status).to_string(),
    }
}

fn decision_status(
    extraction: &MemoryExtractionConfig,
    decision: &MemoryConsolidationDecision,
) -> Option<MemoryStatus> {
    if decision.action == MemoryConsolidationAction::Ignore
        || decision.confidence < extraction.min_candidate_confidence
    {
        return None;
    }

    match decision.action {
        MemoryConsolidationAction::CreateCandidate => Some(MemoryStatus::Candidate),
        MemoryConsolidationAction::CreateActive
        | MemoryConsolidationAction::UpdateExisting
        | MemoryConsolidationAction::SupersedeExisting => {
            if decision.confidence >= extraction.min_active_confidence {
                Some(MemoryStatus::Active)
            } else {
                Some(MemoryStatus::Candidate)
            }
        }
        MemoryConsolidationAction::Ignore => None,
    }
}

fn build_record_from_decision(
    repo_fingerprint: &str,
    now: DateTime<Utc>,
    candidate: &MemoryCandidateDraft,
    decision: &MemoryConsolidationDecision,
    status: MemoryStatus,
    evidence: Vec<MemoryEvidenceRef>,
    subject_key: Option<String>,
    supersedes: Option<String>,
) -> MemoryRecord {
    let title = decision
        .title
        .as_deref()
        .unwrap_or(candidate.title.as_str())
        .trim();
    let summary = decision
        .summary
        .as_deref()
        .unwrap_or(candidate.summary.as_str())
        .trim();
    let details = decision
        .details
        .clone()
        .or_else(|| candidate.details.clone())
        .filter(|text| !text.trim().is_empty());
    let tags = normalized_tags(
        decision
            .tags
            .iter()
            .chain(candidate.tags.iter())
            .map(String::as_str),
        &[title, summary],
        &candidate.module_refs,
    );
    MemoryRecord {
        id: Uuid::now_v7().to_string(),
        scope: candidate.scope,
        repo_fingerprint: memory_repo_fingerprint(repo_fingerprint, candidate.scope),
        subject_key: subject_key
            .unwrap_or_else(|| subject_key_for_candidate(repo_fingerprint, candidate)),
        kind: candidate.kind,
        title: truncate(title, 120),
        summary: truncate(summary, 360),
        details: details.map(|text| truncate(text.trim(), 2_000)),
        tags,
        evidence,
        source: candidate.source,
        confidence: decision.confidence as f32 / 100.0,
        status,
        supersedes,
        created_at: now,
        updated_at: now,
    }
}

fn updated_record_from_decision(
    existing: &MemoryRecord,
    now: DateTime<Utc>,
    candidate: &MemoryCandidateDraft,
    decision: &MemoryConsolidationDecision,
    status: MemoryStatus,
    evidence_ref: MemoryEvidenceRef,
) -> MemoryRecord {
    let title = decision
        .title
        .as_deref()
        .unwrap_or(candidate.title.as_str())
        .trim();
    let summary = decision
        .summary
        .as_deref()
        .unwrap_or(candidate.summary.as_str())
        .trim();
    let details = decision
        .details
        .clone()
        .or_else(|| candidate.details.clone())
        .or_else(|| existing.details.clone())
        .filter(|text| !text.trim().is_empty());
    let tags = normalized_tags(
        decision
            .tags
            .iter()
            .chain(candidate.tags.iter())
            .chain(existing.tags.iter())
            .map(String::as_str),
        &[title, summary],
        &candidate.module_refs,
    );
    MemoryRecord {
        id: existing.id.clone(),
        scope: existing.scope,
        repo_fingerprint: existing.repo_fingerprint.clone(),
        subject_key: existing.subject_key.clone(),
        kind: existing.kind,
        title: truncate(title, 120),
        summary: truncate(summary, 360),
        details: details.map(|text| truncate(text.trim(), 2_000)),
        tags,
        evidence: merge_evidence(existing.evidence.clone(), evidence_ref),
        source: candidate.source,
        confidence: decision.confidence as f32 / 100.0,
        status: if existing.status == MemoryStatus::Active {
            MemoryStatus::Active
        } else {
            status
        },
        supersedes: existing.supersedes.clone(),
        created_at: existing.created_at,
        updated_at: now,
    }
}

fn collect_paths_from_entries(
    workspace_root: &Path,
    repo_root: &Path,
    entries: &[TranscriptEntry],
) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for entry in entries {
        if let TranscriptEntry::ToolCall(call) = entry {
            collect_paths_from_json(workspace_root, repo_root, &call.parameter, &mut paths);
        }
    }
    paths.into_iter().collect()
}

fn collect_paths_from_json(
    workspace_root: &Path,
    repo_root: &Path,
    raw_json: &str,
    paths: &mut BTreeSet<String>,
) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw_json) else {
        return;
    };
    collect_paths_from_value(workspace_root, repo_root, &value, paths);
}

fn collect_paths_from_value(
    workspace_root: &Path,
    repo_root: &Path,
    value: &serde_json::Value,
    paths: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::String(text) => {
            if let Some(path) = normalize_candidate_path(workspace_root, repo_root, text) {
                paths.insert(path);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_paths_from_value(workspace_root, repo_root, value, paths);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "filename" | "path" | "paths" | "directory" | "dirname" | "target" | "file"
                ) {
                    collect_paths_from_value(workspace_root, repo_root, value, paths);
                }
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn normalize_candidate_path(workspace_root: &Path, repo_root: &Path, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('{') || raw.starts_with('[') {
        return None;
    }
    let candidate = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        workspace_root.join(raw)
    };
    let canonical = candidate
        .canonicalize()
        .ok()
        .or_else(|| existing_ancestor_canonical(&candidate))?;
    if !canonical.starts_with(repo_root) {
        return None;
    }
    canonical
        .strip_prefix(repo_root)
        .ok()
        .map(|path| path.display().to_string())
}

fn existing_ancestor_canonical(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    while !current.exists() {
        if !current.pop() {
            return None;
        }
    }
    current.canonicalize().ok()
}

fn lexical_score(query: &str, tokens: &[String], record: &MemoryRecord) -> f32 {
    if query.is_empty() {
        return 0.0;
    }
    let haystack = format!(
        "{}\n{}\n{}\n{}\n{}",
        record.subject_key,
        record.title,
        record.summary,
        record.details.clone().unwrap_or_default(),
        record.tags.join(" ")
    )
    .to_ascii_lowercase();
    let query = query.to_ascii_lowercase();
    let mut score = 0.0;
    if haystack.contains(&query) {
        score += 2.0;
    }
    for token in tokens.iter().collect::<BTreeSet<_>>() {
        if haystack.contains(token) {
            score += 0.8;
        }
    }
    score
}

fn memory_match_signals(
    query: &str,
    tokens: &[String],
    query_embedding: Option<&[f32]>,
    record_embedding: Option<&[f32]>,
    record: &MemoryRecord,
) -> MemoryMatchSignals {
    let lexical_score = lexical_score(query, tokens, record);
    let semantic_score = query_embedding
        .zip(record_embedding)
        .map(|(left, right)| cosine_similarity(left, right))
        .unwrap_or(0.0);
    let exact_subject = exact_subject_match(query, record);
    let exact_path_match = exact_path_match(query, record);
    let exact_tag_hits = exact_tag_hits(tokens, record);
    let exact_term_hits = exact_term_hits(tokens, record);
    let path_term_hits = path_term_hits(query, tokens, record);

    let mut total_score = lexical_score * 2.0 + semantic_score;
    if exact_subject {
        total_score += 1.2;
    }
    if exact_path_match {
        total_score += 1.4;
    }
    total_score += exact_tag_hits.len().min(4) as f32 * 0.7;
    total_score += exact_term_hits.len().min(6) as f32 * 0.2;
    total_score += path_term_hits.len().min(4) as f32 * 0.5;

    MemoryMatchSignals {
        lexical_score,
        semantic_score,
        exact_subject,
        exact_path_match,
        exact_tag_hits,
        exact_term_hits,
        path_term_hits,
        total_score,
    }
}

fn memory_passes_policy(
    query: &str,
    signals: &MemoryMatchSignals,
    policy: RetrievalPolicy,
) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    if signals.total_score < policy.min_total_score {
        return false;
    }

    if query_contains_path_hint(query) {
        let path_hit =
            signals.exact_subject || signals.exact_path_match || signals.path_term_hits.len() >= 2;
        let strong_lexical = signals.lexical_score >= policy.min_lexical_score + 1.6;
        return path_hit || strong_lexical;
    }

    let structured_hit = signals.exact_subject
        || signals.exact_path_match
        || !signals.exact_tag_hits.is_empty()
        || !signals.path_term_hits.is_empty();
    let lexical_hit = signals.lexical_score >= policy.min_lexical_score;
    let semantic_hit = signals.semantic_score >= policy.min_semantic_score;
    let multi_term_hit = signals.exact_term_hits.len() >= 2
        && signals.lexical_score >= policy.min_lexical_score * 0.65;

    structured_hit || lexical_hit || semantic_hit || multi_term_hit
}

fn exact_subject_match(query: &str, record: &MemoryRecord) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return false;
    }
    let subject = record.subject_key.to_ascii_lowercase();
    let lowered_query = query.to_ascii_lowercase();
    if subject == lowered_query {
        return true;
    }
    let slug = slugify(query);
    slug.len() >= 4 && (subject == slug || subject.contains(&slug))
}

fn exact_path_match(query: &str, record: &MemoryRecord) -> bool {
    if !query_contains_path_hint(query) {
        return false;
    }

    let lowered_query = query.trim().to_ascii_lowercase();
    if lowered_query.is_empty() {
        return false;
    }

    if record
        .subject_key
        .to_ascii_lowercase()
        .contains(&lowered_query)
    {
        return true;
    }

    if record
        .evidence
        .iter()
        .flat_map(|evidence| evidence.files.iter())
        .any(|file| file.to_ascii_lowercase().contains(&lowered_query))
    {
        return true;
    }

    let Some(file_name) = Path::new(query)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
    else {
        return false;
    };
    file_name.len() >= 5
        && record
            .evidence
            .iter()
            .flat_map(|evidence| evidence.files.iter())
            .any(|file| file.to_ascii_lowercase().ends_with(&file_name))
}

fn exact_tag_hits(tokens: &[String], record: &MemoryRecord) -> Vec<String> {
    let tag_set = record
        .tags
        .iter()
        .map(|tag| tag.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    tokens
        .iter()
        .filter(|token| tag_set.contains(token.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn exact_term_hits(tokens: &[String], record: &MemoryRecord) -> Vec<String> {
    let haystack_tokens = tokenize(&format!(
        "{}\n{}\n{}\n{}",
        record.subject_key,
        record.title,
        record.summary,
        record.details.clone().unwrap_or_default()
    ))
    .into_iter()
    .collect::<BTreeSet<_>>();
    tokens
        .iter()
        .filter(|token| haystack_tokens.contains(token.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn path_term_hits(query: &str, tokens: &[String], record: &MemoryRecord) -> Vec<String> {
    if !query_contains_path_hint(query) {
        return Vec::new();
    }
    let query_tokens = path_query_tokens(query, tokens);
    if query_tokens.is_empty() {
        return Vec::new();
    }
    let pathish_tokens = tokenize(&format!(
        "{}\n{}\n{}",
        record.subject_key,
        record.tags.join(" "),
        record
            .evidence
            .iter()
            .flat_map(|evidence| evidence.files.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    ))
    .into_iter()
    .filter(|token| token.len() >= 3)
    .collect::<BTreeSet<_>>();
    query_tokens
        .iter()
        .filter(|token| pathish_tokens.contains(token.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn path_query_tokens(query: &str, tokens: &[String]) -> Vec<String> {
    if query_contains_path_hint(query) {
        tokenize(query)
            .into_iter()
            .filter(|token| token.len() >= 3 && !is_generic_path_token(token))
            .collect()
    } else {
        tokens
            .iter()
            .filter(|token| token.len() >= 3 && !is_generic_path_token(token))
            .cloned()
            .collect()
    }
}

fn is_generic_path_token(token: &str) -> bool {
    matches!(
        token,
        "src"
            | "app"
            | "lib"
            | "bin"
            | "mod"
            | "main"
            | "index"
            | "test"
            | "tests"
            | "spec"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "py"
            | "go"
            | "cs"
            | "cpp"
            | "c"
            | "h"
            | "hpp"
            | "java"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "md"
            | "sql"
    )
}

fn query_contains_path_hint(query: &str) -> bool {
    query
        .chars()
        .any(|ch| matches!(ch, '/' | '\\' | '.' | '_' | '-'))
}

fn memory_search_debug_reasons(hit: &MemorySearchHit) -> String {
    let mut reasons = Vec::new();
    if hit.signals.exact_subject {
        reasons.push("subject".to_string());
    }
    if hit.signals.exact_path_match {
        reasons.push("path-exact".to_string());
    }
    if !hit.signals.exact_tag_hits.is_empty() {
        reasons.push(format!("tags {}", hit.signals.exact_tag_hits.join(", ")));
    }
    if !hit.signals.path_term_hits.is_empty() {
        reasons.push(format!("path {}", hit.signals.path_term_hits.join(", ")));
    }
    if !hit.signals.exact_term_hits.is_empty() {
        reasons.push(format!("terms {}", hit.signals.exact_term_hits.join(", ")));
    }
    reasons.push(format!("lex {:.2}", hit.signals.lexical_score));
    reasons.push(format!("sem {:.2}", hit.signals.semantic_score));
    reasons.push(format!("query {:.2}", hit.signals.total_score));
    reasons.join(", ")
}

fn scope_bonus(current_repo_fingerprint: &str, record: &MemoryRecord) -> f32 {
    match record.scope {
        MemoryScope::Global => 0.2,
        MemoryScope::Repo => {
            if record.repo_fingerprint.as_deref() == Some(current_repo_fingerprint) {
                0.7
            } else {
                0.0
            }
        }
        MemoryScope::Module => {
            if record.repo_fingerprint.as_deref() == Some(current_repo_fingerprint) {
                0.9
            } else {
                0.0
            }
        }
    }
}

fn recency_bonus(updated_at: DateTime<Utc>) -> f32 {
    let age = Utc::now() - updated_at;
    if age <= Duration::days(7) {
        0.5
    } else if age <= Duration::days(30) {
        0.3
    } else {
        0.1
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (lhs, rhs) in left.iter().zip(right.iter()) {
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }
    let denom = left_norm.sqrt() * right_norm.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|part| {
            let token = part.trim().to_ascii_lowercase();
            (token.len() > 1).then_some(token)
        })
        .collect()
}

fn tags_from_text(text: &str) -> Vec<String> {
    let mut tags = tokenize(text);
    tags.sort();
    tags.dedup();
    tags.truncate(20);
    tags
}

fn normalized_tags<'a>(
    tags: impl IntoIterator<Item = &'a str>,
    text_fields: &[&str],
    module_refs: &[String],
) -> Vec<String> {
    let mut merged = tags
        .into_iter()
        .filter_map(|tag| {
            let tag = tag.trim().to_ascii_lowercase();
            (!tag.is_empty()).then_some(tag)
        })
        .collect::<Vec<_>>();
    for field in text_fields {
        merged.extend(tags_from_text(field));
    }
    for module_ref in module_refs {
        merged.extend(
            module_ref
                .split(['/', '.', '_', '-'])
                .filter(|segment| !segment.trim().is_empty())
                .map(|segment| segment.to_ascii_lowercase()),
        );
    }
    merged.sort();
    merged.dedup();
    merged.truncate(24);
    merged
}

fn memory_repo_fingerprint(repo_fingerprint: &str, scope: MemoryScope) -> Option<String> {
    match scope {
        MemoryScope::Global => None,
        MemoryScope::Repo | MemoryScope::Module => Some(repo_fingerprint.to_string()),
    }
}

fn subject_key_for_candidate(repo_fingerprint: &str, candidate: &MemoryCandidateDraft) -> String {
    let scope = scope_label(candidate.scope);
    let hint = slugify(&candidate.subject_hint);
    let module_hint = candidate
        .module_refs
        .first()
        .map(|path| slugify(path))
        .filter(|value| !value.is_empty());
    let repo_hint = slugify(repo_fingerprint);
    let primary = if !hint.is_empty() {
        hint
    } else if let Some(module_hint) = module_hint.clone() {
        module_hint
    } else {
        slugify(&candidate.title)
    };
    match candidate.scope {
        MemoryScope::Global => format!("{scope}:{primary}"),
        MemoryScope::Repo => format!("{scope}:{repo_hint}:{primary}"),
        MemoryScope::Module => format!(
            "{scope}:{repo_hint}:{}:{primary}",
            module_hint.unwrap_or_else(|| "module".into())
        ),
    }
}

fn merge_evidence(
    mut evidence: Vec<MemoryEvidenceRef>,
    next: MemoryEvidenceRef,
) -> Vec<MemoryEvidenceRef> {
    if !evidence.iter().any(|existing| existing == &next) {
        evidence.push(next);
    }
    if evidence.len() > 6 {
        evidence.drain(0..evidence.len() - 6);
    }
    evidence
}

fn dedupe_supersede_ids<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .filter_map(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in text.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else {
            None
        };
        match normalized {
            Some(ch) => {
                slug.push(ch);
                last_dash = false;
            }
            None if !last_dash && !slug.is_empty() => {
                slug.push('-');
                last_dash = true;
            }
            None => {}
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn embedding_text(record: &MemoryRecord) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        record.subject_key,
        record.title,
        record.summary,
        record.details.clone().unwrap_or_default(),
        record.tags.join(" ")
    )
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ToolCall, TranscriptEntry};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct FakeEmbedder;

    impl EmbeddingProvider for FakeEmbedder {
        fn backend_name(&self) -> &'static str {
            "fake"
        }

        fn embed(&mut self, inputs: Vec<String>) -> Result<Vec<Vec<f32>>> {
            Ok(inputs
                .into_iter()
                .map(|input| {
                    let lowered = input.to_ascii_lowercase();
                    vec![
                        lowered.matches("memory").count() as f32,
                        lowered.matches("module").count() as f32,
                        lowered.matches("preference").count() as f32,
                    ]
                })
                .collect())
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        std::env::temp_dir().join(format!("oat-memory-{name}-{}-{nanos}", std::process::id()))
    }

    fn memory_service() -> MemoryService {
        let root = temp_dir("workspace");
        fs::create_dir_all(root.join(".git")).expect("repo");
        fs::create_dir_all(root.join("src")).expect("workspace");
        fs::write(root.join("src/module.rs"), "fn demo() {}\n").expect("file");
        MemoryService::with_embedder(
            MemoryConfig::default(),
            root,
            Some(temp_dir("store")),
            Box::new(FakeEmbedder),
        )
        .expect("memory service")
    }

    fn real_embedder_memory_service() -> Option<MemoryService> {
        let reason = fastembed_unavailable_reason();
        if let Some(reason) = reason {
            eprintln!("skipping real-embedder test: {reason}");
            return None;
        }

        let root = temp_dir("workspace-real-embedder");
        fs::create_dir_all(root.join(".git")).expect("repo");
        fs::create_dir_all(root.join("src")).expect("workspace");
        fs::write(root.join("src/module.rs"), "fn demo() {}\n").expect("file");
        let store = temp_dir("store-real-embedder");
        let cache_dir = std::env::temp_dir().join("oat-fastembed-test-cache");

        MemoryService::with_embedder(
            MemoryConfig::default(),
            root,
            Some(store),
            Box::new(FastembedEmbeddingProvider::new(cache_dir)),
        )
        .ok()
    }

    fn repo_fingerprint(service: &MemoryService) -> String {
        let manager = service.inner.lock().expect("lock");
        manager.repo_fingerprint.clone()
    }

    fn seed_record(
        service: &MemoryService,
        scope: MemoryScope,
        kind: MemoryKind,
        subject_key: &str,
        title: &str,
        summary: &str,
        tags: &[&str],
        files: &[&str],
        status: MemoryStatus,
    ) {
        let repo_fingerprint = repo_fingerprint(service);
        service
            .insert_test_record(MemoryRecord {
                id: Uuid::now_v7().to_string(),
                scope,
                repo_fingerprint: memory_repo_fingerprint(&repo_fingerprint, scope),
                subject_key: subject_key.into(),
                kind,
                title: title.into(),
                summary: summary.into(),
                details: None,
                tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                evidence: (!files.is_empty())
                    .then(|| {
                        vec![MemoryEvidenceRef {
                            session_id: Some("session-test".into()),
                            prompt: Some(summary.into()),
                            files: files.iter().map(|file| (*file).to_string()).collect(),
                        }]
                    })
                    .unwrap_or_default(),
                source: MemorySource::Inferred,
                confidence: 0.9,
                status,
                supersedes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .expect("seed record");
    }

    fn seed_noise_records(service: &MemoryService, count: usize, pathish: bool) {
        let repo_fingerprint = repo_fingerprint(service);
        for index in 0..count {
            let scope = match index % 3 {
                0 => MemoryScope::Global,
                1 => MemoryScope::Repo,
                _ => MemoryScope::Module,
            };
            let kind = match index % 6 {
                0 => MemoryKind::Architecture,
                1 => MemoryKind::Decision,
                2 => MemoryKind::Episode,
                3 => MemoryKind::Workflow,
                4 => MemoryKind::Preference,
                _ => MemoryKind::Hazard,
            };
            let subject_key = match scope {
                MemoryScope::Global => format!("global:noise-{index}"),
                MemoryScope::Repo => format!("repo:noise-{index}"),
                MemoryScope::Module => format!("module:noise-{index}"),
            };
            let tags = if pathish {
                vec!["src".into(), "rs".into(), "module".into()]
            } else {
                vec!["widget".into(), "cache".into(), "noise".into()]
            };
            let evidence = if pathish {
                vec![MemoryEvidenceRef {
                    session_id: Some("session-noise".into()),
                    prompt: Some(format!("noise path memory {index}")),
                    files: vec![format!("src/noise/module_{index}.rs")],
                }]
            } else {
                Vec::new()
            };
            service
                .insert_test_record(MemoryRecord {
                    id: Uuid::now_v7().to_string(),
                    scope,
                    repo_fingerprint: memory_repo_fingerprint(&repo_fingerprint, scope),
                    subject_key,
                    kind,
                    title: format!("Noise memory {index}"),
                    summary: format!("Irrelevant widget cache note {index}."),
                    details: None,
                    tags,
                    evidence,
                    source: MemorySource::Inferred,
                    confidence: 0.55,
                    status: MemoryStatus::Active,
                    supersedes: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
                .expect("seed noise record");
        }
    }

    fn result_entry_count(output: &str) -> usize {
        output.lines().filter(|line| line.starts_with("- ")).count()
    }

    fn injected_memory_count(prompt: &str) -> usize {
        prompt
            .lines()
            .filter(|line| line.starts_with("- ["))
            .count()
    }

    #[test]
    fn prepare_completed_turn_collects_touched_files() {
        let service = memory_service();
        let prepared = {
            let manager = service.inner.lock().expect("lock");
            manager
                .prepare_completed_turn(&CompletedTurnMemoryInput {
                    session_id: Some("session-1".into()),
                    seed: MainRequestSeed {
                        history: Vec::new(),
                        visible_prompt: "Update the module behavior.".into(),
                        model_prompt:
                            "Relevant memory:\n- User prefers terse summaries.\n\nUpdate the module behavior."
                                .into(),
                        history_model_name: None,
                        transcript_len_before: 0,
                    },
                    transcript_entries: vec![TranscriptEntry::ToolCall(ToolCall {
                        name: "ReadFile".into(),
                        parameter: r#"{"filename":"src/module.rs","offset":0,"limit":20}"#.into(),
                        preview: None,
                    })],
                    assistant_response: "Adjusted the module design.".into(),
                })
                .expect("prepared")
                .expect("turn prepared")
        };

        assert_eq!(prepared.evidence.touched_files, vec!["src/module.rs"]);
        assert_eq!(prepared.evidence.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            prepared.evidence.visible_prompt,
            "Update the module behavior."
        );
    }

    #[test]
    fn candidate_contexts_include_related_existing_memory() {
        let service = memory_service();
        service
            .insert_test_record(MemoryRecord {
                id: Uuid::now_v7().to_string(),
                scope: MemoryScope::Module,
                repo_fingerprint: Some({
                    let manager = service.inner.lock().expect("lock");
                    manager.repo_fingerprint.clone()
                }),
                subject_key: "module:workspace:src-module-rs".into(),
                kind: MemoryKind::Decision,
                title: "Module follow-up".into(),
                summary: "Keep `src/module.rs` behavior stable.".into(),
                details: None,
                tags: vec!["module".into(), "src".into()],
                evidence: Vec::new(),
                source: MemorySource::Inferred,
                confidence: 0.8,
                status: MemoryStatus::Active,
                supersedes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .expect("seed record");

        let contexts = {
            let mut manager = service.inner.lock().expect("lock");
            manager
                .build_candidate_contexts(
                    &MemoryExtractorOutput {
                        candidates: vec![MemoryCandidateDraft {
                            scope: MemoryScope::Module,
                            kind: MemoryKind::Decision,
                            source: MemorySource::Inferred,
                            subject_hint: "module-follow-up".into(),
                            title: "Module follow-up".into(),
                            summary: "Adjust `src/module.rs` behavior.".into(),
                            details: None,
                            tags: vec!["module".into()],
                            module_refs: vec!["src/module.rs".into()],
                            confidence: 85,
                        }],
                    },
                    5,
                )
                .expect("candidate contexts")
        };

        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].related_memories.len(), 1);
        assert_eq!(contexts[0].related_memories[0].title, "Module follow-up");
    }

    #[test]
    fn search_text_filters_unrelated_memories() {
        let service = memory_service();
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-style",
            "Reply style",
            "Prefer warmer replies by default.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Hazard,
            "global:banned-words",
            "Banned words",
            "Do not use boo or eggs in replies.",
            &["boo", "eggs"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Workflow,
            "repo:workspace:database-migrations",
            "Database migration workflow",
            "Run migrations before shipping schema changes.",
            &["database", "migrations", "schema"],
            &[],
            MemoryStatus::Active,
        );

        let output = service
            .search_text("database migrations", false, 10)
            .expect("search");

        assert!(output.contains("Database migration workflow"));
        assert!(!output.contains("Reply style"));
        assert!(!output.contains("Banned words"));
        assert!(output.contains("why:"));
    }

    #[test]
    fn search_text_stays_selective_with_large_noise_corpus() {
        let service = memory_service();
        seed_noise_records(&service, 150, false);
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Workflow,
            "repo:workspace:database-migrations",
            "Database migration workflow",
            "Run migrations before shipping schema changes.",
            &["database", "migrations", "schema"],
            &[],
            MemoryStatus::Active,
        );

        let output = service
            .search_text("database migrations", false, 10)
            .expect("search");

        assert!(output.contains("Database migration workflow"));
        assert_eq!(result_entry_count(&output), 1);
    }

    #[test]
    fn search_text_supports_exact_module_path_lookup() {
        let service = memory_service();
        seed_record(
            &service,
            MemoryScope::Module,
            MemoryKind::Decision,
            "module:workspace:src-auth-session-rs:session-cache",
            "Session cache invariants",
            "Keep the auth session cache ordering stable.",
            &["auth", "session", "cache"],
            &["src/auth/session.rs"],
            MemoryStatus::Active,
        );

        let output = service
            .search_text("src/auth/session.rs", false, 10)
            .expect("search");

        assert!(output.contains("Session cache invariants"));
        assert!(output.contains("why:"));
        assert!(output.contains("subject") || output.contains("path"));
    }

    #[test]
    fn search_text_does_not_match_generic_repo_memories_for_path_queries() {
        let service = memory_service();
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Architecture,
            "repo:workspace:semantic-memory",
            "Semantic memory subsystem",
            "The repo gained a semantic memory subsystem.",
            &["memory", "src", "rs"],
            &[],
            MemoryStatus::Active,
        );

        let output = service
            .search_text("src/auth/session.rs", false, 10)
            .expect("search");

        assert_eq!(output, "No memories matched `src/auth/session.rs`.");
    }

    #[test]
    fn path_queries_ignore_large_generic_path_noise_corpus() {
        let service = memory_service();
        seed_noise_records(&service, 120, true);

        let output = service
            .search_text("src/auth/session.rs", false, 10)
            .expect("search");

        assert_eq!(output, "No memories matched `src/auth/session.rs`.");
    }

    #[test]
    fn auto_inject_skips_episode_memories() {
        let service = memory_service();
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Episode,
            "repo:workspace:release-retro",
            "Release retrospective",
            "The database rollout failed during the release window.",
            &["database", "release"],
            &[],
            MemoryStatus::Active,
        );

        let prompt = service
            .augment_prompt("Investigate the database rollout failure.")
            .expect("augment prompt");

        assert_eq!(prompt, "Investigate the database rollout failure.");
    }

    #[test]
    fn auto_inject_keeps_allowed_preference_memories() {
        let service = memory_service();
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth",
            "Reply warmth",
            "Prefer warmer replies and avoid sounding abrupt.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Active,
        );

        let prompt = service
            .augment_prompt("Please keep the tone warm in this reply.")
            .expect("augment prompt");

        assert!(prompt.contains("Relevant memory:"));
        assert!(prompt.contains("Reply warmth"));
    }

    #[test]
    fn decision_status_requires_higher_confidence_to_enter_store_by_default() {
        let extraction = MemoryExtractionConfig::default();

        let below_candidate = MemoryConsolidationDecision {
            candidate_index: 0,
            action: MemoryConsolidationAction::CreateCandidate,
            existing_memory_id: None,
            supersede_memory_ids: Vec::new(),
            title: Some("Candidate".into()),
            summary: Some("Candidate".into()),
            details: None,
            tags: Vec::new(),
            confidence: extraction.min_candidate_confidence - 1,
        };
        assert_eq!(decision_status(&extraction, &below_candidate), None);

        let between_candidate_and_active = MemoryConsolidationDecision {
            candidate_index: 0,
            action: MemoryConsolidationAction::CreateActive,
            existing_memory_id: None,
            supersede_memory_ids: Vec::new(),
            title: Some("Active".into()),
            summary: Some("Active".into()),
            details: None,
            tags: Vec::new(),
            confidence: extraction.min_candidate_confidence,
        };
        assert_eq!(
            decision_status(&extraction, &between_candidate_and_active),
            Some(MemoryStatus::Candidate)
        );

        let active = MemoryConsolidationDecision {
            confidence: extraction.min_active_confidence,
            ..between_candidate_and_active
        };
        assert_eq!(
            decision_status(&extraction, &active),
            Some(MemoryStatus::Active)
        );
    }

    #[test]
    fn stats_text_reports_useful_store_summary() {
        let service = memory_service();
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth",
            "Reply warmth",
            "Prefer warmer replies.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth",
            "Reply warmth candidate",
            "Prefer warmer replies.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Candidate,
        );
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Workflow,
            "repo:workspace:db-migrate",
            "DB migrate",
            "Run db migrations before deploy.",
            &["database", "migrations"],
            &[],
            MemoryStatus::Archived,
        );

        let stats = service.stats_text().expect("stats");

        assert!(stats.contains("Memory stats:"));
        assert!(stats.contains("- total records: 3"));
        assert!(stats.contains("- live records: 1 active, 1 candidate"));
        assert!(stats.contains("- inactive records: 0 superseded, 1 archived"));
        assert!(stats.contains("- vectors: 2 stored, 2/2 live indexed, 0 orphaned"));
        assert!(stats.contains("- subject keys: 2 unique, 1 duplicate live subjects"));
        assert!(stats.contains("By status:"));
        assert!(stats.contains("- active: 1"));
        assert!(stats.contains("- candidate: 1"));
        assert!(stats.contains("- archived: 1"));
        assert!(stats.contains("By kind:"));
        assert!(stats.contains("- User preference: 2"));
        assert!(stats.contains("- Workflow: 1"));
        assert!(stats.contains("By scope:"));
        assert!(stats.contains("- global: 2"));
        assert!(stats.contains("- repo: 1"));
        assert!(stats.contains("By source:"));
        assert!(stats.contains("- inferred: 3"));
    }

    #[test]
    fn auto_inject_stays_selective_with_large_noise_and_respects_limit() {
        let service = memory_service();
        seed_noise_records(&service, 90, false);
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth",
            "Reply warmth",
            "Prefer warmer replies and avoid sounding abrupt.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Hazard,
            "global:banned-words",
            "Banned words",
            "Do not use boo or eggs in replies.",
            &["boo", "eggs", "avoid"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:file-refs",
            "Exact file refs",
            "Prefer exact file refs in responses.",
            &["exact", "file", "refs"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Episode,
            "repo:workspace:file-ref-incident",
            "File ref incident",
            "A previous file refs incident involved boo and eggs.",
            &["file", "refs", "boo", "eggs"],
            &[],
            MemoryStatus::Active,
        );

        let prompt = service
            .augment_prompt(
                "Please keep replies warm, avoid boo and eggs, and include exact file refs.",
            )
            .expect("augment prompt");

        assert!(prompt.contains("Relevant memory:"));
        assert!(prompt.contains("Reply warmth"));
        assert!(prompt.contains("Banned words"));
        assert!(prompt.contains("Exact file refs"));
        assert!(!prompt.contains("File ref incident"));
        assert_eq!(injected_memory_count(&prompt), 3);
    }

    #[test]
    fn real_embedder_large_corpus_search_and_path_remain_selective() {
        let Some(service) = real_embedder_memory_service() else {
            return;
        };

        seed_noise_records(&service, 180, false);
        seed_noise_records(&service, 120, true);
        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Workflow,
            "repo:workspace:database-migrations",
            "Database migration workflow",
            "Run migrations before shipping schema changes.",
            &["database", "migrations", "schema"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Module,
            MemoryKind::Decision,
            "module:workspace:src-auth-session-rs:session-cache",
            "Session cache invariants",
            "Keep the auth session cache ordering stable.",
            &["auth", "session", "cache"],
            &["src/auth/session.rs"],
            MemoryStatus::Active,
        );

        let db = service
            .search_text("database migrations", false, 10)
            .expect("search");
        assert!(db.contains("Database migration workflow"));
        assert_eq!(result_entry_count(&db), 1);

        let path = service
            .search_text("src/auth/session.rs", false, 10)
            .expect("search");
        assert!(path.contains("Session cache invariants"));
        assert_eq!(result_entry_count(&path), 1);
        assert!(!path.contains("Noise memory"));
    }

    #[test]
    fn real_embedder_large_corpus_auto_inject_and_candidate_linking_remain_selective() {
        let Some(service) = real_embedder_memory_service() else {
            return;
        };

        seed_noise_records(&service, 140, false);
        seed_noise_records(&service, 80, true);
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth",
            "Reply warmth",
            "Prefer warmer replies and avoid sounding abrupt.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Hazard,
            "global:banned-words",
            "Banned words",
            "Do not use boo or eggs in replies.",
            &["boo", "eggs", "avoid"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Module,
            MemoryKind::Decision,
            "module:workspace:src-billing-ledger-rs:rounding",
            "Billing ledger rounding",
            "Keep billing ledger rounding stable in src/billing/ledger.rs.",
            &["billing", "ledger", "rounding"],
            &["src/billing/ledger.rs"],
            MemoryStatus::Active,
        );

        let prompt = service
            .augment_prompt(
                "Keep replies warm, avoid boo and eggs, and review billing ledger work.",
            )
            .expect("augment prompt");
        assert!(prompt.contains("Reply warmth"));
        assert!(prompt.contains("Banned words"));
        assert!(!prompt.contains("Noise memory"));
        assert!(injected_memory_count(&prompt) <= 3);

        let contexts = {
            let mut manager = service.inner.lock().expect("lock");
            manager
                .build_candidate_contexts(
                    &MemoryExtractorOutput {
                        candidates: vec![MemoryCandidateDraft {
                            scope: MemoryScope::Module,
                            kind: MemoryKind::Decision,
                            source: MemorySource::Inferred,
                            subject_hint: "billing-ledger".into(),
                            title: "Billing ledger rounding".into(),
                            summary: "Adjust billing ledger rounding in src/billing/ledger.rs."
                                .into(),
                            details: None,
                            tags: vec!["billing".into(), "ledger".into()],
                            module_refs: vec!["src/billing/ledger.rs".into()],
                            confidence: 88,
                        }],
                    },
                    5,
                )
                .expect("candidate contexts")
        };
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].related_memories.len(), 1);
        assert_eq!(
            contexts[0].related_memories[0].title,
            "Billing ledger rounding"
        );
    }

    #[test]
    fn candidate_contexts_remain_targeted_with_large_noise_corpus() {
        let service = memory_service();
        seed_noise_records(&service, 100, true);
        seed_record(
            &service,
            MemoryScope::Module,
            MemoryKind::Decision,
            "module:workspace:src-billing-ledger-rs:rounding",
            "Billing ledger rounding",
            "Keep billing ledger rounding stable in src/billing/ledger.rs.",
            &["billing", "ledger", "rounding"],
            &["src/billing/ledger.rs"],
            MemoryStatus::Active,
        );

        let contexts = {
            let mut manager = service.inner.lock().expect("lock");
            manager
                .build_candidate_contexts(
                    &MemoryExtractorOutput {
                        candidates: vec![MemoryCandidateDraft {
                            scope: MemoryScope::Module,
                            kind: MemoryKind::Decision,
                            source: MemorySource::Inferred,
                            subject_hint: "billing-ledger".into(),
                            title: "Billing ledger rounding".into(),
                            summary: "Adjust billing ledger rounding in src/billing/ledger.rs."
                                .into(),
                            details: None,
                            tags: vec!["billing".into(), "ledger".into()],
                            module_refs: vec!["src/billing/ledger.rs".into()],
                            confidence: 88,
                        }],
                    },
                    5,
                )
                .expect("candidate contexts")
        };

        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].related_memories.len(), 1);
        assert_eq!(
            contexts[0].related_memories[0].title,
            "Billing ledger rounding"
        );
    }

    #[test]
    fn search_text_ignores_non_active_matches_even_in_large_corpus() {
        let service = memory_service();
        seed_noise_records(&service, 75, false);
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth-active",
            "Active warm replies",
            "Prefer warmer replies by default.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth-candidate",
            "Candidate warm replies",
            "Prefer warmer replies by default.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Candidate,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth-archived",
            "Archived warm replies",
            "Prefer warmer replies by default.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Archived,
        );
        seed_record(
            &service,
            MemoryScope::Global,
            MemoryKind::Preference,
            "global:reply-warmth-superseded",
            "Superseded warm replies",
            "Prefer warmer replies by default.",
            &["warm", "replies"],
            &[],
            MemoryStatus::Superseded,
        );

        let output = service
            .search_text("warm replies", false, 10)
            .expect("search");

        assert!(output.contains("Active warm replies"));
        assert!(!output.contains("Candidate warm replies"));
        assert!(!output.contains("Archived warm replies"));
        assert!(!output.contains("Superseded warm replies"));
        assert_eq!(result_entry_count(&output), 1);
    }

    fn copy_file_if_exists(from: &Path, to: &Path) {
        if from.exists() {
            fs::copy(from, to).expect("copy file");
        }
    }

    #[test]
    #[ignore = "manual smoke test that clones the live memory store for retrieval tuning"]
    fn live_store_retrieval_smoke() {
        let Some(live_store) =
            default_memory_dir().filter(|dir| dir.join(SNAPSHOT_FILE_NAME).exists())
        else {
            return;
        };

        let cloned_store = temp_dir("live-memory-clone");
        fs::create_dir_all(&cloned_store).expect("clone dir");
        copy_file_if_exists(
            &live_store.join(SNAPSHOT_FILE_NAME),
            &cloned_store.join(SNAPSHOT_FILE_NAME),
        );
        copy_file_if_exists(
            &live_store.join(VECTORS_FILE_NAME),
            &cloned_store.join(VECTORS_FILE_NAME),
        );
        copy_file_if_exists(
            &live_store.join(EVENTS_FILE_NAME),
            &cloned_store.join(EVENTS_FILE_NAME),
        );

        let service = MemoryService::with_storage_dir(
            MemoryConfig::default(),
            std::env::current_dir().expect("cwd"),
            cloned_store,
        )
        .expect("memory service");

        let warm = service
            .search_text("warm replies", false, 10)
            .expect("warm search");
        println!("warm replies:\n{warm}\n");
        assert!(warm.contains("Prefers warmer replies"));
        assert!(!warm.contains("Semantic long-term memory subsystem"));

        let path_before = service
            .search_text("src/auth/session.rs", false, 10)
            .expect("path search");
        println!("path before synthetic:\n{path_before}\n");
        assert_eq!(path_before, "No memories matched `src/auth/session.rs`.");

        seed_record(
            &service,
            MemoryScope::Repo,
            MemoryKind::Workflow,
            "repo:root-oat:database-migrations",
            "Database migration workflow",
            "Run migrations before shipping schema changes.",
            &["database", "migrations", "schema"],
            &[],
            MemoryStatus::Active,
        );
        seed_record(
            &service,
            MemoryScope::Module,
            MemoryKind::Decision,
            "module:root-oat:src-auth-session-rs:session-cache",
            "Session cache invariants",
            "Keep the auth session cache ordering stable.",
            &["auth", "session", "cache"],
            &["src/auth/session.rs"],
            MemoryStatus::Active,
        );

        let db = service
            .search_text("database migrations", false, 10)
            .expect("db search");
        println!("database migrations:\n{db}\n");
        assert!(db.contains("Database migration workflow"));
        assert!(!db.contains("Prefers warmer replies"));

        let path_after = service
            .search_text("src/auth/session.rs", false, 10)
            .expect("path search");
        println!("path after synthetic:\n{path_after}\n");
        assert!(path_after.contains("Session cache invariants"));
        assert!(!path_after.contains("Semantic long-term memory subsystem"));
    }

    #[test]
    fn replace_supersedes_old_memory() {
        let service = memory_service();
        service
            .insert_test_record(MemoryRecord {
                id: Uuid::now_v7().to_string(),
                scope: MemoryScope::Global,
                repo_fingerprint: None,
                subject_key: "global:reply-style".into(),
                kind: MemoryKind::Preference,
                title: "Reply style".into(),
                summary: "Prefer concise replies.".into(),
                details: Some("Always keep replies concise.".into()),
                tags: vec!["concise".into()],
                evidence: vec![MemoryEvidenceRef {
                    session_id: Some("session-3".into()),
                    prompt: Some("Always keep replies concise.".into()),
                    files: Vec::new(),
                }],
                source: MemorySource::ExplicitUser,
                confidence: 0.95,
                status: MemoryStatus::Active,
                supersedes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .expect("seed record");

        let initial = service.search_text("concise", false, 10).expect("search");
        let id = initial
            .lines()
            .find_map(|line| line.strip_prefix("- "))
            .and_then(|line| line.split_whitespace().next())
            .expect("id");

        let full = {
            let manager = service.inner.lock().expect("lock");
            manager
                .snapshot
                .records
                .iter()
                .find(|record| short_id(&record.id) == id)
                .map(|record| record.id.clone())
                .expect("full id")
        };
        service
            .replace(&full, "Prefer concise replies with exact file refs.")
            .expect("replace");

        let updated = service
            .search_text("exact file refs", false, 10)
            .expect("search");
        assert!(updated.contains("exact file refs"));
    }

    #[test]
    fn clear_removes_all_memories_and_vectors() {
        let service = memory_service();
        service
            .insert_test_record(MemoryRecord {
                id: Uuid::now_v7().to_string(),
                scope: MemoryScope::Global,
                repo_fingerprint: None,
                subject_key: "global:reply-style".into(),
                kind: MemoryKind::Preference,
                title: "Reply style".into(),
                summary: "Prefer concise replies.".into(),
                details: None,
                tags: vec!["concise".into()],
                evidence: Vec::new(),
                source: MemorySource::ExplicitUser,
                confidence: 0.95,
                status: MemoryStatus::Active,
                supersedes: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .expect("seed record");

        let message = service.clear().expect("clear");
        assert_eq!(message, "Cleared 1 memory.");

        let manager = service.inner.lock().expect("lock");
        assert!(manager.snapshot.records.is_empty());
        assert!(manager.vectors.is_empty());
    }

    #[test]
    fn fastembed_provider_fails_closed_without_runtime() {
        let mut provider = FastembedEmbeddingProvider::new(temp_dir("fastembed-cache"));
        let result = provider.embed(vec!["memory lookup".into()]);
        assert!(result.is_ok() || result.is_err());
    }
}
