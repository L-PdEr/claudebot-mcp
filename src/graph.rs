//! Graph Memory Layer (E3)
//!
//! Entity extraction, relationships, and hybrid retrieval.
//! Extends basic memory with graph-based knowledge representation.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info};

/// Entity types for knowledge graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Project,
    Person,
    Technology,
    Preference,
    Concept,
    Decision,
    File,
}

impl EntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityType::Project => "project",
            EntityType::Person => "person",
            EntityType::Technology => "technology",
            EntityType::Preference => "preference",
            EntityType::Concept => "concept",
            EntityType::Decision => "decision",
            EntityType::File => "file",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "project" => Some(EntityType::Project),
            "person" => Some(EntityType::Person),
            "technology" => Some(EntityType::Technology),
            "preference" => Some(EntityType::Preference),
            "concept" => Some(EntityType::Concept),
            "decision" => Some(EntityType::Decision),
            "file" => Some(EntityType::File),
            _ => None,
        }
    }
}

/// Relationship types between entities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    WorksOn,
    Prefers,
    Knows,
    Uses,
    RelatedTo,
    DependsOn,
    CreatedBy,
    Contains,
}

impl RelationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            RelationType::WorksOn => "works_on",
            RelationType::Prefers => "prefers",
            RelationType::Knows => "knows",
            RelationType::Uses => "uses",
            RelationType::RelatedTo => "related_to",
            RelationType::DependsOn => "depends_on",
            RelationType::CreatedBy => "created_by",
            RelationType::Contains => "contains",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "works_on" => Some(RelationType::WorksOn),
            "prefers" => Some(RelationType::Prefers),
            "knows" => Some(RelationType::Knows),
            "uses" => Some(RelationType::Uses),
            "related_to" => Some(RelationType::RelatedTo),
            "depends_on" => Some(RelationType::DependsOn),
            "created_by" => Some(RelationType::CreatedBy),
            "contains" => Some(RelationType::Contains),
            _ => None,
        }
    }
}

/// An entity in the knowledge graph
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub entity_type: String,
    pub name: String,
    pub attributes: serde_json::Value,
    pub created_at: i64,
}

/// A relationship between two entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub weight: f64,
    pub valid_from: i64,
    pub valid_until: Option<i64>,
    pub evidence_count: i64,
}

/// Extracted entity from text (for Llama extraction)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub name: String,
    #[serde(default)]
    pub attributes: serde_json::Value,
}

/// Extracted relation from text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedRelation {
    pub source: String,
    pub target: String,
    pub relation: String,
}

/// Graph search result combining vector and graph scores
#[derive(Debug, Clone, Serialize)]
pub struct GraphSearchResult {
    pub entity: Entity,
    pub score: f64,
    pub path: Vec<String>,
    pub relations: Vec<Relation>,
}

/// Graph memory store
pub struct GraphStore {
    conn: Connection,
}

impl GraphStore {
    /// Initialize graph tables in existing database
    pub fn init(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            -- Entity table
            CREATE TABLE IF NOT EXISTS entities (
                id TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                name TEXT NOT NULL,
                attributes TEXT DEFAULT '{}',
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                UNIQUE(entity_type, name)
            );

            CREATE INDEX IF NOT EXISTS idx_entities_type ON entities(entity_type);
            CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name);

            -- Relations table
            CREATE TABLE IF NOT EXISTS relations (
                id TEXT PRIMARY KEY,
                source_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                target_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                relation_type TEXT NOT NULL,
                weight REAL DEFAULT 1.0,
                valid_from INTEGER NOT NULL DEFAULT (unixepoch()),
                valid_until INTEGER,
                evidence_count INTEGER DEFAULT 1,
                UNIQUE(source_id, target_id, relation_type)
            );

            CREATE INDEX IF NOT EXISTS idx_relations_source ON relations(source_id);
            CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_id);
            CREATE INDEX IF NOT EXISTS idx_relations_type ON relations(relation_type);

            -- Entity-Memory links
            CREATE TABLE IF NOT EXISTS entity_memories (
                entity_id TEXT NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                memory_id TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                PRIMARY KEY (entity_id, memory_id)
            );
            "#,
        )?;

        info!("Graph schema initialized");
        Ok(())
    }

    /// Open graph store with existing connection
    pub fn new(conn: Connection) -> Result<Self> {
        Self::init(&conn)?;
        Ok(Self { conn })
    }

    /// Open graph store from file path
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::new(conn)
    }

    /// Generate entity ID from type and name
    fn entity_id(entity_type: &str, name: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(entity_type.as_bytes());
        hasher.update(b":");
        hasher.update(name.to_lowercase().as_bytes());
        hex::encode(&hasher.finalize()[..16])
    }

    /// Generate relation ID
    fn relation_id(source_id: &str, target_id: &str, relation_type: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(source_id.as_bytes());
        hasher.update(b"->");
        hasher.update(target_id.as_bytes());
        hasher.update(b":");
        hasher.update(relation_type.as_bytes());
        hex::encode(&hasher.finalize()[..16])
    }

    /// Add or update an entity
    pub fn add_entity(
        &self,
        entity_type: &str,
        name: &str,
        attributes: Option<serde_json::Value>,
    ) -> Result<String> {
        let id = Self::entity_id(entity_type, name);
        let attrs = attributes.unwrap_or(serde_json::json!({})).to_string();

        self.conn.execute(
            r#"
            INSERT INTO entities (id, entity_type, name, attributes)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(entity_type, name) DO UPDATE SET
                attributes = json_patch(entities.attributes, excluded.attributes)
            "#,
            params![id, entity_type, name, attrs],
        )?;

        debug!("Entity added/updated: {} ({}: {})", &id[..8], entity_type, name);
        Ok(id)
    }

    /// Add or strengthen a relationship
    pub fn add_relation(
        &self,
        source_id: &str,
        target_id: &str,
        relation_type: &str,
        weight: Option<f64>,
    ) -> Result<String> {
        let id = Self::relation_id(source_id, target_id, relation_type);
        let w = weight.unwrap_or(1.0);

        self.conn.execute(
            r#"
            INSERT INTO relations (id, source_id, target_id, relation_type, weight)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(source_id, target_id, relation_type) DO UPDATE SET
                weight = MIN(relations.weight + 0.1, 2.0),
                evidence_count = relations.evidence_count + 1
            "#,
            params![id, source_id, target_id, relation_type, w],
        )?;

        debug!(
            "Relation added/strengthened: {} -> {} ({})",
            &source_id[..8],
            &target_id[..8],
            relation_type
        );
        Ok(id)
    }

    /// Link entity to a memory
    pub fn link_to_memory(&self, entity_id: &str, memory_id: &str) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO entity_memories (entity_id, memory_id)
            VALUES (?1, ?2)
            "#,
            params![entity_id, memory_id],
        )?;
        Ok(())
    }

    /// Find entity by name (fuzzy)
    pub fn find_entity(&self, name: &str) -> Result<Option<Entity>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, entity_type, name, attributes, created_at
            FROM entities
            WHERE name LIKE ?1 OR name LIKE ?2
            LIMIT 1
            "#,
        )?;

        let pattern = format!("%{}%", name);
        let exact = name.to_lowercase();

        let result = stmt
            .query_row(params![exact, pattern], |row| {
                Ok(Entity {
                    id: row.get(0)?,
                    entity_type: row.get(1)?,
                    name: row.get(2)?,
                    attributes: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                    created_at: row.get(4)?,
                })
            })
            .ok();

        Ok(result)
    }

    /// Get entities by type
    pub fn get_by_type(&self, entity_type: &str, limit: usize) -> Result<Vec<Entity>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, entity_type, name, attributes, created_at
            FROM entities
            WHERE entity_type = ?1
            ORDER BY created_at DESC
            LIMIT ?2
            "#,
        )?;

        let results = stmt
            .query_map(params![entity_type, limit], |row| {
                Ok(Entity {
                    id: row.get(0)?,
                    entity_type: row.get(1)?,
                    name: row.get(2)?,
                    attributes: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
                    created_at: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Traverse graph from entity (1-2 hops)
    pub fn traverse(&self, entity_id: &str, max_hops: usize) -> Result<Vec<GraphSearchResult>> {
        let mut results = Vec::new();
        let mut visited = std::collections::HashSet::new();
        visited.insert(entity_id.to_string());

        // First hop
        let first_hop = self.get_related(entity_id)?;
        for (entity, relation) in first_hop {
            if visited.insert(entity.id.clone()) {
                let first_hop_weight = relation.weight;
                let first_hop_relation = relation.clone();

                results.push(GraphSearchResult {
                    entity: entity.clone(),
                    score: first_hop_weight,
                    path: vec![entity_id.to_string(), entity.id.clone()],
                    relations: vec![relation],
                });

                // Second hop if allowed
                if max_hops >= 2 {
                    let second_hop = self.get_related(&entity.id)?;
                    for (e2, r2) in second_hop {
                        if visited.insert(e2.id.clone()) {
                            results.push(GraphSearchResult {
                                entity: e2.clone(),
                                score: first_hop_weight * r2.weight * 0.5, // Decay
                                path: vec![entity_id.to_string(), entity.id.clone(), e2.id.clone()],
                                relations: vec![first_hop_relation.clone(), r2],
                            });
                        }
                    }
                }
            }
        }

        // Sort by score
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results)
    }

    /// Get directly related entities
    fn get_related(&self, entity_id: &str) -> Result<Vec<(Entity, Relation)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT e.id, e.entity_type, e.name, e.attributes, e.created_at,
                   r.id, r.source_id, r.target_id, r.relation_type, r.weight,
                   r.valid_from, r.valid_until, r.evidence_count
            FROM relations r
            JOIN entities e ON (r.target_id = e.id OR r.source_id = e.id)
            WHERE (r.source_id = ?1 OR r.target_id = ?1)
              AND e.id != ?1
              AND (r.valid_until IS NULL OR r.valid_until > unixepoch())
            ORDER BY r.weight DESC
            LIMIT 20
            "#,
        )?;

        let results = stmt
            .query_map(params![entity_id], |row| {
                Ok((
                    Entity {
                        id: row.get(0)?,
                        entity_type: row.get(1)?,
                        name: row.get(2)?,
                        attributes: serde_json::from_str(&row.get::<_, String>(3)?)
                            .unwrap_or_default(),
                        created_at: row.get(4)?,
                    },
                    Relation {
                        id: row.get(5)?,
                        source_id: row.get(6)?,
                        target_id: row.get(7)?,
                        relation_type: row.get(8)?,
                        weight: row.get(9)?,
                        valid_from: row.get(10)?,
                        valid_until: row.get(11)?,
                        evidence_count: row.get(12)?,
                    },
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Extract entities from text using pattern matching
    /// (Fallback when Llama is not available)
    /// Optimized: lowercase once, use static regex
    pub fn extract_entities_simple(text: &str) -> Vec<ExtractedEntity> {
        use once_cell::sync::Lazy;

        static TECH_KEYWORDS: &[&str] = &[
            "rust", "typescript", "vue", "nuxt", "axum", "tokio", "postgresql",
            "redis", "sqlite", "docker", "kubernetes", "wasm", "grpc",
        ];

        static PROJECT_REGEX: Lazy<regex::Regex> =
            Lazy::new(|| regex::Regex::new(r"\b([A-Z][a-z]+(?:[A-Z][a-z]+)*)\b").unwrap());

        let mut entities = Vec::with_capacity(8); // Pre-allocate typical size

        // Lowercase once for all comparisons
        let text_lower = text.to_lowercase();

        for &tech in TECH_KEYWORDS {
            if text_lower.contains(tech) {
                entities.push(ExtractedEntity {
                    entity_type: "technology".to_string(),
                    name: tech.to_string(),
                    attributes: serde_json::json!({}),
                });
            }
        }

        // Project patterns (capitalized words)
        for cap in PROJECT_REGEX.captures_iter(text) {
            let name = &cap[1];
            let name_lower = name.to_lowercase();
            if name.len() > 2 && !TECH_KEYWORDS.contains(&name_lower.as_str()) {
                entities.push(ExtractedEntity {
                    entity_type: "project".to_string(),
                    name: name.to_string(),
                    attributes: serde_json::json!({}),
                });
            }
        }

        entities
    }

    /// Store extracted entities from memory content
    pub fn store_extracted(
        &self,
        memory_id: &str,
        entities: &[ExtractedEntity],
        relations: &[ExtractedRelation],
    ) -> Result<()> {
        // Store entities
        let mut entity_ids: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        for e in entities {
            let id = self.add_entity(&e.entity_type, &e.name, Some(e.attributes.clone()))?;
            self.link_to_memory(&id, memory_id)?;
            entity_ids.insert(e.name.to_lowercase(), id);
        }

        // Store relations
        for r in relations {
            let source = entity_ids.get(&r.source.to_lowercase());
            let target = entity_ids.get(&r.target.to_lowercase());

            if let (Some(src), Some(tgt)) = (source, target) {
                self.add_relation(src, tgt, &r.relation, None)?;
            }
        }

        Ok(())
    }

    /// Get graph statistics
    pub fn stats(&self) -> Result<GraphStats> {
        let entity_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))?;

        let relation_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM relations", [], |row| row.get(0))?;

        let mut stmt = self
            .conn
            .prepare("SELECT entity_type, COUNT(*) FROM entities GROUP BY entity_type")?;
        let by_type: Vec<(String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(GraphStats {
            entity_count: entity_count as usize,
            relation_count: relation_count as usize,
            by_type,
        })
    }
}

/// Graph statistics
#[derive(Debug, Clone, Serialize)]
pub struct GraphStats {
    pub entity_count: usize,
    pub relation_count: usize,
    pub by_type: Vec<(String, i64)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn temp_graph(name: &str) -> GraphStore {
        let conn = Connection::open(format!("/tmp/claudebot_graph_{}.db", name)).unwrap();
        GraphStore::new(conn).unwrap()
    }

    #[test]
    fn test_entity_crud() {
        let store = temp_graph("entity");

        let id1 = store
            .add_entity("technology", "Rust", Some(serde_json::json!({"year": 2010})))
            .unwrap();
        let id2 = store.add_entity("project", "Velofi", None).unwrap();

        let entity = store.find_entity("Rust").unwrap();
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().name, "Rust");

        let techs = store.get_by_type("technology", 10).unwrap();
        assert_eq!(techs.len(), 1);

        // Add relation
        store.add_relation(&id2, &id1, "uses", None).unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.entity_count, 2);
        assert_eq!(stats.relation_count, 1);
    }

    #[test]
    fn test_graph_traversal() {
        let store = temp_graph("traverse");

        let rust = store.add_entity("technology", "Rust", None).unwrap();
        let axum = store.add_entity("technology", "Axum", None).unwrap();
        let tokio = store.add_entity("technology", "Tokio", None).unwrap();
        let velofi = store.add_entity("project", "Velofi", None).unwrap();

        // Velofi uses Rust, Axum, Tokio
        store.add_relation(&velofi, &rust, "uses", Some(1.0)).unwrap();
        store.add_relation(&velofi, &axum, "uses", Some(0.9)).unwrap();
        store.add_relation(&axum, &tokio, "depends_on", Some(1.0)).unwrap();

        // Traverse from Velofi
        let results = store.traverse(&velofi, 2).unwrap();
        assert!(results.len() >= 2);

        // Should find Rust with high score
        let rust_result = results.iter().find(|r| r.entity.name == "Rust");
        assert!(rust_result.is_some());
    }

    #[test]
    fn test_simple_extraction() {
        let text = "We use Rust and TypeScript with Vue for the Velofi project";
        let entities = GraphStore::extract_entities_simple(text);

        assert!(entities.iter().any(|e| e.name == "rust"));
        assert!(entities.iter().any(|e| e.name == "typescript"));
        assert!(entities.iter().any(|e| e.name == "vue"));
    }

    #[test]
    fn test_empty_text_extraction() {
        let entities = GraphStore::extract_entities_simple("");
        assert!(entities.is_empty());
    }

    #[test]
    fn test_unicode_entity_names() {
        let store = temp_graph("unicode");

        // Entity with unicode characters
        let id = store
            .add_entity("project", "Velofi™", Some(serde_json::json!({"emoji": "🚀"})))
            .unwrap();

        let entity = store.find_entity("Velofi™").unwrap();
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().id, id);
    }

    #[test]
    fn test_duplicate_entity_merge() {
        let store = temp_graph("duplicate");

        // Add same entity twice with different attributes
        let id1 = store
            .add_entity("technology", "Rust", Some(serde_json::json!({"year": 2010})))
            .unwrap();
        let id2 = store
            .add_entity("technology", "Rust", Some(serde_json::json!({"paradigm": "systems"})))
            .unwrap();

        // Should be same entity (upsert)
        assert_eq!(id1, id2);

        // Stats should show only 1 entity
        let stats = store.stats().unwrap();
        assert_eq!(stats.entity_count, 1);
    }

    #[test]
    fn test_relation_strengthening() {
        let store = temp_graph("strengthen");

        let e1 = store.add_entity("project", "A", None).unwrap();
        let e2 = store.add_entity("project", "B", None).unwrap();

        // Add relation multiple times
        store.add_relation(&e1, &e2, "related_to", Some(1.0)).unwrap();
        store.add_relation(&e1, &e2, "related_to", Some(1.0)).unwrap();
        store.add_relation(&e1, &e2, "related_to", Some(1.0)).unwrap();

        // Should still be 1 relation (strengthened)
        let stats = store.stats().unwrap();
        assert_eq!(stats.relation_count, 1);
    }

    #[test]
    fn test_traverse_no_relations() {
        let store = temp_graph("no_rel");

        let id = store.add_entity("project", "Isolated", None).unwrap();

        let results = store.traverse(&id, 2).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_find_nonexistent_entity() {
        let store = temp_graph("nonexist");

        let entity = store.find_entity("DoesNotExist").unwrap();
        assert!(entity.is_none());
    }
}
