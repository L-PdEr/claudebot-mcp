//! MCP Tool Registry
//!
//! Defines and executes MCP tools for Claude Code integration.
//! Provides 20+ tools across memory, graph, circle, and metrics.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::cache::ResponseCache;
use crate::circle::{Circle, PipelineMode};
use crate::claude::ClaudeClient;
use crate::config::Config;
use crate::graph::GraphStore;
use crate::memory::MemoryStore;
use crate::metrics::MetricsCollector;
use crate::router::TaskRouter;

/// Tool definition for MCP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Tool registry with all components
pub struct ToolRegistry {
    config: Arc<Config>,
    router: TaskRouter,
    cache: ResponseCache,
    memory: MemoryStore,
    graph: GraphStore,
    claude: ClaudeClient,
    circle: Circle,
    metrics: Arc<MetricsCollector>,
}

impl ToolRegistry {
    /// Create new tool registry
    pub async fn new(config: Arc<Config>) -> Result<Self> {
        let router = TaskRouter::new(config.ollama_url.clone());
        let cache = ResponseCache::new(1000, config.cache_ttl_secs, config.cache_enabled);
        let memory = MemoryStore::open(&config.db_path)?;

        // Open separate connection for graph (same db)
        let graph_conn = Connection::open(&config.db_path)?;
        let graph = GraphStore::new(graph_conn)?;

        let claude = ClaudeClient::new(config.anthropic_api_key.as_deref());
        let circle = Circle::new(claude.clone());
        let metrics = Arc::new(MetricsCollector::new(10000));

        Ok(Self {
            config,
            router,
            cache,
            memory,
            graph,
            claude,
            circle,
            metrics,
        })
    }

    /// List all tool definitions
    pub fn list_definitions(&self) -> Vec<ToolDefinition> {
        vec![
            // ========== Router Tools ==========
            ToolDefinition {
                name: "router_classify".to_string(),
                description: "Classify a message and determine optimal model (Haiku/Sonnet/Opus)"
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "The message to classify"
                        }
                    },
                    "required": ["message"]
                }),
            },
            // ========== Memory Tools ==========
            ToolDefinition {
                name: "memory_learn".to_string(),
                description: "Store a fact in memory with category and confidence".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The fact to remember"
                        },
                        "category": {
                            "type": "string",
                            "description": "Category: facts, preferences, decisions, lessons",
                            "default": "facts"
                        },
                        "confidence": {
                            "type": "number",
                            "description": "Confidence score 0-1",
                            "default": 0.8
                        }
                    },
                    "required": ["content"]
                }),
            },
            ToolDefinition {
                name: "memory_search".to_string(),
                description: "Search memories using full-text search".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results",
                            "default": 5
                        }
                    },
                    "required": ["query"]
                }),
            },
            ToolDefinition {
                name: "memory_recall".to_string(),
                description: "Get recent memories, optionally filtered by category".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "category": {
                            "type": "string",
                            "description": "Optional category filter"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results",
                            "default": 10
                        }
                    }
                }),
            },
            ToolDefinition {
                name: "memory_forget".to_string(),
                description: "Delete a memory by ID".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Memory ID to delete"
                        }
                    },
                    "required": ["id"]
                }),
            },
            ToolDefinition {
                name: "memory_stats".to_string(),
                description: "Get memory statistics".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            // ========== Graph Tools (E3) ==========
            ToolDefinition {
                name: "graph_add_entity".to_string(),
                description: "Add an entity to the knowledge graph".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "entity_type": {
                            "type": "string",
                            "description": "Type: project, person, technology, preference, concept, decision, file"
                        },
                        "name": {
                            "type": "string",
                            "description": "Entity name"
                        },
                        "attributes": {
                            "type": "object",
                            "description": "Optional attributes"
                        }
                    },
                    "required": ["entity_type", "name"]
                }),
            },
            ToolDefinition {
                name: "graph_add_relation".to_string(),
                description: "Add a relationship between two entities".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "source_id": {
                            "type": "string",
                            "description": "Source entity ID"
                        },
                        "target_id": {
                            "type": "string",
                            "description": "Target entity ID"
                        },
                        "relation_type": {
                            "type": "string",
                            "description": "Type: works_on, prefers, knows, uses, related_to, depends_on, created_by, contains"
                        },
                        "weight": {
                            "type": "number",
                            "description": "Relationship strength 0-2",
                            "default": 1.0
                        }
                    },
                    "required": ["source_id", "target_id", "relation_type"]
                }),
            },
            ToolDefinition {
                name: "graph_find_entity".to_string(),
                description: "Find an entity by name".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Entity name to search"
                        }
                    },
                    "required": ["name"]
                }),
            },
            ToolDefinition {
                name: "graph_traverse".to_string(),
                description: "Traverse graph from an entity (1-2 hops)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "entity_id": {
                            "type": "string",
                            "description": "Starting entity ID"
                        },
                        "max_hops": {
                            "type": "integer",
                            "description": "Maximum hops (1-2)",
                            "default": 2
                        }
                    },
                    "required": ["entity_id"]
                }),
            },
            ToolDefinition {
                name: "graph_entities_by_type".to_string(),
                description: "Get entities of a specific type".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "entity_type": {
                            "type": "string",
                            "description": "Entity type to list"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results",
                            "default": 20
                        }
                    },
                    "required": ["entity_type"]
                }),
            },
            ToolDefinition {
                name: "graph_extract".to_string(),
                description: "Extract entities from text and store them".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Text to extract entities from"
                        },
                        "memory_id": {
                            "type": "string",
                            "description": "Optional memory ID to link entities to"
                        }
                    },
                    "required": ["text"]
                }),
            },
            ToolDefinition {
                name: "graph_stats".to_string(),
                description: "Get graph statistics".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            // ========== Cache Tools ==========
            ToolDefinition {
                name: "cache_stats".to_string(),
                description: "Get response cache statistics".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "cache_clear".to_string(),
                description: "Clear the response cache".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            // ========== Circle Tools (E5) ==========
            ToolDefinition {
                name: "circle_run".to_string(),
                description: "Run the Development Circle pipeline (5 phases)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "feature": {
                            "type": "string",
                            "description": "Feature description to implement"
                        },
                        "context": {
                            "type": "string",
                            "description": "Current code context",
                            "default": ""
                        },
                        "mode": {
                            "type": "string",
                            "description": "Mode: full, review_only, quick_fix, security_only",
                            "default": "full"
                        }
                    },
                    "required": ["feature"]
                }),
            },
            // ========== Metrics Tools (E6) ==========
            ToolDefinition {
                name: "metrics_quick".to_string(),
                description: "Get quick metrics summary".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "metrics_cost".to_string(),
                description: "Get cost breakdown (today, week, month)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "metrics_latency".to_string(),
                description: "Get latency percentiles (p50, p90, p99)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "metrics_export".to_string(),
                description: "Export all metrics as JSON".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            ToolDefinition {
                name: "metrics_reset".to_string(),
                description: "Reset all metrics".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
            // ========== Claude Tools ==========
            ToolDefinition {
                name: "claude_complete".to_string(),
                description: "Send a prompt to Claude API with prompt caching".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The prompt to send"
                        },
                        "model": {
                            "type": "string",
                            "description": "Model: haiku, sonnet, opus",
                            "default": "opus"
                        },
                        "max_tokens": {
                            "type": "integer",
                            "description": "Max response tokens",
                            "default": 4096
                        }
                    },
                    "required": ["prompt"]
                }),
            },
        ]
    }

    /// Call a tool by name
    pub async fn call(&self, name: &str, args: serde_json::Value) -> Result<String> {
        info!("Tool call: {} with args: {}", name, args);
        let start = std::time::Instant::now();

        let result = match name {
            // ========== Router ==========
            "router_classify" => {
                let message = args["message"].as_str().unwrap_or("");
                let result = self.router.route(message);
                Ok(json!({
                    "target": result.target.as_str(),
                    "model": result.model.as_str(),
                    "reasoning": result.reasoning,
                    "confidence": result.confidence
                })
                .to_string())
            }

            // ========== Memory ==========
            "memory_learn" => {
                let content = args["content"].as_str().unwrap_or("");
                let category = args["category"].as_str().unwrap_or("facts");
                let confidence = args["confidence"].as_f64().unwrap_or(0.8);
                let id = self.memory.learn(content, category, "mcp", confidence)?;

                // Auto-extract entities
                let entities = GraphStore::extract_entities_simple(content);
                if !entities.is_empty() {
                    self.graph.store_extracted(&id, &entities, &[])?;
                }

                Ok(json!({
                    "id": id,
                    "status": "learned",
                    "entities_extracted": entities.len()
                })
                .to_string())
            }
            "memory_search" => {
                let query = args["query"].as_str().unwrap_or("");
                let limit = args["limit"].as_u64().unwrap_or(5) as usize;
                let results = self.memory.search(query, limit)?;
                let entries: Vec<_> = results
                    .iter()
                    .map(|r| {
                        json!({
                            "id": r.entry.id,
                            "content": r.entry.content,
                            "category": r.entry.category,
                            "score": r.score
                        })
                    })
                    .collect();
                Ok(json!({ "results": entries }).to_string())
            }
            "memory_recall" => {
                let limit = args["limit"].as_u64().unwrap_or(10) as usize;
                let entries = if let Some(category) = args["category"].as_str() {
                    self.memory.get_by_category(category, limit)?
                } else {
                    self.memory.get_recent(limit)?
                };
                let results: Vec<_> = entries
                    .iter()
                    .map(|e| {
                        json!({
                            "id": e.id,
                            "content": e.content,
                            "category": e.category,
                            "confidence": e.confidence
                        })
                    })
                    .collect();
                Ok(json!({ "memories": results }).to_string())
            }
            "memory_forget" => {
                let id = args["id"].as_str().unwrap_or("");
                let deleted = self.memory.forget(id)?;
                Ok(json!({ "deleted": deleted }).to_string())
            }
            "memory_stats" => {
                let stats = self.memory.stats()?;
                Ok(json!({
                    "total": stats.total_entries,
                    "by_category": stats.by_category
                })
                .to_string())
            }

            // ========== Graph (E3) ==========
            "graph_add_entity" => {
                let entity_type = args["entity_type"].as_str().unwrap_or("concept");
                let name = args["name"].as_str().unwrap_or("");
                let attributes = args.get("attributes").cloned();
                let id = self.graph.add_entity(entity_type, name, attributes)?;
                Ok(json!({ "id": id, "status": "created" }).to_string())
            }
            "graph_add_relation" => {
                let source_id = args["source_id"].as_str().unwrap_or("");
                let target_id = args["target_id"].as_str().unwrap_or("");
                let relation_type = args["relation_type"].as_str().unwrap_or("related_to");
                let weight = args["weight"].as_f64();
                let id = self
                    .graph
                    .add_relation(source_id, target_id, relation_type, weight)?;
                Ok(json!({ "id": id, "status": "created" }).to_string())
            }
            "graph_find_entity" => {
                let name = args["name"].as_str().unwrap_or("");
                let entity = self.graph.find_entity(name)?;
                match entity {
                    Some(e) => Ok(json!({
                        "found": true,
                        "entity": {
                            "id": e.id,
                            "type": e.entity_type,
                            "name": e.name,
                            "attributes": e.attributes
                        }
                    })
                    .to_string()),
                    None => Ok(json!({ "found": false }).to_string()),
                }
            }
            "graph_traverse" => {
                let entity_id = args["entity_id"].as_str().unwrap_or("");
                let max_hops = args["max_hops"].as_u64().unwrap_or(2) as usize;
                let results = self.graph.traverse(entity_id, max_hops.min(2))?;
                let nodes: Vec<_> = results
                    .iter()
                    .map(|r| {
                        json!({
                            "entity": {
                                "id": r.entity.id,
                                "type": r.entity.entity_type,
                                "name": r.entity.name
                            },
                            "score": r.score,
                            "path": r.path,
                            "relations": r.relations.iter().map(|rel| {
                                json!({
                                    "type": rel.relation_type,
                                    "weight": rel.weight
                                })
                            }).collect::<Vec<_>>()
                        })
                    })
                    .collect();
                Ok(json!({ "nodes": nodes }).to_string())
            }
            "graph_entities_by_type" => {
                let entity_type = args["entity_type"].as_str().unwrap_or("");
                let limit = args["limit"].as_u64().unwrap_or(20) as usize;
                let entities = self.graph.get_by_type(entity_type, limit)?;
                let results: Vec<_> = entities
                    .iter()
                    .map(|e| {
                        json!({
                            "id": e.id,
                            "name": e.name,
                            "attributes": e.attributes
                        })
                    })
                    .collect();
                Ok(json!({ "entities": results }).to_string())
            }
            "graph_extract" => {
                let text = args["text"].as_str().unwrap_or("");
                let memory_id = args["memory_id"].as_str().unwrap_or("manual");
                let entities = GraphStore::extract_entities_simple(text);
                self.graph.store_extracted(memory_id, &entities, &[])?;
                Ok(json!({
                    "extracted": entities.len(),
                    "entities": entities.iter().map(|e| json!({
                        "type": e.entity_type,
                        "name": e.name
                    })).collect::<Vec<_>>()
                })
                .to_string())
            }
            "graph_stats" => {
                let stats = self.graph.stats()?;
                Ok(json!({
                    "entities": stats.entity_count,
                    "relations": stats.relation_count,
                    "by_type": stats.by_type
                })
                .to_string())
            }

            // ========== Cache ==========
            "cache_stats" => {
                let stats = self.cache.stats();
                Ok(json!({
                    "entries": stats.entries,
                    "hits": stats.hits,
                    "misses": stats.misses,
                    "hit_rate_percent": stats.hit_rate_percent
                })
                .to_string())
            }
            "cache_clear" => {
                self.cache.clear().await;
                Ok(json!({ "status": "cleared" }).to_string())
            }

            // ========== Circle (E5) ==========
            "circle_run" => {
                let feature = args["feature"].as_str().unwrap_or("");
                let context = args["context"].as_str().unwrap_or("");
                let mode_str = args["mode"].as_str().unwrap_or("full");

                let mode = match mode_str {
                    "review_only" => PipelineMode::ReviewOnly,
                    "quick_fix" => PipelineMode::QuickFix,
                    "security_only" => PipelineMode::SecurityOnly,
                    _ => PipelineMode::Full,
                };

                let result = self.circle.run(feature, context, mode).await?;
                let summary = Circle::summarize(&result);

                Ok(json!({
                    "success": result.success,
                    "revisions": result.revisions,
                    "blocked_at": result.blocked_at,
                    "duration_ms": result.total_duration_ms,
                    "phases": result.phases.len(),
                    "summary": summary
                })
                .to_string())
            }

            // ========== Metrics (E6) ==========
            "metrics_quick" => {
                let stats = self.metrics.quick_stats();
                Ok(json!({
                    "total_requests": stats.total_requests,
                    "total_cost_usd": stats.total_cost_usd,
                    "cache_hit_rate": stats.cache_hit_rate
                })
                .to_string())
            }
            "metrics_cost" => {
                let cost = self.metrics.cost_breakdown();
                Ok(json!({
                    "today_usd": cost.today_usd,
                    "this_week_usd": cost.this_week_usd,
                    "this_month_usd": cost.this_month_usd,
                    "by_model": cost.by_model,
                    "savings_from_cache_usd": cost.savings_from_cache_usd
                })
                .to_string())
            }
            "metrics_latency" => {
                let latency = self.metrics.latency_stats();
                Ok(json!({
                    "p50_ms": latency.p50_ms,
                    "p90_ms": latency.p90_ms,
                    "p99_ms": latency.p99_ms,
                    "min_ms": latency.min_ms,
                    "max_ms": latency.max_ms
                })
                .to_string())
            }
            "metrics_export" => Ok(self.metrics.export_json()),
            "metrics_reset" => {
                self.metrics.reset();
                Ok(json!({ "status": "reset" }).to_string())
            }

            // ========== Claude ==========
            "claude_complete" => {
                let prompt = args["prompt"].as_str().unwrap_or("");
                let model = args["model"].as_str().unwrap_or(&self.config.default_model);
                let max_tokens = args["max_tokens"].as_u64().unwrap_or(4096) as usize;

                // Static context (cached by Anthropic)
                let static_context = include_str!("../static_context.txt");

                let result = self
                    .claude
                    .complete(prompt, static_context, None, max_tokens, model)
                    .await?;

                // Record metrics
                self.metrics.record(
                    &result.model,
                    result.input_tokens,
                    result.output_tokens,
                    result.cache_read_tokens,
                    start.elapsed(),
                    result.cache_read_tokens > 0,
                    Some("claude_complete"),
                );

                Ok(json!({
                    "content": result.content,
                    "model": result.model,
                    "input_tokens": result.input_tokens,
                    "output_tokens": result.output_tokens,
                    "cache_read_tokens": result.cache_read_tokens,
                    "cache_efficiency_percent": result.cache_efficiency(),
                    "estimated_cost_usd": result.estimated_cost()
                })
                .to_string())
            }

            _ => anyhow::bail!("Unknown tool: {}", name),
        };

        // Log tool execution time
        let elapsed = start.elapsed();
        if elapsed > Duration::from_millis(100) {
            info!("Tool {} completed in {}ms", name, elapsed.as_millis());
        }

        result
    }
}
