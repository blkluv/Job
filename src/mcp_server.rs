// src/mcp_server.rs
// Standalone MCP Server for Nostr Job Listings (Kind 9993) with Performance Metrics

use std::sync::Arc;
use std::time::Duration;
use nostr_sdk::prelude::*;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    prompt, prompt_handler, prompt_router, schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde_json::json;
use tokio::sync::{Mutex, RwLock};
use tokio::time::timeout;
use std::collections::HashMap;

// ==================== Configuration ====================

#[allow(dead_code)]
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RELAY_FETCH_TIMEOUT: Duration = Duration::from_secs(2);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(30);

// ==================== Performance Metrics ====================

#[derive(Clone, Debug, Default)]
struct PerformanceMetrics {
    total_requests: usize,
    cache_hits: usize,
    cache_misses: usize,
    relay_fetches: usize,
    failed_fetches: usize,
    total_fetch_time_ms: u128,
    total_cache_time_ms: u128,
    fastest_fetch_ms: Option<u128>,
    slowest_fetch_ms: Option<u128>,
    fastest_cache_ms: Option<u128>,
    slowest_cache_ms: Option<u128>,
}

impl PerformanceMetrics {
    fn record_cache_hit(&mut self, duration_ms: u128) {
        self.total_requests += 1;
        self.cache_hits += 1;
        self.total_cache_time_ms += duration_ms;
        
        self.fastest_cache_ms = Some(
            self.fastest_cache_ms.map_or(duration_ms, |f| f.min(duration_ms))
        );
        self.slowest_cache_ms = Some(
            self.slowest_cache_ms.map_or(duration_ms, |s| s.max(duration_ms))
        );
    }

    fn record_cache_miss(&mut self, duration_ms: u128, success: bool) {
        self.total_requests += 1;
        self.cache_misses += 1;
        
        if success {
            self.relay_fetches += 1;
            self.total_fetch_time_ms += duration_ms;
            
            self.fastest_fetch_ms = Some(
                self.fastest_fetch_ms.map_or(duration_ms, |f| f.min(duration_ms))
            );
            self.slowest_fetch_ms = Some(
                self.slowest_fetch_ms.map_or(duration_ms, |s| s.max(duration_ms))
            );
        } else {
            self.failed_fetches += 1;
        }
    }

    fn cache_hit_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            (self.cache_hits as f64 / self.total_requests as f64) * 100.0
        }
    }

    fn avg_cache_time(&self) -> f64 {
        if self.cache_hits == 0 {
            0.0
        } else {
            self.total_cache_time_ms as f64 / self.cache_hits as f64
        }
    }

    fn avg_fetch_time(&self) -> f64 {
        if self.relay_fetches == 0 {
            0.0
        } else {
            self.total_fetch_time_ms as f64 / self.relay_fetches as f64
        }
    }

    fn time_saved_ms(&self) -> u128 {
        if self.cache_hits == 0 || self.relay_fetches == 0 {
            return 0;
        }
        
        let avg_fetch = self.avg_fetch_time();
        let avg_cache = self.avg_cache_time();
        let time_saved_per_hit = (avg_fetch - avg_cache).max(0.0);
        
        (time_saved_per_hit * self.cache_hits as f64) as u128
    }

    fn format_report(&self) -> String {
        format!(
            "📊 Performance Metrics Report\n\
            ═══════════════════════════════════════════════════════════\n\n\
            🔢 Request Statistics:\n\
            • Total Requests: {}\n\
            • Cache Hits: {} ({}%)\n\
            • Cache Misses: {}\n\
            • Relay Fetches: {}\n\
            • Failed Fetches: {}\n\n\
            ⚡ Cache Performance:\n\
            • Average Cache Response: {:.2}ms\n\
            • Fastest Cache Hit: {}ms\n\
            • Slowest Cache Hit: {}ms\n\n\
            🌐 Relay Performance:\n\
            • Average Relay Fetch: {:.2}ms\n\
            • Fastest Fetch: {}ms\n\
            • Slowest Fetch: {}ms\n\n\
            💡 Performance Gains:\n\
            • Cache Hit Rate: {:.1}%\n\
            • Time Saved by Cache: {:.2}s\n\
            • Speed Improvement: {:.1}x faster with cache\n\n\
            📈 Efficiency Metrics:\n\
            • Relay Load Reduction: {:.1}%\n\
            • Success Rate: {:.1}%",
            self.total_requests,
            self.cache_hits,
            self.cache_hit_rate(),
            self.cache_misses,
            self.relay_fetches,
            self.failed_fetches,
            self.avg_cache_time(),
            self.fastest_cache_ms.unwrap_or(0),
            self.slowest_cache_ms.unwrap_or(0),
            self.avg_fetch_time(),
            self.fastest_fetch_ms.unwrap_or(0),
            self.slowest_fetch_ms.unwrap_or(0),
            self.cache_hit_rate(),
            self.time_saved_ms() as f64 / 1000.0,
            if self.avg_cache_time() > 0.0 { 
                self.avg_fetch_time() / self.avg_cache_time() 
            } else { 
                1.0 
            },
            if self.total_requests > 0 {
                (self.cache_hits as f64 / self.total_requests as f64) * 100.0
            } else {
                0.0
            },
            if self.total_requests > 0 {
                (self.relay_fetches as f64 / self.total_requests as f64) * 100.0
            } else {
                0.0
            }
        )
    }
}

// ==================== Cache Types ====================

#[derive(Clone, Debug)]
struct CachedEvents {
    events: Vec<Event>,
    timestamp: std::time::Instant,
}

impl CachedEvents {
    fn is_fresh(&self, ttl: Duration) -> bool {
        self.timestamp.elapsed() < ttl
    }
}

// ==================== Request/Response Types ====================

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchJobsArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employment_type: Option<String>,
    
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetJobArgs {
    pub job_id: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct JobAnalysisArgs {
    pub query: String,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

// ==================== Nostr Jobs MCP Server ====================

#[derive(Clone, Debug)]
pub struct NostrJobsServer {
    client: Arc<Mutex<Client>>,
    relays: Vec<String>,
    cache: Arc<RwLock<HashMap<String, CachedEvents>>>,
    relay_healthy: Arc<Mutex<bool>>,
    metrics: Arc<RwLock<PerformanceMetrics>>,
    pub tool_router: ToolRouter<NostrJobsServer>,
    pub prompt_router: PromptRouter<NostrJobsServer>,
}

#[tool_router]
impl NostrJobsServer {
    pub async fn new() -> Self {
        let client = Client::default();
        
        let relays = vec![
            "wss://relay.damus.io".to_string(),
            "wss://relay.nostr.band".to_string(),
            "wss://nos.lol".to_string(),
        ];

        tracing::info!(
            relay_count = relays.len(),
            relays = ?relays,
            "initializing_nostr_mcp_server"
        );

        for relay in &relays {
            let _ = client.add_relay(relay).await;
        }
        
        let client_clone = client.clone();
        tokio::spawn(async move {
            let _ = timeout(Duration::from_secs(15), client_clone.connect()).await;
        });

        let server = Self {
            client: Arc::new(Mutex::new(client)),
            relays,
            cache: Arc::new(RwLock::new(HashMap::new())),
            relay_healthy: Arc::new(Mutex::new(false)),
            metrics: Arc::new(RwLock::new(PerformanceMetrics::default())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        };

        let server_clone = server.clone();
        tokio::spawn(async move {
            server_clone.health_check_loop().await;
        });

        tracing::info!("nostr_mcp_server_initialized");

        server
    }

    async fn health_check_loop(&self) {
        loop {
            tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
            
            let client = self.client.lock().await;
            let filter = Filter::new().limit(1);
            
            match timeout(Duration::from_secs(5), client.fetch_events(filter, Duration::from_secs(3))).await {
                Ok(Ok(_)) => {
                    let was_healthy = *self.relay_healthy.lock().await;
                    *self.relay_healthy.lock().await = true;
                    
                    if !was_healthy {
                        tracing::info!("relay_health_recovered");
                    }
                }
                _ => {
                    let was_healthy = *self.relay_healthy.lock().await;
                    *self.relay_healthy.lock().await = false;
                    
                    if was_healthy {
                        tracing::warn!("relay_health_degraded");
                    }
                }
            }
        }
    }

    // ==================== Helper Methods ====================

    fn format_job_summary(&self, event: &Event) -> String {
        let tags: Vec<_> = event.tags.iter().collect();
        
        let title = Self::find_tag_value(&tags, "title").unwrap_or_else(|| "Untitled".to_string());
        let company = Self::find_tag_value(&tags, "company").unwrap_or_else(|| "Unknown".to_string());
        let location = Self::find_tag_value(&tags, "location").unwrap_or_else(|| "Remote".to_string());
        let job_id = Self::find_tag_value(&tags, "job-id").unwrap_or_else(|| event.id.to_hex());
        
        let skills: Vec<_> = tags
            .iter()
            .filter_map(|t| {
                let slice = t.as_slice();
                if slice.len() >= 2 && slice[0] == "skill" {
                    Some(slice[1].to_string())
                } else {
                    None
                }
            })
            .collect();

        let employment_types: Vec<_> = tags
            .iter()
            .filter_map(|t| {
                let slice = t.as_slice();
                if slice.len() >= 2 && slice[0] == "employment-type" {
                    Some(slice[1].to_string())
                } else {
                    None
                }
            })
            .collect();

        let salary = tags.iter().find(|t| {
            let slice = t.as_slice();
            !slice.is_empty() && slice[0] == "salary"
        }).and_then(|tag| {
            let slice = tag.as_slice();
            if slice.len() >= 5 {
                Some(format!("${} - ${} {} per {}", slice[1], slice[2], slice[3], slice[4]))
            } else {
                None
            }
        });

        format!(
            "🏢 {} - {}\n📍 Location: {}\n💼 Type: {}\n🛠️  Skills: {}\n{}\n🆔 Job ID: {}\n📅 Posted: {}",
            company,
            title,
            location,
            if employment_types.is_empty() { "Not specified".to_string() } else { employment_types.join(", ") },
            if skills.is_empty() { "Not specified".to_string() } else { skills.join(", ") },
            salary.map(|s| format!("💰 Salary: {}", s)).unwrap_or_default(),
            job_id,
            event.created_at.to_human_datetime()
        )
    }

    fn find_tag_value(tags: &[&Tag], name: &str) -> Option<String> {
        tags.iter().find_map(|t| {
            let slice = t.as_slice();
            if slice.len() >= 2 && slice[0] == name {
                Some(slice[1].to_string())
            } else {
                None
            }
        })
    }

    fn build_filter(_company: Option<&str>, _skill: Option<&str>, _employment_type: Option<&str>, _limit: usize) -> Filter {
        Filter::new()
            .kind(Kind::from(9993u16))
            .limit(100)
    }

    fn cache_key(company: Option<&str>, skill: Option<&str>, employment_type: Option<&str>, limit: usize) -> String {
        format!("{}:{}:{}:{}", 
            company.unwrap_or("*"),
            skill.unwrap_or("*"),
            employment_type.unwrap_or("*"),
            limit
        )
    }

    async fn fetch_events_fast(
        &self,
        filter: Filter,
        cache_key: String,
    ) -> Result<Vec<Event>, String> {
        let start = std::time::Instant::now();
        let client = self.client.lock().await;
        
        match timeout(RELAY_FETCH_TIMEOUT, client.fetch_events(filter, Duration::from_millis(1500))).await {
            Ok(Ok(events)) => {
                let duration_ms = start.elapsed().as_millis();
                let events_vec: Vec<Event> = events.into_iter().collect();
                
                tracing::info!(
                    cache_key = %cache_key,
                    duration_ms = duration_ms,
                    event_count = events_vec.len(),
                    source = "relay",
                    success = true,
                    "fetch_events_success"
                );
                
                if !events_vec.is_empty() {
                    let cache = self.cache.clone();
                    let cached = CachedEvents {
                        events: events_vec.clone(),
                        timestamp: std::time::Instant::now(),
                    };
                    tokio::spawn(async move {
                        cache.write().await.insert(cache_key, cached);
                    });
                    *self.relay_healthy.lock().await = true;
                }
                
                self.metrics.write().await.record_cache_miss(duration_ms, true);
                Ok(events_vec)
            }
            Ok(Err(e)) => {
                let duration_ms = start.elapsed().as_millis();
                
                tracing::warn!(
                    cache_key = %cache_key,
                    duration_ms = duration_ms,
                    error = %e,
                    source = "relay",
                    success = false,
                    "fetch_events_error"
                );
                
                self.metrics.write().await.record_cache_miss(duration_ms, false);
                *self.relay_healthy.lock().await = false;
                Err(format!("Fetch error: {}", e))
            }
            Err(_) => {
                let duration_ms = start.elapsed().as_millis();
                
                tracing::warn!(
                    cache_key = %cache_key,
                    duration_ms = duration_ms,
                    source = "relay",
                    success = false,
                    reason = "timeout",
                    "fetch_events_timeout"
                );
                
                self.metrics.write().await.record_cache_miss(duration_ms, false);
                *self.relay_healthy.lock().await = false;
                Err("Relay timeout".to_string())
            }
        }
    }

    // ==================== Tools ====================

    #[tool(description = "Search for job listings on Nostr. You can filter by company, skill, or employment type.")]
    pub async fn search_jobs(
        &self,
        Parameters(args): Parameters<SearchJobsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let clean_company = args.company.as_ref().map(|s| s.trim_matches('"').to_string());
        let clean_skill = args.skill.as_ref().map(|s| s.trim_matches('"').to_string());
        let clean_employment_type = args.employment_type.as_ref().map(|s| s.trim_matches('"').to_string());
        
        let filter = Self::build_filter(
            clean_company.as_deref(),
            clean_skill.as_deref(),
            clean_employment_type.as_deref(),
            args.limit,
        );

        let key = Self::cache_key(
            clean_company.as_deref(),
            clean_skill.as_deref(),
            clean_employment_type.as_deref(),
            args.limit,
        );

        // Check cache first
        {
            let start = std::time::Instant::now();
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&key) {
                let duration_ms = start.elapsed().as_millis();
                let is_fresh = cached.is_fresh(Duration::from_secs(60));
                
                tracing::info!(
                    cache_key = %key,
                    duration_ms = duration_ms,
                    event_count = cached.events.len(),
                    source = "cache",
                    is_fresh = is_fresh,
                    "cache_hit"
                );
                
                self.metrics.write().await.record_cache_hit(duration_ms);
                
                let mut results = format!("Found {} job listing(s){}:\n\n", 
                    cached.events.len(),
                    if is_fresh { " ⚡ [CACHED]" } else { " 📦 [CACHED - STALE]" }
                );
                for (i, event) in cached.events.iter().enumerate() {
                    results.push_str(&format!("{}. {}\n\n", i + 1, self.format_job_summary(event)));
                }
                return Ok(CallToolResult::success(vec![Content::text(results)]));
            } else {
                tracing::debug!(
                    cache_key = %key,
                    "cache_miss"
                );
            }
        }

        // Try fresh fetch
        match timeout(Duration::from_millis(2500), self.fetch_events_fast(filter, key.clone())).await {
            Ok(Ok(mut events)) => {
                events.retain(|event| {
                    let tags: Vec<_> = event.tags.iter().collect();
                    
                    let matches_company = if let Some(comp) = &clean_company {
                        tags.iter().any(|t| {
                            let slice = t.as_slice();
                            slice.len() >= 2 && slice[0] == "company" && 
                            slice[1].to_lowercase().contains(&comp.to_lowercase())
                        })
                    } else {
                        true
                    };
                    
                    let matches_skill = if let Some(sk) = &clean_skill {
                        tags.iter().any(|t| {
                            let slice = t.as_slice();
                            slice.len() >= 2 && slice[0] == "skill" && 
                            slice[1].to_lowercase().contains(&sk.to_lowercase())
                        })
                    } else {
                        true
                    };
                    
                    let matches_employment = if let Some(et) = &clean_employment_type {
                        tags.iter().any(|t| {
                            let slice = t.as_slice();
                            slice.len() >= 2 && slice[0] == "employment-type" && 
                            slice[1].to_lowercase().contains(&et.to_lowercase())
                        })
                    } else {
                        true
                    };
                    
                    matches_company && matches_skill && matches_employment
                });
                
                events.truncate(args.limit);
                
                if events.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "No job listings found matching your criteria.".to_string()
                    )]));
                }

                let mut results = format!("Found {} job listing(s) 🌐 [FRESH]:\n\n", events.len());
                for (i, event) in events.iter().enumerate() {
                    results.push_str(&format!("{}. {}\n\n", i + 1, self.format_job_summary(event)));
                }

                Ok(CallToolResult::success(vec![Content::text(results)]))
            }
            _ => {
                let healthy = *self.relay_healthy.lock().await;
                if healthy {
                    Ok(CallToolResult::success(vec![Content::text(
                        "⏳ Search in progress...\n\
                         Relays are responding but queries are slow.\n\
                         Please try again shortly."
                    )]))
                } else {
                    Ok(CallToolResult::success(vec![Content::text(
                        "🔄 Starting relay connection...\n\n\
                         The Nostr relays are initializing.\n\
                         Please try again in a moment.\n\n\
                         💡 Tip: Results will be cached once available."
                    )]))
                }
            }
        }
    }

    #[tool(description = "Get detailed information about a specific job listing by its Job ID or Event ID")]
    pub async fn get_job_details(
        &self,
        Parameters(args): Parameters<GetJobArgs>,
    ) -> Result<CallToolResult, McpError> {
        let key = format!("job:{}", args.job_id);

        // Check cache first - avoid relay request entirely if cached
        {
            let start = std::time::Instant::now();
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&key) {
                if let Some(event) = cached.events.first() {
                    let duration_ms = start.elapsed().as_millis();
                    self.metrics.write().await.record_cache_hit(duration_ms);
                    
                    let mut result = self.format_job_summary(event);
                    result.push_str("\n\n⚡ [CACHED]\n\n📄 Full Job Details:\n");
                    result.push_str(&event.content);
                    return Ok(CallToolResult::success(vec![Content::text(result)]));
                }
            }
        }

        // Not in cache, fetch from relays
        let filter = if let Ok(event_id) = EventId::from_hex(&args.job_id) {
            Filter::new().id(event_id)
        } else {
            Filter::new()
                .kind(Kind::from(9993u16))
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::J),
                    args.job_id.clone()
                )
        };

        match timeout(Duration::from_millis(2500), self.fetch_events_fast(filter, key)).await {
            Ok(Ok(events)) => {
                if events.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("No job found with ID: {}", args.job_id)
                    )]));
                }

                let event = events.first().unwrap();
                let mut result = self.format_job_summary(event);
                result.push_str("\n\n🌐 [FRESH]\n\n📄 Full Job Details:\n");
                result.push_str(&event.content);

                Ok(CallToolResult::success(vec![Content::text(result)]))
            }
            _ => {
                Ok(CallToolResult::success(vec![Content::text(
                    "⚠️ Unable to fetch job details. Relays are currently unresponsive.\n\
                     Please try again shortly."
                )]))
            }
        }
    }

    #[tool(description = "Get comprehensive performance metrics showing cache effectiveness")]
    pub async fn get_performance_metrics(&self) -> Result<CallToolResult, McpError> {
        let metrics = self.metrics.read().await;
        let report = metrics.format_report();
        
        // Log metrics snapshot for monitoring systems
        tracing::info!(
            total_requests = metrics.total_requests,
            cache_hits = metrics.cache_hits,
            cache_misses = metrics.cache_misses,
            cache_hit_rate = metrics.cache_hit_rate(),
            relay_fetches = metrics.relay_fetches,
            failed_fetches = metrics.failed_fetches,
            avg_cache_time_ms = metrics.avg_cache_time(),
            avg_fetch_time_ms = metrics.avg_fetch_time(),
            total_time_saved_ms = metrics.time_saved_ms(),
            "performance_metrics_snapshot"
        );
        
        Ok(CallToolResult::success(vec![Content::text(report)]))
    }

    #[tool(description = "Reset performance metrics (useful for testing)")]
    pub async fn reset_metrics(&self) -> Result<CallToolResult, McpError> {
        let old_metrics = self.metrics.read().await.clone();
        *self.metrics.write().await = PerformanceMetrics::default();
        
        tracing::info!(
            old_total_requests = old_metrics.total_requests,
            old_cache_hit_rate = old_metrics.cache_hit_rate(),
            old_time_saved_ms = old_metrics.time_saved_ms(),
            "metrics_reset"
        );
        
        Ok(CallToolResult::success(vec![Content::text(
            "✅ Performance metrics have been reset.".to_string()
        )]))
    }

    #[tool(description = "Clear the cache and show before/after metrics")]
    pub async fn clear_cache(&self) -> Result<CallToolResult, McpError> {
        let metrics_before = self.metrics.read().await.clone();
        let cache_size = self.cache.read().await.len();
        self.cache.write().await.clear();
        
        tracing::warn!(
            cache_entries_cleared = cache_size,
            cache_hits_before = metrics_before.cache_hits,
            cache_hit_rate_before = metrics_before.cache_hit_rate(),
            "cache_cleared"
        );
        
        let report = format!(
            "🗑️  Cache Cleared Successfully\n\n\
            Cache statistics before clear:\n\
            • Total cached queries: {}\n\
            • Cache hits: {}\n\
            • Cache hit rate: {:.1}%\n\n\
            ⚠️  Next queries will fetch fresh data from relays.\n\
            💡 Use get_performance_metrics to track the impact.",
            cache_size,
            metrics_before.cache_hits,
            metrics_before.cache_hit_rate()
        );
        
        Ok(CallToolResult::success(vec![Content::text(report)]))
    }

    #[tool(description = "List all connected Nostr relays")]
    pub async fn list_relays(&self) -> Result<CallToolResult, McpError> {
        let relays_text = format!(
            "Connected to {} relay(s):\n{}",
            self.relays.len(),
            self.relays.iter().map(|r| format!("  • {}", r)).collect::<Vec<_>>().join("\n")
        );
        
        Ok(CallToolResult::success(vec![Content::text(relays_text)]))
    }

    #[tool(description = "Get statistics about job listings on Nostr")]
    pub async fn get_stats(&self) -> Result<CallToolResult, McpError> {
        let filter = Self::build_filter(None, None, None, 100);
        let key = "stats:all".to_string();

        {
            let start = std::time::Instant::now();
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&key) {
                let duration_ms = start.elapsed().as_millis();
                self.metrics.write().await.record_cache_hit(duration_ms);
                
                let events = &cached.events;
                let (employment_counts, company_counts, skill_counts) = 
                    Self::analyze_events(events);

                let stats = format!(
                    "📊 Nostr Job Listings Statistics{}\n\n\
                    Total Listings: {}\n\n\
                    Employment Types:\n{}\n\n\
                    Top Companies:\n{}\n\n\
                    Top Skills:\n{}",
                    if cached.is_fresh(Duration::from_secs(120)) { " ⚡ [CACHED]" } else { " 📦 [CACHED - STALE]" },
                    events.len(),
                    format_top_items(&employment_counts, 5),
                    format_top_items(&company_counts, 5),
                    format_top_items(&skill_counts, 10)
                );
                return Ok(CallToolResult::success(vec![Content::text(stats)]));
            }
        }

        match timeout(Duration::from_millis(2500), self.fetch_events_fast(filter, key)).await {
            Ok(Ok(events)) => {
                let (employment_counts, company_counts, skill_counts) = 
                    Self::analyze_events(&events);

                let stats = format!(
                    "📊 Nostr Job Listings Statistics 🌐 [FRESH]\n\n\
                    Total Listings: {}\n\n\
                    Employment Types:\n{}\n\n\
                    Top Companies:\n{}\n\n\
                    Top Skills:\n{}",
                    events.len(),
                    format_top_items(&employment_counts, 5),
                    format_top_items(&company_counts, 5),
                    format_top_items(&skill_counts, 10)
                );

                Ok(CallToolResult::success(vec![Content::text(stats)]))
            }
            _ => Ok(CallToolResult::success(vec![Content::text(
                "📊 Statistics unavailable\n\nRelays are currently unresponsive.\n\
                 Try again shortly for cached results."
            )]))
        }
    }

    fn analyze_events(events: &[Event]) -> (HashMap<String, usize>, HashMap<String, usize>, HashMap<String, usize>) {
        let mut employment_counts = HashMap::new();
        let mut company_counts = HashMap::new();
        let mut skill_counts = HashMap::new();

        for event in events.iter() {
            let tags: Vec<_> = event.tags.iter().collect();
            
            for tag in &tags {
                let slice = tag.as_slice();
                if slice.len() >= 2 {
                    match slice[0].as_str() {
                        "employment-type" => {
                            *employment_counts.entry(slice[1].to_string()).or_insert(0) += 1;
                        }
                        "company" => {
                            *company_counts.entry(slice[1].to_string()).or_insert(0) += 1;
                        }
                        "skill" => {
                            *skill_counts.entry(slice[1].to_string()).or_insert(0) += 1;
                        }
                        _ => {}
                    }
                }
            }
        }

        (employment_counts, company_counts, skill_counts)
    }
}

fn format_top_items(map: &HashMap<String, usize>, limit: usize) -> String {
    let mut items: Vec<_> = map.iter().collect();
    items.sort_by(|a, b| b.1.cmp(a.1));
    
    if items.is_empty() {
        return "  (none)".to_string();
    }
    
    items.iter()
        .take(limit)
        .map(|(k, v)| format!("  • {}: {}", k, v))
        .collect::<Vec<_>>()
        .join("\n")
}

// ==================== Prompts ====================

#[prompt_router]
impl NostrJobsServer {
    #[prompt(name = "job_search_assistant")]
    pub async fn job_search_assistant(
        &self,
        Parameters(args): Parameters<JobAnalysisArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let skills_text = args.skills
            .map(|s| format!("Required skills: {}", s.join(", ")))
            .unwrap_or_default();

        let messages = vec![
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                "I'm your Nostr job search assistant. I'll help you find relevant job listings on the decentralized Nostr network.",
            ),
            PromptMessage::new_text(
                PromptMessageRole::User,
                format!(
                    "Search Query: {}\n{}\n\nPlease help me find relevant job listings and provide recommendations based on this query.",
                    args.query,
                    skills_text
                ),
            ),
        ];

        Ok(GetPromptResult {
            description: Some(format!("Job search assistance for: {}", args.query)),
            messages,
        })
    }

    #[prompt(name = "analyze_job_market")]
    pub async fn analyze_job_market(
        &self,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let messages = vec![
            PromptMessage::new_text(
                PromptMessageRole::Assistant,
                "I'll analyze the current job market on Nostr and provide insights.",
            ),
            PromptMessage::new_text(
                PromptMessageRole::User,
                "Please analyze the job listings available on Nostr. What are the trending skills? Which companies are hiring? What's the salary range for different positions?",
            ),
        ];

        Ok(GetPromptResult {
            description: Some("Analysis of the Nostr job market".to_string()),
            messages,
        })
    }
}

// ==================== MCP Server Handler ====================

#[tool_handler]
#[prompt_handler]
impl ServerHandler for NostrJobsServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder()
                .enable_prompts()
                .enable_resources()
                .enable_tools()
                .build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Nostr Jobs MCP Server - Access decentralized job listings from the Nostr network.\n\n\
                Tools:\n\
                • search_jobs - Search for jobs by company, skill, or employment type\n\
                • get_job_details - Get detailed information about a specific job\n\
                • get_performance_metrics - View cache performance and efficiency gains\n\
                • clear_cache - Clear cache and see impact on performance\n\
                • reset_metrics - Reset performance tracking\n\
                • list_relays - Show connected Nostr relays\n\
                • get_stats - Get statistics about job listings\n\n\
                Prompts:\n\
                • job_search_assistant - Get help searching for jobs\n\
                • analyze_job_market - Analyze current job market trends\n\n\
                Resources:\n\
                • jobs://latest - Latest job listings\n\
                • jobs://stats - Job market statistics\n\n\
                Performance Features:\n\
                • Automatic caching with 60s TTL\n\
                • Detailed metrics tracking\n\
                • Cache hit/miss analytics\n\
                • Response time comparison".to_string()
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                RawResource::new("jobs://latest", "Latest Job Listings".to_string()).no_annotation(),
                RawResource::new("jobs://stats", "Job Market Statistics".to_string()).no_annotation(),
            ],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        ReadResourceRequestParam { uri }: ReadResourceRequestParam,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match uri.as_str() {
            "jobs://latest" => {
                let filter = Self::build_filter(None, None, None, 20);
                
                match timeout(Duration::from_millis(2500), self.fetch_events_fast(filter, "latest:20".to_string())).await {
                    Ok(Ok(events)) => {
                        let content = if events.is_empty() {
                            "No job listings found.".to_string()
                        } else {
                            let mut result = format!("Latest {} Job Listings:\n\n", events.len());
                            for (i, event) in events.iter().enumerate() {
                                result.push_str(&format!("{}. {}\n\n", i + 1, self.format_job_summary(event)));
                            }
                            result
                        };

                        Ok(ReadResourceResult {
                            contents: vec![ResourceContents::text(&content, uri)],
                        })
                    }
                    _ => Err(McpError::internal_error(
                        "Failed to read resource",
                        Some(json!({"uri": uri}))
                    ))
                }
            }
            "jobs://stats" => {
                match self.get_stats().await {
                    Ok(stats_result) => {
                        let mut content_text = String::new();
                        for c in &stats_result.content {
                            if let RawContent::Text(text_content) = &c.raw {
                                content_text.push_str(&text_content.text);
                                content_text.push('\n');
                            }
                        }

                        Ok(ReadResourceResult {
                            contents: vec![ResourceContents::text(&content_text, uri)],
                        })
                    }
                    Err(e) => Err(e),
                }
            }
            _ => Err(McpError::resource_not_found(
                "Resource not found",
                Some(json!({ "uri": uri })),
            )),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParam>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            next_cursor: None,
            resource_templates: Vec::new(),
        })
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        if let Some(http_request_part) = context.extensions.get::<http::request::Parts>() {
            let initialize_headers = &http_request_part.headers;
            let initialize_uri = &http_request_part.uri;
            tracing::info!(?initialize_headers, %initialize_uri, "initialize from http server");
        }
        Ok(self.get_info())
    }
}
