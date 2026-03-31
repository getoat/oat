use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::{
    config::{AppConfig, MemoryExtractionConfig},
    llm::run_internal_plain_prompt,
    stats::StatsHook,
};

use super::{MemoryKind, MemoryScope, MemorySource};

const EXTRACTOR_PREAMBLE: &str = concat!(
    "You extract long-term memories for Oat.\n",
    "Work only from the provided evidence.\n",
    "Return JSON only.\n",
    "Do not wrap the JSON in markdown fences.\n",
);

const CONSOLIDATOR_PREAMBLE: &str = concat!(
    "You consolidate extracted memories into Oat's long-term memory store.\n",
    "Work only from the provided evidence, candidates, and related existing memories.\n",
    "Return JSON only.\n",
    "Do not wrap the JSON in markdown fences.\n",
);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MemoryTurnEvidence {
    pub session_id: Option<String>,
    pub repo_fingerprint: String,
    pub visible_prompt: String,
    pub assistant_response: String,
    pub touched_files: Vec<String>,
    pub transcript: Vec<MemoryTranscriptEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MemoryTranscriptEvidence {
    pub kind: String,
    pub label: Option<String>,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MemoryCandidateDraft {
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub source: MemorySource,
    pub subject_hint: String,
    pub title: String,
    pub summary: String,
    pub details: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub module_refs: Vec<String>,
    pub confidence: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct MemoryExtractorOutput {
    #[serde(default)]
    pub candidates: Vec<MemoryCandidateDraft>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct RelatedMemorySummary {
    pub id: String,
    pub scope: MemoryScope,
    pub kind: MemoryKind,
    pub source: MemorySource,
    pub subject_key: String,
    pub title: String,
    pub summary: String,
    pub details: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub confidence: f32,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MemoryCandidateContext {
    pub candidate_index: usize,
    pub candidate: MemoryCandidateDraft,
    #[serde(default)]
    pub related_memories: Vec<RelatedMemorySummary>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum MemoryConsolidationAction {
    CreateActive,
    CreateCandidate,
    UpdateExisting,
    SupersedeExisting,
    Ignore,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct MemoryConsolidationDecision {
    pub candidate_index: usize,
    pub action: MemoryConsolidationAction,
    pub existing_memory_id: Option<String>,
    #[serde(default)]
    pub supersede_memory_ids: Vec<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub details: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub confidence: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(super) struct MemoryConsolidationOutput {
    #[serde(default)]
    pub decisions: Vec<MemoryConsolidationDecision>,
}

pub(super) async fn extract_candidates(
    app_config: &AppConfig,
    extraction: &MemoryExtractionConfig,
    evidence: &MemoryTurnEvidence,
    stats_hook: StatsHook,
) -> Result<MemoryExtractorOutput> {
    let prompt = format!(
        concat!(
            "Extract durable memories from this completed turn.\n",
            "Return at most {max_candidates} candidates.\n\n",
            "Only keep information that is likely to matter in future interactions.\n",
            "Ignore ephemeral progress updates, one-off wording, and temporary tool noise.\n",
            "Use `global` for user-wide preferences or facts that apply across repos.\n",
            "Use `repo` for repository-specific practices, architecture, decisions, or hazards.\n",
            "Use `module` only when the evidence clearly points to a specific module, path, or subsystem.\n",
            "`subject_hint` must be short, stable, and reusable across future updates.\n",
            "`summary` must be concise and factual.\n",
            "`confidence` must be an integer from 0 to 100.\n\n",
            "JSON schema:\n",
            "{{\n",
            "  \"candidates\": [\n",
            "    {{\n",
            "      \"scope\": \"global|repo|module\",\n",
            "      \"kind\": \"preference|workflow|architecture|decision|hazard|episode\",\n",
            "      \"source\": \"explicit_user|inferred|auto_summary\",\n",
            "      \"subject_hint\": \"short-stable-key\",\n",
            "      \"title\": \"short title\",\n",
            "      \"summary\": \"concise factual summary\",\n",
            "      \"details\": null,\n",
            "      \"tags\": [\"tag\"],\n",
            "      \"module_refs\": [\"src/module.rs\"],\n",
            "      \"confidence\": 0\n",
            "    }}\n",
            "  ]\n",
            "}}\n\n",
            "Evidence JSON:\n{evidence_json}\n"
        ),
        max_candidates = extraction.max_candidates_per_turn,
        evidence_json = serde_json::to_string_pretty(evidence)?,
    );
    let raw = run_internal_plain_prompt(
        app_config,
        &extraction.model_name,
        EXTRACTOR_PREAMBLE,
        extraction.reasoning,
        prompt,
        stats_hook,
    )
    .await?;
    let mut output: MemoryExtractorOutput =
        parse_json_payload(&raw).context("failed to parse memory extractor output as JSON")?;
    output
        .candidates
        .truncate(extraction.max_candidates_per_turn.max(1));
    Ok(output)
}

pub(super) async fn consolidate_candidates(
    app_config: &AppConfig,
    extraction: &MemoryExtractionConfig,
    evidence: &MemoryTurnEvidence,
    candidates: &[MemoryCandidateContext],
    stats_hook: StatsHook,
) -> Result<MemoryConsolidationOutput> {
    let prompt = format!(
        concat!(
            "For each extracted candidate, choose exactly one consolidation action.\n",
            "Prefer `update_existing` when the candidate refines an existing memory with the same stable subject.\n",
            "Use `supersede_existing` when an old memory is now meaningfully outdated or replaced.\n",
            "Use `create_active` for durable memories worth surfacing automatically.\n",
            "Use `create_candidate` for useful but not yet stable memories.\n",
            "Use `ignore` for duplicates, low-signal items, or unsupported inferences.\n",
            "If the action is `update_existing` or `supersede_existing`, `existing_memory_id` must point to one of the related memories.\n",
            "Use `supersede_memory_ids` for any additional related memories that should be retired because they are duplicates, contradictions, or outdated versions.\n",
            "When not ignoring a candidate, provide the final `title`, `summary`, optional `details`, `tags`, and an integer `confidence` from 0 to 100.\n\n",
            "JSON schema:\n",
            "{{\n",
            "  \"decisions\": [\n",
            "    {{\n",
            "      \"candidate_index\": 0,\n",
            "      \"action\": \"create_active|create_candidate|update_existing|supersede_existing|ignore\",\n",
            "      \"existing_memory_id\": null,\n",
            "      \"supersede_memory_ids\": [],\n",
            "      \"title\": null,\n",
            "      \"summary\": null,\n",
            "      \"details\": null,\n",
            "      \"tags\": [\"tag\"],\n",
            "      \"confidence\": 0\n",
            "    }}\n",
            "  ]\n",
            "}}\n\n",
            "Evidence JSON:\n{evidence_json}\n\n",
            "Candidate context JSON:\n{candidate_json}\n"
        ),
        evidence_json = serde_json::to_string_pretty(evidence)?,
        candidate_json = serde_json::to_string_pretty(candidates)?,
    );
    let raw = run_internal_plain_prompt(
        app_config,
        &extraction.model_name,
        CONSOLIDATOR_PREAMBLE,
        extraction.reasoning,
        prompt,
        stats_hook,
    )
    .await?;
    parse_json_payload(&raw).context("failed to parse memory consolidator output as JSON")
}

fn parse_json_payload<T>(raw: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(raw)
        .or_else(|_| {
            let start = raw
                .find('{')
                .ok_or_else(|| anyhow!("missing JSON object"))?;
            let end = raw
                .rfind('}')
                .ok_or_else(|| anyhow!("missing JSON object"))?;
            serde_json::from_str(&raw[start..=end]).map_err(anyhow::Error::from)
        })
        .with_context(|| format!("raw model output was: {}", raw.trim()))
}
