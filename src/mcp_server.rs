// src/mcp_server.rs
// Standalone MCP Server for Nostr Job Listings (Kind 9993)

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
            "wss://nostr-pub.wellorder.net".to_string(),
            "wss://nostr.wine".to_string(),
        ];

        // Add all relays first (non-blocking)
        for relay in &relays {
            match client.add_relay(relay).await {
                Ok(_) => tracing::info!("Added relay: {}", relay),
                Err(e) => tracing::warn!("Failed to add relay {}: {}", relay, e),
            }
        }
        
        // Spawn background connection task
        let client_clone = client.clone();
        tokio::spawn(async move {
            match timeout(Duration::from_secs(15), client_clone.connect()).await {
                Ok(_) => tracing::info!("Connected to relays"),
                Err(_) => tracing::warn!("Relay connection timeout"),
            }
        });

        let server = Self {
            client: Arc::new(Mutex::new(client)),
            relays,
            cache: Arc::new(RwLock::new(HashMap::new())),
            relay_healthy: Arc::new(Mutex::new(false)),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        };

        // Spawn background health check
        let server_clone = server.clone();
        tokio::spawn(async move {
            server_clone.health_check_loop().await;
        });

        server
    }

    // Background health check loop
    async fn health_check_loop(&self) {
        loop {
            tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
            
            let client = self.client.lock().await;
            let filter = Filter::new().limit(1);
            
            match timeout(Duration::from_secs(5), client.fetch_events(filter, Duration::from_secs(3))).await {
                Ok(Ok(_)) => {
                    *self.relay_healthy.lock().await = true;
                    tracing::debug!("Relays healthy");
                }
                _ => {
                    *self.relay_healthy.lock().await = false;
                    tracing::warn!("Relays unhealthy");
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
    // Fetch all kind 9993 events for in-memory filtering
    // Use a larger limit since we filter after fetching
    Filter::new()
        .kind(Kind::from(9993u16))
        .limit(100) // Fetch up to 100 to filter from
}


    fn cache_key(company: Option<&str>, skill: Option<&str>, employment_type: Option<&str>, limit: usize) -> String {
        format!("{}:{}:{}:{}", 
            company.unwrap_or("*"),
            skill.unwrap_or("*"),
            employment_type.unwrap_or("*"),
            limit
        )
    }

    // Fast fetch with aggressive timeout and cache fallback
    async fn fetch_events_fast(
        &self,
        filter: Filter,
        cache_key: String,
    ) -> Result<Vec<Event>, String> {
        let client = self.client.lock().await;
        
        // Use very short timeout - fail fast to avoid MCP timeout
        match timeout(RELAY_FETCH_TIMEOUT, client.fetch_events(filter, Duration::from_millis(1500))).await {
            Ok(Ok(events)) => {
                let events_vec: Vec<Event> = events.into_iter().collect();
                if !events_vec.is_empty() {
                    tracing::debug!("Fetched {} events", events_vec.len());
                    // Update cache asynchronously
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
                Ok(events_vec)
            }
            Ok(Err(e)) => {
                tracing::warn!("Fetch error: {}", e);
                *self.relay_healthy.lock().await = false;
                Err(format!("Fetch error: {}", e))
            }
            Err(_) => {
                tracing::warn!("Fetch timeout");
                *self.relay_healthy.lock().await = false;
                Err("Relay timeout".to_string())
            }
        }
    }

    // ==================== Tools ====================

// Replace the entire search_jobs method in src/mcp_server.rs with this:

#[tool(description = "Search for job listings on Nostr. You can filter by company, skill, or employment type.")]
pub async fn search_jobs(
    &self,
    Parameters(args): Parameters<SearchJobsArgs>,
) -> Result<CallToolResult, McpError> {
    // Strip any surrounding quotes from string parameters (MCP Inspector bug workaround)
    let clean_company = args.company.as_ref().map(|s| s.trim_matches('"').to_string());
    let clean_skill = args.skill.as_ref().map(|s| s.trim_matches('"').to_string());
    let clean_employment_type = args.employment_type.as_ref().map(|s| s.trim_matches('"').to_string());
    
    // Log parameters for debugging (visible in server logs only)
    tracing::debug!(
        "search_jobs params - company: {:?}, skill: {:?}, employment_type: {:?}, limit: {}",
        clean_company, clean_skill, clean_employment_type, args.limit
    );
    
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

    // Check cache first - use any cached data
    {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(&key) {
            tracing::info!("Cache hit for search_jobs (age: {:?})", cached.timestamp.elapsed());
            
            let mut results = format!("Found {} job listing(s){}:\n\n", 
                cached.events.len(),
                if cached.is_fresh(Duration::from_secs(60)) { "" } else { " (cached)" }
            );
            for (i, event) in cached.events.iter().enumerate() {
                results.push_str(&format!("{}. {}\n\n", i + 1, self.format_job_summary(event)));
            }
            return Ok(CallToolResult::success(vec![Content::text(results)]));
        }
    }

    // Try fresh fetch with very short timeout
    match timeout(Duration::from_millis(2500), self.fetch_events_fast(filter, key.clone())).await {
        Ok(Ok(mut events)) => {
            tracing::debug!("Fetched {} events before filtering", events.len());
            tracing::debug!("Filter params - company: {:?}, skill: {:?}, employment_type: {:?}", 
                clean_company, clean_skill, clean_employment_type);
            
            // Filter in-memory since relay filtering may not work
            events.retain(|event| {
                let tags: Vec<_> = event.tags.iter().collect();
                
                let matches_company = if let Some(comp) = &clean_company {
                    let result = tags.iter().any(|t| {
                        let slice = t.as_slice();
                        slice.len() >= 2 && slice[0] == "company" && 
                        slice[1].to_lowercase().contains(&comp.to_lowercase())
                    });
                    tracing::debug!("Company filter '{}': {}", comp, result);
                    result
                } else {
                    true
                };
                
                let matches_skill = if let Some(sk) = &clean_skill {
                    let result = tags.iter().any(|t| {
                        let slice = t.as_slice();
                        let is_skill_tag = slice.len() >= 2 && slice[0] == "skill";
                        let matches = is_skill_tag && slice[1].to_lowercase().contains(&sk.to_lowercase());
                        if is_skill_tag {
                            tracing::debug!("Checking skill tag '{}' against '{}': {}", slice[1], sk, matches);
                        }
                        matches
                    });
                    tracing::debug!("Skill filter '{}': {}", sk, result);
                    result
                } else {
                    true
                };
                
                let matches_employment = if let Some(et) = &clean_employment_type {
                    let result = tags.iter().any(|t| {
                        let slice = t.as_slice();
                        slice.len() >= 2 && slice[0] == "employment-type" && 
                        slice[1].to_lowercase().contains(&et.to_lowercase())
                    });
                    tracing::debug!("Employment type filter '{}': {}", et, result);
                    result
                } else {
                    true
                };
                
                let overall_match = matches_company && matches_skill && matches_employment;
                tracing::debug!("Event {} overall match: {}", event.id.to_hex(), overall_match);
                overall_match
            });
            
            tracing::debug!("After filtering: {} events", events.len());
            
            // Limit to requested amount after filtering
            events.truncate(args.limit);
            
            if events.is_empty() {
                return Ok(CallToolResult::success(vec![Content::text(
                    "No job listings found matching your criteria.".to_string()
                )]));
            }

            let mut results = format!("Found {} job listing(s):\n\n", events.len());
            for (i, event) in events.iter().enumerate() {
                results.push_str(&format!("{}. {}\n\n", i + 1, self.format_job_summary(event)));
            }

            Ok(CallToolResult::success(vec![Content::text(results)]))
        }
        _ => {
            // Return helpful message about relay issues
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

        let key = format!("job:{}", args.job_id);

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&key)
                && let Some(event) = cached.events.first() {
                    let mut result = self.format_job_summary(event);
                    result.push_str(&format!("\n\n📄 Full Job Details:\n{}", event.content));
                    return Ok(CallToolResult::success(vec![Content::text(result)]));
                }
        }

        match timeout(Duration::from_millis(2500), self.fetch_events_fast(filter, key)).await {
            Ok(Ok(events)) => {
                if events.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        format!("No job found with ID: {}", args.job_id)
                    )]));
                }

                let event = events.first().unwrap();
                let mut result = self.format_job_summary(event);
                result.push_str(&format!("\n\n📄 Full Job Details:\n{}", event.content));

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

        // Check cache
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.get(&key) {
                let events = &cached.events;
                let (employment_counts, company_counts, skill_counts) = 
                    Self::analyze_events(events);

                let stats = format!(
                    "📊 Nostr Job Listings Statistics{}\n\n\
                    Total Listings: {}\n\n\
                    Employment Types:\n{}\n\n\
                    Top Companies:\n{}\n\n\
                    Top Skills:\n{}",
                    if cached.is_fresh(Duration::from_secs(120)) { "" } else { " (cached)" },
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
                    "📊 Nostr Job Listings Statistics\n\n\
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

// Helper function to format top items
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
                • list_relays - Show connected Nostr relays\n\
                • get_stats - Get statistics about job listings\n\n\
                Prompts:\n\
                • job_search_assistant - Get help searching for jobs\n\
                • analyze_job_market - Analyze current job market trends\n\n\
                Resources:\n\
                • jobs://latest - Latest job listings\n\
                • jobs://stats - Job market statistics".to_string()
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
