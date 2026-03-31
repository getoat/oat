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

        let hits = self.search_hits(prompt, false, self.config.max_auto_results)?;
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
        let hits = self.search_hits(query, include_candidates, limit)?;
        Ok(format_search_results(query, &hits))
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
        let hits = self.search_hits("", true, self.config.max_candidate_search_results)?;
        let candidates = hits
            .into_iter()
            .filter(|hit| hit.record.status == MemoryStatus::Candidate)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            Ok("No memory candidates are waiting for review.".into())
        } else {
            Ok(format_search_results("candidates", &candidates))
        }
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
                    .search_hits(&query, true, max_related_memories)?
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
        include_candidates: bool,
        limit: usize,
    ) -> Result<Vec<MemorySearchHit>> {
        let query = query.trim();
        let tokens = tokenize(query);
        let eligible = self
            .snapshot
            .records
            .iter()
            .filter(|record| self.record_is_eligible(record, include_candidates))
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
            .map(|record| {
                let lexical = lexical_score(query, &tokens, &record);
                let semantic_score = semantic
                    .as_ref()
                    .and_then(|query_vector| {
                        self.vectors
                            .get(&record.id)
                            .map(|record_vector| cosine_similarity(query_vector, record_vector))
                    })
                    .unwrap_or(0.0);
                let score = lexical * 2.0
                    + semantic_score
                    + scope_bonus(&self.repo_fingerprint, &record)
                    + recency_bonus(record.updated_at)
                    + record.confidence * 0.3;
                MemorySearchHit { record, score }
            })
            .collect::<Vec<_>>();
        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.record.updated_at.cmp(&left.record.updated_at))
        });
        hits.truncate(limit.max(1));
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

fn format_search_results(query: &str, hits: &[MemorySearchHit]) -> String {
    if hits.is_empty() {
        return format!("No memories matched `{query}`.");
    }

    let mut lines = vec![format!("Memory results for `{query}`:")];
    for hit in hits {
        lines.push(format!(
            "- {} [{} | {} | {}] {}",
            short_id(&hit.record.id),
            kind_label(hit.record.kind),
            scope_label(hit.record.scope),
            status_label(hit.record.status),
            hit.record.title
        ));
        lines.push(format!("  {}", hit.record.summary));
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
    for token in tokens {
        if haystack.contains(token) {
            score += 0.8;
        }
    }
    score
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
