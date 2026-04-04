use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use regex::Regex;
use reqwest::{
    StatusCode, Url,
    header::{CONTENT_TYPE, HeaderValue},
};
use scraper::{ElementRef, Html, Selector};

use crate::token_counting::TokenCounter;

const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const DEFAULT_PAGE_CACHE_MAX_ENTRIES: usize = 12;
const DEFAULT_SEARCH_CACHE_MAX_ENTRIES: usize = 8;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_OPEN_METADATA_TOKEN_RESERVE: usize = 512;
const DEFAULT_FIND_MATCH_LIMIT: usize = 20;
const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36 oat-web-run/1.0";

#[derive(Clone)]
pub(crate) struct WebService {
    inner: Arc<WebServiceInner>,
}

struct WebServiceInner {
    client: reqwest::Client,
    tokenizer: TokenCounter,
    config: WebServiceConfig,
    open_chunk_tokens: usize,
    state: Mutex<WebState>,
}

#[derive(Clone, Copy)]
struct WebServiceConfig {
    cache_ttl: Duration,
    page_max_entries: usize,
    search_max_entries: usize,
    timeout: Duration,
    max_body_bytes: usize,
    metadata_token_reserve: usize,
    find_match_limit: usize,
}

impl Default for WebServiceConfig {
    fn default() -> Self {
        Self {
            cache_ttl: DEFAULT_CACHE_TTL,
            page_max_entries: DEFAULT_PAGE_CACHE_MAX_ENTRIES,
            search_max_entries: DEFAULT_SEARCH_CACHE_MAX_ENTRIES,
            timeout: DEFAULT_TIMEOUT,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            metadata_token_reserve: DEFAULT_OPEN_METADATA_TOKEN_RESERVE,
            find_match_limit: DEFAULT_FIND_MATCH_LIMIT,
        }
    }
}

#[derive(Default)]
struct WebState {
    pages: HashMap<String, Arc<CachedPage>>,
    page_order: VecDeque<String>,
    searches: HashMap<String, Arc<CachedSearch>>,
    search_order: VecDeque<String>,
    aliases: HashMap<String, CachedAlias>,
    url_to_ref: HashMap<String, String>,
    next_ref_id: u64,
}

#[derive(Clone)]
struct CachedAlias {
    normalized_url: String,
    cached_at: Instant,
}

struct CachedSearch {
    fetched_at: DateTime<Utc>,
    cached_at: Instant,
    results: Vec<SearchResultItem>,
}

struct CachedPage {
    ref_id: String,
    requested_url: String,
    final_url: String,
    status_code: u16,
    status_text: String,
    content_type: Option<String>,
    title: Option<String>,
    fetched_at: DateTime<Utc>,
    cached_at: Instant,
    lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResponseLength {
    Short,
    Medium,
    Long,
}

impl ResponseLength {
    fn search_limit(self) -> usize {
        match self {
            Self::Short => 3,
            Self::Medium => 5,
            Self::Long => 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct SearchResultItem {
    pub(crate) ref_id: String,
    pub(crate) title: String,
    pub(crate) url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct SearchResults {
    pub(crate) query: String,
    pub(crate) fetched_at: DateTime<Utc>,
    pub(crate) results: Vec<SearchResultItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct OpenPageChunk {
    pub(crate) ref_id: String,
    pub(crate) requested_ref: String,
    pub(crate) requested_url: String,
    pub(crate) final_url: String,
    pub(crate) status_code: u16,
    pub(crate) status_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    pub(crate) fetched_at: DateTime<Utc>,
    pub(crate) requested_lineno: usize,
    pub(crate) start_lineno: usize,
    pub(crate) end_lineno: usize,
    pub(crate) total_lines: usize,
    pub(crate) is_end: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_lineno: Option<usize>,
    pub(crate) content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct FindMatch {
    pub(crate) line: usize,
    pub(crate) text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct FindResults {
    pub(crate) ref_id: String,
    pub(crate) requested_ref: String,
    pub(crate) url: String,
    pub(crate) pattern: String,
    pub(crate) total_matches: usize,
    pub(crate) returned_matches: usize,
    pub(crate) truncated: bool,
    pub(crate) matches: Vec<FindMatch>,
}

impl WebService {
    pub(crate) fn new(max_output_tokens: usize) -> Result<Self> {
        Self::with_config(max_output_tokens, WebServiceConfig::default())
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests(
        max_output_tokens: usize,
        cache_ttl: Duration,
        max_body_bytes: usize,
    ) -> Result<Self> {
        Self::with_config(
            max_output_tokens,
            WebServiceConfig {
                cache_ttl,
                max_body_bytes,
                ..WebServiceConfig::default()
            },
        )
    }

    fn with_config(max_output_tokens: usize, config: WebServiceConfig) -> Result<Self> {
        let tokenizer = TokenCounter::cl100k().context("failed to initialize web tokenizer")?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .timeout(config.timeout)
            .user_agent(DEFAULT_USER_AGENT)
            .build()
            .context("failed to build web HTTP client")?;
        let open_chunk_tokens = if max_output_tokens > config.metadata_token_reserve {
            max_output_tokens - config.metadata_token_reserve
        } else {
            (max_output_tokens.saturating_mul(3) / 4).max(1)
        };

        Ok(Self {
            inner: Arc::new(WebServiceInner {
                client,
                tokenizer,
                config,
                open_chunk_tokens,
                state: Mutex::new(WebState::default()),
            }),
        })
    }

    pub(crate) async fn search_query(
        &self,
        query: &str,
        response_length: ResponseLength,
    ) -> Result<SearchResults> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            bail!("query must not be empty");
        }

        if let Some(cached) = self.lookup_search(trimmed) {
            return Ok(SearchResults {
                query: trimmed.to_string(),
                fetched_at: cached.fetched_at,
                results: cached
                    .results
                    .into_iter()
                    .take(response_length.search_limit())
                    .collect(),
            });
        }

        let fetched = self.fetch_search_results(trimmed).await?;
        let response = SearchResults {
            query: trimmed.to_string(),
            fetched_at: fetched.fetched_at,
            results: fetched
                .results
                .iter()
                .take(response_length.search_limit())
                .cloned()
                .collect(),
        };
        self.store_search(trimmed.to_string(), fetched);
        Ok(response)
    }

    pub(crate) async fn open(
        &self,
        ref_id_or_url: &str,
        lineno: Option<usize>,
    ) -> Result<OpenPageChunk> {
        let requested_ref = ref_id_or_url.trim().to_string();
        let requested_lineno = lineno.unwrap_or(1).max(1);
        let (normalized_url, requested_url) = self.resolve_target(ref_id_or_url)?;

        if let Some(cached) = self.lookup_page(&normalized_url) {
            return cached.chunk(
                &self.inner.tokenizer,
                self.inner.open_chunk_tokens,
                requested_ref,
                requested_lineno,
            );
        }

        let fetched = self.fetch_page(&requested_url, &normalized_url).await?;
        let chunk = fetched.chunk(
            &self.inner.tokenizer,
            self.inner.open_chunk_tokens,
            requested_ref,
            requested_lineno,
        )?;
        self.store_page(normalized_url, fetched);
        Ok(chunk)
    }

    pub(crate) async fn find(&self, ref_id_or_url: &str, pattern: &str) -> Result<FindResults> {
        let requested_ref = ref_id_or_url.trim().to_string();
        let trimmed_pattern = pattern.trim();
        if trimmed_pattern.is_empty() {
            bail!("pattern must not be empty");
        }

        let (normalized_url, requested_url) = self.resolve_target(ref_id_or_url)?;
        let page = if let Some(cached) = self.lookup_page(&normalized_url) {
            cached
        } else {
            let fetched = self.fetch_page(&requested_url, &normalized_url).await?;
            let cached = Arc::new(fetched);
            self.store_page_arc(normalized_url, Arc::clone(&cached));
            cached
        };

        Ok(page.find(
            trimmed_pattern,
            requested_ref,
            self.inner.config.find_match_limit,
        ))
    }

    fn lookup_search(&self, query: &str) -> Option<SearchResults> {
        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        let cached = state.searches.get(query)?.clone();
        Some(SearchResults {
            query: query.to_string(),
            fetched_at: cached.fetched_at,
            results: cached.results.clone(),
        })
    }

    fn lookup_page(&self, normalized_url: &str) -> Option<Arc<CachedPage>> {
        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        state.pages.get(normalized_url).cloned()
    }

    fn resolve_target(&self, ref_id_or_url: &str) -> Result<(String, String)> {
        let trimmed = ref_id_or_url.trim();
        if trimmed.is_empty() {
            bail!("ref_id must not be empty");
        }

        if let Ok(normalized_url) = normalize_url(trimmed) {
            return Ok((normalized_url, trimmed.to_string()));
        }

        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        let alias = state
            .aliases
            .get_mut(trimmed)
            .ok_or_else(|| anyhow::anyhow!("unknown ref_id `{trimmed}`"))?;
        alias.cached_at = Instant::now();
        Ok((alias.normalized_url.clone(), alias.normalized_url.clone()))
    }

    fn store_search(&self, query: String, fetched: CachedSearch) {
        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        state.search_order.retain(|existing| existing != &query);
        state.search_order.push_back(query.clone());
        state.searches.insert(query, Arc::new(fetched));
        while state.searches.len() > self.inner.config.search_max_entries {
            let Some(oldest_query) = state.search_order.pop_front() else {
                break;
            };
            state.searches.remove(&oldest_query);
        }
    }

    fn store_page(&self, normalized_url: String, fetched: CachedPage) {
        self.store_page_arc(normalized_url, Arc::new(fetched));
    }

    fn store_page_arc(&self, normalized_url: String, fetched: Arc<CachedPage>) {
        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        ensure_alias_locked(&mut state, &normalized_url, Some(&fetched.ref_id));
        state
            .page_order
            .retain(|existing| existing != &normalized_url);
        state.page_order.push_back(normalized_url.clone());
        state.pages.insert(normalized_url, fetched);
        while state.pages.len() > self.inner.config.page_max_entries {
            let Some(oldest_url) = state.page_order.pop_front() else {
                break;
            };
            state.pages.remove(&oldest_url);
        }
    }

    async fn fetch_search_results(&self, query: &str) -> Result<CachedSearch> {
        let encoded = urlencoding::encode(query);
        let url = format!("https://html.duckduckgo.com/html/?q={encoded}");
        let response = self
            .inner
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("failed to search for `{query}`"))?;
        let html = response
            .text()
            .await
            .with_context(|| format!("failed to read search results for `{query}`"))?;

        let parsed = parse_duckduckgo_results(&html)?;
        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        let results = parsed
            .into_iter()
            .filter_map(|result| {
                let normalized_url = normalize_url(&result.url).ok()?;
                let ref_id = ensure_alias_locked(&mut state, &normalized_url, None);
                Some(SearchResultItem {
                    ref_id,
                    title: result.title,
                    url: normalized_url,
                    snippet: result.snippet,
                })
            })
            .collect::<Vec<_>>();

        Ok(CachedSearch {
            fetched_at: Utc::now(),
            cached_at: Instant::now(),
            results,
        })
    }

    async fn fetch_page(&self, requested_url: &str, normalized_url: &str) -> Result<CachedPage> {
        let response = self
            .inner
            .client
            .get(normalized_url)
            .send()
            .await
            .with_context(|| format!("failed to fetch {requested_url}"))?;
        let status = response.status();
        let final_url = response.url().to_string();
        let content_type = header_to_string(response.headers().get(CONTENT_TYPE));
        if !is_supported_content_type(content_type.as_deref()) {
            let label = content_type.as_deref().unwrap_or("unknown");
            bail!(
                "Unsupported content type `{label}`. WebRun only supports text-like HTTP responses."
            );
        }

        let body_bytes = response
            .bytes()
            .await
            .with_context(|| format!("failed to read response body from {requested_url}"))?;
        if body_bytes.len() > self.inner.config.max_body_bytes {
            bail!(
                "Response body exceeded the {} byte limit.",
                self.inner.config.max_body_bytes
            );
        }

        let body = String::from_utf8_lossy(&body_bytes).into_owned();
        let page = extract_page(&body, content_type.as_deref());
        let fetched_at = Utc::now();

        let mut state = self.inner.state.lock().expect("web state lock");
        prune_expired_locked(&mut state, &self.inner.config);
        let ref_id = ensure_alias_locked(&mut state, normalized_url, None);

        Ok(CachedPage {
            ref_id,
            requested_url: requested_url.trim().to_string(),
            final_url,
            status_code: status.as_u16(),
            status_text: format_status(status),
            content_type,
            title: page.title,
            fetched_at,
            cached_at: Instant::now(),
            lines: page.lines,
        })
    }
}

impl CachedPage {
    fn chunk(
        &self,
        tokenizer: &TokenCounter,
        token_budget: usize,
        requested_ref: String,
        requested_lineno: usize,
    ) -> Result<OpenPageChunk> {
        let total_lines = self.lines.len();
        if total_lines == 0 {
            if requested_lineno > 1 {
                bail!(
                    "line {} is out of range for `{}` (total lines: 0)",
                    requested_lineno,
                    self.requested_url
                );
            }

            return Ok(OpenPageChunk {
                ref_id: self.ref_id.clone(),
                requested_ref,
                requested_url: self.requested_url.clone(),
                final_url: self.final_url.clone(),
                status_code: self.status_code,
                status_text: self.status_text.clone(),
                content_type: self.content_type.clone(),
                title: self.title.clone(),
                fetched_at: self.fetched_at,
                requested_lineno: 1,
                start_lineno: 1,
                end_lineno: 0,
                total_lines: 0,
                is_end: true,
                next_lineno: None,
                content: String::new(),
            });
        }

        let start_index = requested_lineno
            .checked_sub(1)
            .ok_or_else(|| anyhow::anyhow!("line numbers start at 1"))?;
        if start_index >= total_lines {
            bail!(
                "line {} is out of range for `{}` (total lines: {total_lines})",
                requested_lineno,
                self.requested_url
            );
        }

        let mut used_tokens = 0usize;
        let mut rendered = Vec::new();
        let mut end_index = start_index;
        while end_index < total_lines {
            let numbered = format!("L{}: {}", end_index + 1, self.lines[end_index]);
            let line_tokens = tokenizer.encode_text(&numbered).len().max(1);
            if end_index > start_index && used_tokens + line_tokens > token_budget.max(1) {
                break;
            }
            used_tokens += line_tokens;
            rendered.push(numbered);
            end_index += 1;
        }

        Ok(OpenPageChunk {
            ref_id: self.ref_id.clone(),
            requested_ref,
            requested_url: self.requested_url.clone(),
            final_url: self.final_url.clone(),
            status_code: self.status_code,
            status_text: self.status_text.clone(),
            content_type: self.content_type.clone(),
            title: self.title.clone(),
            fetched_at: self.fetched_at,
            requested_lineno,
            start_lineno: start_index + 1,
            end_lineno: end_index,
            total_lines,
            is_end: end_index >= total_lines,
            next_lineno: (end_index < total_lines).then_some(end_index + 1),
            content: rendered.join("\n"),
        })
    }

    fn find(&self, pattern: &str, requested_ref: String, limit: usize) -> FindResults {
        let pattern_lower = pattern.to_ascii_lowercase();
        let mut matches = Vec::new();
        let mut total_matches = 0usize;

        for (index, line) in self.lines.iter().enumerate() {
            if line.to_ascii_lowercase().contains(&pattern_lower) {
                total_matches += 1;
                if matches.len() < limit {
                    matches.push(FindMatch {
                        line: index + 1,
                        text: line.clone(),
                    });
                }
            }
        }

        FindResults {
            ref_id: self.ref_id.clone(),
            requested_ref,
            url: self.final_url.clone(),
            pattern: pattern.to_string(),
            total_matches,
            returned_matches: matches.len(),
            truncated: total_matches > matches.len(),
            matches,
        }
    }
}

#[derive(Debug)]
struct ExtractedPage {
    title: Option<String>,
    lines: Vec<String>,
}

#[derive(Debug)]
struct ParsedSearchResult {
    title: String,
    url: String,
    snippet: String,
}

fn prune_expired_locked(state: &mut WebState, config: &WebServiceConfig) {
    let expired_pages = state
        .pages
        .iter()
        .filter_map(|(url, page)| {
            (page.cached_at.elapsed() >= config.cache_ttl).then_some(url.clone())
        })
        .collect::<Vec<_>>();
    for url in expired_pages {
        state.pages.remove(&url);
        state.page_order.retain(|existing| existing != &url);
    }

    let expired_searches = state
        .searches
        .iter()
        .filter_map(|(query, cached)| {
            (cached.cached_at.elapsed() >= config.cache_ttl).then_some(query.clone())
        })
        .collect::<Vec<_>>();
    for query in expired_searches {
        state.searches.remove(&query);
        state.search_order.retain(|existing| existing != &query);
    }

    let expired_aliases = state
        .aliases
        .iter()
        .filter_map(|(ref_id, alias)| {
            (alias.cached_at.elapsed() >= config.cache_ttl)
                .then_some((ref_id.clone(), alias.normalized_url.clone()))
        })
        .collect::<Vec<_>>();
    for (ref_id, normalized_url) in expired_aliases {
        state.aliases.remove(&ref_id);
        if state
            .url_to_ref
            .get(&normalized_url)
            .is_some_and(|current| current == &ref_id)
        {
            state.url_to_ref.remove(&normalized_url);
        }
    }
}

fn ensure_alias_locked(
    state: &mut WebState,
    normalized_url: &str,
    preferred_ref_id: Option<&str>,
) -> String {
    if let Some(existing) = state.url_to_ref.get(normalized_url).cloned() {
        if let Some(alias) = state.aliases.get_mut(&existing) {
            alias.cached_at = Instant::now();
        }
        return existing;
    }

    let ref_id = preferred_ref_id.map(ToOwned::to_owned).unwrap_or_else(|| {
        state.next_ref_id += 1;
        format!("web_{:x}", state.next_ref_id)
    });
    state.aliases.insert(
        ref_id.clone(),
        CachedAlias {
            normalized_url: normalized_url.to_string(),
            cached_at: Instant::now(),
        },
    );
    state
        .url_to_ref
        .insert(normalized_url.to_string(), ref_id.clone());
    ref_id
}

fn extract_page(body: &str, content_type: Option<&str>) -> ExtractedPage {
    if is_html_content_type(content_type) {
        extract_html_page(body)
    } else {
        extract_plain_page(body)
    }
}

fn extract_plain_page(body: &str) -> ExtractedPage {
    let lines = normalize_lines(
        body.lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>(),
    );
    let title = lines.iter().find(|line| !line.is_empty()).cloned();
    ExtractedPage { title, lines }
}

fn extract_html_page(body: &str) -> ExtractedPage {
    let cleaned = strip_non_content_blocks(body);
    let document = Html::parse_document(&cleaned);

    let title = selector("title")
        .and_then(|sel| document.select(&sel).next())
        .map(extract_element_text)
        .filter(|title| !title.is_empty());

    let mut lines = selector("h1, h2, h3, h4, h5, h6, p, li, pre, code, blockquote, td, th")
        .map(|sel| {
            document
                .select(&sel)
                .map(extract_element_text)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if lines.is_empty()
        && let Some(body_text) = selector("body")
            .and_then(|sel| document.select(&sel).next())
            .map(extract_element_text)
            .filter(|line| !line.is_empty())
    {
        lines.push(body_text);
    }

    ExtractedPage {
        title,
        lines: normalize_lines(lines),
    }
}

fn parse_duckduckgo_results(html: &str) -> Result<Vec<ParsedSearchResult>> {
    let document = Html::parse_document(html);
    let result_selector =
        selector(".result").ok_or_else(|| anyhow::anyhow!("invalid result selector"))?;
    let title_selector = selector(".result__title a.result__a, a.result__a")
        .ok_or_else(|| anyhow::anyhow!("invalid title selector"))?;
    let snippet_selector =
        selector(".result__snippet").ok_or_else(|| anyhow::anyhow!("invalid snippet selector"))?;

    let mut results = Vec::new();
    for result in document.select(&result_selector) {
        let Some(link) = result.select(&title_selector).next() else {
            continue;
        };
        let Some(href) = link.value().attr("href") else {
            continue;
        };
        let Some(url) = resolve_search_result_url(href) else {
            continue;
        };
        let title = extract_element_text(link);
        if title.is_empty() {
            continue;
        }
        let snippet = result
            .select(&snippet_selector)
            .next()
            .map(extract_element_text)
            .unwrap_or_default();

        results.push(ParsedSearchResult {
            title,
            url,
            snippet: truncate_chars(&snippet, 240),
        });
    }

    Ok(results)
}

fn resolve_search_result_url(raw_href: &str) -> Option<String> {
    let absolute = if raw_href.starts_with("//") {
        format!("https:{raw_href}")
    } else if raw_href.starts_with('/') {
        format!("https://duckduckgo.com{raw_href}")
    } else {
        raw_href.to_string()
    };

    let parsed = Url::parse(&absolute).ok()?;
    if parsed.domain() == Some("duckduckgo.com")
        && let Some(decoded) = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "uddg").then_some(value.into_owned()))
    {
        return Some(decoded);
    }

    Some(absolute)
}

fn selector(css: &str) -> Option<Selector> {
    Selector::parse(css).ok()
}

fn extract_element_text(element: ElementRef<'_>) -> String {
    normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "))
}

fn normalize_lines(lines: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    let mut previous_blank = false;
    for raw in lines {
        let line = raw.trim().to_string();
        let is_blank = line.is_empty();
        if is_blank {
            if previous_blank || normalized.is_empty() {
                continue;
            }
            previous_blank = true;
            normalized.push(String::new());
        } else {
            previous_blank = false;
            normalized.push(line);
        }
    }

    while normalized.last().is_some_and(|line| line.is_empty()) {
        normalized.pop();
    }

    normalized
}

fn normalize_whitespace(input: &str) -> String {
    whitespace_regex()
        .replace_all(input.trim(), " ")
        .trim()
        .to_string()
}

fn strip_non_content_blocks(input: &str) -> String {
    let no_script = script_regex().replace_all(input, " ");
    let no_style = style_regex().replace_all(&no_script, " ");
    noscript_regex().replace_all(&no_style, " ").to_string()
}

fn script_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new("(?is)<script\\b.*?</script>").expect("script regex"))
}

fn style_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new("(?is)<style\\b.*?</style>").expect("style regex"))
}

fn noscript_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new("(?is)<noscript\\b.*?</noscript>").expect("noscript regex"))
}

fn whitespace_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new("\\s+").expect("whitespace regex"))
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn normalize_url(url: &str) -> Result<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        bail!("url must not be empty");
    }
    let parsed = Url::parse(trimmed).with_context(|| format!("invalid url `{trimmed}`"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed.to_string()),
        scheme => bail!("unsupported URL scheme `{scheme}`"),
    }
}

fn header_to_string(value: Option<&HeaderValue>) -> Option<String> {
    value
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn is_supported_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return true;
    };
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();

    !(mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || matches!(
            mime.as_str(),
            "application/pdf"
                | "application/octet-stream"
                | "application/zip"
                | "application/gzip"
                | "application/x-gzip"
        ))
}

fn is_html_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    mime == "text/html" || mime == "application/xhtml+xml"
}

fn format_status(status: StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("{} {}", status.as_u16(), reason),
        None => status.as_u16().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
    };

    struct TestResponse {
        status_line: &'static str,
        content_type: &'static str,
        body: String,
        extra_headers: Vec<(String, String)>,
    }

    impl TestResponse {
        fn ok(body: impl Into<String>, content_type: &'static str) -> Self {
            Self {
                status_line: "HTTP/1.1 200 OK",
                content_type,
                body: body.into(),
                extra_headers: Vec::new(),
            }
        }
    }

    struct TestServer {
        base_url: String,
        hits: Arc<AtomicUsize>,
        shutdown_tx: mpsc::Sender<()>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl TestServer {
        fn start(handler: impl Fn(usize, &str) -> TestResponse + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
            listener
                .set_nonblocking(true)
                .expect("set nonblocking listener");
            let address = listener.local_addr().expect("local addr");
            let hits = Arc::new(AtomicUsize::new(0));
            let handler = Arc::new(handler);
            let (shutdown_tx, shutdown_rx) = mpsc::channel();
            let server_hits = hits.clone();

            let handle = thread::spawn(move || {
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }

                    let Ok((mut stream, _)) = listener.accept() else {
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .expect("set read timeout");

                    let mut buffer = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(read) => {
                                buffer.extend_from_slice(&chunk[..read]);
                                if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                            {
                                break;
                            }
                            Err(_) => break,
                        }
                    }

                    let request = String::from_utf8_lossy(&buffer);
                    let request_line = request.lines().next().unwrap_or_default();
                    let path = request_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    let hit = server_hits.fetch_add(1, Ordering::SeqCst) + 1;
                    let response = handler(hit, &path);
                    let mut raw = format!(
                        "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                        response.status_line,
                        response.content_type,
                        response.body.len()
                    );
                    for (name, value) in response.extra_headers {
                        raw.push_str(&format!("{name}: {value}\r\n"));
                    }
                    raw.push_str("\r\n");
                    raw.push_str(&response.body);
                    let _ = stream.write_all(raw.as_bytes());
                    let _ = stream.flush();
                }
            });

            Self {
                base_url: format!("http://{}", address),
                hits,
                shutdown_tx,
                handle: Some(handle),
            }
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            let _ = self.shutdown_tx.send(());
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    #[test]
    fn parses_duckduckgo_results_and_decodes_redirect_urls() {
        let html = r#"
        <div class="result">
          <div class="result__title">
            <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs">Example Docs</a>
          </div>
          <a class="result__snippet">Example snippet text.</a>
        </div>
        "#;

        let results = parse_duckduckgo_results(html).expect("parses");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example Docs");
        assert_eq!(results[0].url, "https://example.com/docs");
        assert_eq!(results[0].snippet, "Example snippet text.");
    }

    #[test]
    fn extracts_html_text_and_ignores_script_content() {
        let html = r#"
        <html>
          <head>
            <title>Demo Title</title>
            <script>window.bad = true;</script>
          </head>
          <body>
            <h1>Heading</h1>
            <p>First paragraph.</p>
            <p>Second paragraph.</p>
          </body>
        </html>
        "#;

        let page = extract_html_page(html);
        assert_eq!(page.title.as_deref(), Some("Demo Title"));
        assert!(page.lines.contains(&"Heading".to_string()));
        assert!(page.lines.contains(&"First paragraph.".to_string()));
        assert!(page.lines.contains(&"Second paragraph.".to_string()));
        assert!(!page.lines.iter().any(|line| line.contains("window.bad")));
    }

    #[tokio::test]
    async fn open_reuses_cached_page_and_returns_line_chunks() {
        let long_line = "token ".repeat(40);
        let body = format!(
            "line one {long_line}\nline two {long_line}\nline three {long_line}\nline four {long_line}\n"
        );
        let server = TestServer::start(move |_, _| TestResponse::ok(body.clone(), "text/plain"));
        let service =
            WebService::new_for_tests(96, Duration::from_secs(60), 16 * 1024).expect("service");
        let url = format!("{}/page", server.base_url);

        let first = service.open(&url, Some(1)).await.expect("first open");
        let continuation = first.next_lineno.expect("continuation");
        let second = service
            .open(&first.ref_id, Some(continuation))
            .await
            .expect("second open");

        assert_eq!(server.hits.load(Ordering::SeqCst), 1);
        assert!(first.content.contains("L1: line one"));
        assert!(!first.is_end);
        assert_eq!(second.start_lineno, continuation);
    }

    #[tokio::test]
    async fn find_searches_cached_pages_by_ref_id() {
        let server = TestServer::start(|_, _| {
            TestResponse::ok("alpha\nbeta target\ngamma target\n", "text/plain")
        });
        let service =
            WebService::new_for_tests(128, Duration::from_secs(60), 16 * 1024).expect("service");
        let opened = service
            .open(&format!("{}/find", server.base_url), Some(1))
            .await
            .expect("open");

        let found = service.find(&opened.ref_id, "target").await.expect("find");

        assert_eq!(found.total_matches, 2);
        assert_eq!(found.returned_matches, 2);
        assert_eq!(found.matches[0].line, 2);
        assert_eq!(found.matches[1].line, 3);
    }

    #[tokio::test]
    async fn expired_pages_refetch() {
        let server =
            TestServer::start(|hit, _| TestResponse::ok(format!("version {hit}"), "text/plain"));
        let service =
            WebService::new_for_tests(128, Duration::from_millis(5), 16 * 1024).expect("service");
        let url = format!("{}/ttl", server.base_url);

        let first = service.open(&url, Some(1)).await.expect("first");
        tokio::time::sleep(Duration::from_millis(20)).await;
        let second = service.open(&url, Some(1)).await.expect("second");

        assert_eq!(server.hits.load(Ordering::SeqCst), 2);
        assert_ne!(first.fetched_at, second.fetched_at);
        assert_ne!(first.content, second.content);
    }

    #[tokio::test]
    #[ignore = "manual live smoke test requiring network access"]
    async fn live_open_known_llms_txt_returns_content() {
        let service =
            WebService::new_for_tests(512, Duration::from_secs(60), 256 * 1024).expect("service");
        let page = service
            .open(
                "https://mastra.ai/guides/getting-started/quickstart/llms.txt",
                Some(1),
            )
            .await
            .expect("open live llms.txt");

        assert!(page.total_lines > 0, "expected visible lines, got none");
        assert!(
            page.content.contains("Mastra Quickstart"),
            "expected quickstart heading in content, got: {}",
            page.content
        );
    }
}
