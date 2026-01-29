# ClaudeBot v2 - Optimierte Architektur

> Basierend auf Industry Benchmarks: Mem0, Letta/MemGPT, Zep, MIRIX

---

## 🏆 Benchmark-Vergleich

| System | J-Score | Stärke | ClaudeBot Status |
|--------|---------|--------|------------------|
| **MIRIX** | 78%+ | Multi-Agent | Ziel |
| **Zep** | 76.6% | Knowledge Graph | ✅ Graph implementiert |
| **Letta/MemGPT** | ~74% | Document Analysis | ⚠️ Kein Wake/Sleep |
| **Mem0g** | 68.5% | Temporal Queries | ✅ Timestamps vorhanden |
| **Mem0** | 66.9% | Production Ready | ✅ Baseline erreicht |

---

## 🎯 Was wir HABEN ✅

### 1. Prompt Caching (90% Ersparnis)
```rust
// src/claude.rs - IMPLEMENTIERT
.header("anthropic-beta", "prompt-caching-2024-07-31")
cache_control: { type: "ephemeral" }
```

### 2. Response Caching (~20% Ersparnis)
```rust
// src/cache.rs - IMPLEMENTIERT
pub struct ResponseCache {
    cache: moka::future::Cache<String, CachedResponse>,
}
```

### 3. Graph Memory (Relational)
```rust
// src/graph.rs - IMPLEMENTIERT
pub enum EntityType { Project, Person, Technology, Preference, ... }
pub enum RelationType { WorksOn, Prefers, Knows, Uses, ... }
```

### 4. Model Routing (3-Tier)
```rust
// src/router.rs - IMPLEMENTIERT
pub enum ModelHint {
    Haiku,   // $0.25/M - 80% queries
    Sonnet,  // $3/M    - 15% queries
    Opus,    // $15/M   - 5% queries
}
```

### 5. Llama Classification
```rust
// src/router.rs:193-261 - IMPLEMENTIERT
async fn route_with_llama(query: &str) -> RouteResult
// Ollama llama3.2:3b für Komplexitäts-Klassifikation
```

---

## 🔥 Was FEHLT ❌

### 1. Llama Compression Worker (KRITISCH)

**Problem:** Context wächst unbegrenzt → Token-Kosten explodieren

**Lösung:**
```rust
// src/llama_worker.rs - NEU ERSTELLEN
pub struct LlamaWorker {
    ollama_url: String,
    model: String,  // llama3.2:3b
}

impl LlamaWorker {
    /// Komprimiert lange Konversationen auf Kernaussagen
    pub async fn compress_context(&self, messages: &[Message], target_tokens: usize) -> Result<String> {
        let prompt = format!(
            "Komprimiere diese Konversation auf die wichtigsten Fakten und Entscheidungen. \
            Ziel: max {} Tokens. Behalte: Namen, Zahlen, Entscheidungen, TODOs.\n\n{}",
            target_tokens,
            messages.iter().map(|m| m.content.as_str()).collect::<Vec<_>>().join("\n---\n")
        );

        self.generate(&prompt).await
    }

    /// Extrahiert Entitäten und Relationen für Graph Memory
    pub async fn extract_entities(&self, text: &str) -> Result<Vec<Entity>> {
        let prompt = format!(
            "Extrahiere Entitäten aus diesem Text als JSON:\n\
            Format: {{\"entities\": [{{\"name\": \"...\", \"type\": \"person|project|tech\", \"context\": \"...\"}}]}}\n\n{}",
            text
        );

        let response = self.generate(&prompt).await?;
        serde_json::from_str(&response)
    }

    /// Klassifiziert Query-Komplexität für Model-Routing
    pub async fn classify_complexity(&self, query: &str) -> QueryComplexity {
        // Bereits implementiert in router.rs, hierher verschieben
    }
}
```

### 2. Wake/Sleep Cycle (MemGPT-Style)

**Problem:** Bot ist immer "wach" → keine Hintergrund-Verarbeitung

**Lösung:**
```rust
// src/lifecycle.rs - NEU ERSTELLEN
pub struct WakeSleepCycle {
    state: Arc<AtomicU8>,  // 0=Sleep, 1=Wake, 2=Processing
    idle_timeout: Duration,
    last_activity: Arc<AtomicI64>,
}

impl WakeSleepCycle {
    pub async fn run(&self) {
        loop {
            match self.current_state() {
                State::Sleep => {
                    // Hintergrund-Tasks
                    self.consolidate_memories().await;
                    self.decay_old_memories().await;
                    self.compress_long_conversations().await;

                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                State::Wake => {
                    // Warte auf Aktivität oder Timeout
                    if self.idle_for() > self.idle_timeout {
                        self.transition_to_sleep();
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                State::Processing => {
                    // Aktive Verarbeitung, nicht stören
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn consolidate_memories(&self) {
        // Ähnliche Memories zusammenführen
        // Graph-Relationen stärken
        // Ebbinghaus-Decay anwenden
    }
}
```

### 3. Vector Embeddings (Semantic Search)

**Problem:** FTS5 ist keyword-basiert, nicht semantisch

**Lösung:**
```rust
// src/embeddings.rs - NEU ERSTELLEN
pub struct EmbeddingStore {
    model: String,  // "nomic-embed-text" via Ollama
    dimension: usize,  // 384 oder 768
}

impl EmbeddingStore {
    /// Generiert Embedding lokal via Ollama
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let response = reqwest::Client::new()
            .post(&format!("{}/api/embeddings", self.ollama_url))
            .json(&json!({
                "model": self.model,
                "prompt": text
            }))
            .send()
            .await?;

        let result: EmbeddingResponse = response.json().await?;
        Ok(result.embedding)
    }

    /// Cosine Similarity für Semantic Search
    pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (norm_a * norm_b)
    }
}

// SQLite Schema-Erweiterung für Embeddings
// (Alternative zu pgvector - funktioniert mit sqlite-vss)
```

### 4. Token Pre-counting

**Problem:** Kosten erst nach API-Call bekannt

**Lösung:**
```rust
// src/tokenizer.rs - NEU ERSTELLEN
use tiktoken_rs::cl100k_base;

pub struct TokenCounter {
    bpe: tiktoken_rs::CoreBPE,
}

impl TokenCounter {
    pub fn count(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }

    pub fn estimate_cost(&self, input: &str, output_estimate: usize, model: &ModelHint) -> f64 {
        let input_tokens = self.count(input);
        let (input_price, output_price) = match model {
            ModelHint::Haiku => (0.25 / 1_000_000.0, 1.25 / 1_000_000.0),
            ModelHint::Sonnet => (3.0 / 1_000_000.0, 15.0 / 1_000_000.0),
            ModelHint::Opus => (15.0 / 1_000_000.0, 75.0 / 1_000_000.0),
        };

        (input_tokens as f64 * input_price) + (output_estimate as f64 * output_price)
    }

    /// Warnt wenn Budget überschritten wird
    pub fn check_budget(&self, input: &str, daily_remaining: f64, model: &ModelHint) -> BudgetCheck {
        let estimated = self.estimate_cost(input, 2000, model);
        if estimated > daily_remaining {
            BudgetCheck::Exceeded { estimated, remaining: daily_remaining }
        } else {
            BudgetCheck::Ok { estimated }
        }
    }
}
```

---

## 📐 Optimierte Architektur v2

```
┌─────────────────────────────────────────────────────────────────────┐
│                      ClaudeBot v2 Pipeline                          │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  User Message                                                       │
│       │                                                             │
│       ▼                                                             │
│  ┌─────────────┐     ┌─────────────┐                               │
│  │ Token Count │ ──► │Budget Check │ ──► Exceeded? → Warn User     │
│  │ (tiktoken)  │     │ Pre-flight  │                               │
│  └──────┬──────┘     └──────┬──────┘                               │
│         │                    │                                      │
│         ▼                    ▼                                      │
│  ┌─────────────────────────────────┐                               │
│  │       Response Cache            │                               │
│  │   (SHA256 Context-Aware)        │ ──► Hit? → Return (0 cost)    │
│  └──────────────┬──────────────────┘                               │
│                 │ Miss                                              │
│                 ▼                                                   │
│  ┌─────────────────────────────────┐                               │
│  │     Llama Classifier            │                               │
│  │   (Query Complexity)            │                               │
│  │   Simple → Haiku (80%)          │                               │
│  │   Moderate → Sonnet (15%)       │                               │
│  │   Complex → Opus (5%)           │                               │
│  └──────────────┬──────────────────┘                               │
│                 │                                                   │
│                 ▼                                                   │
│  ┌─────────────────────────────────┐                               │
│  │      Memory Retrieval           │                               │
│  │  ┌─────────┐   ┌─────────────┐  │                               │
│  │  │ Vector  │ + │   Graph     │  │                               │
│  │  │(Ollama) │   │(Relational) │  │                               │
│  │  └────┬────┘   └──────┬──────┘  │                               │
│  │       └───────┬───────┘         │                               │
│  │               ▼                 │                               │
│  │        Hybrid Score             │                               │
│  └──────────────┬──────────────────┘                               │
│                 │                                                   │
│                 ▼                                                   │
│  ┌─────────────────────────────────┐                               │
│  │     Context Compression         │                               │
│  │   (Llama wenn > 4K tokens)      │ ◄─── Wake/Sleep prüft        │
│  └──────────────┬──────────────────┘       periodisch              │
│                 │                                                   │
│                 ▼                                                   │
│  ┌─────────────────────────────────┐                               │
│  │       Claude API Call           │                               │
│  │   WITH PROMPT CACHING           │                               │
│  │   (Static: -90% cost)           │                               │
│  │   (Session: -90% cost)          │                               │
│  └──────────────┬──────────────────┘                               │
│                 │                                                   │
│                 ▼                                                   │
│  ┌─────────────────────────────────┐                               │
│  │     Post-Processing             │                               │
│  │  - Entity Extraction (Llama)    │                               │
│  │  - Graph Update                 │                               │
│  │  - Memory Store                 │                               │
│  │  - Response Cache Update        │                               │
│  │  - Usage Tracking               │                               │
│  └──────────────┬──────────────────┘                               │
│                 │                                                   │
│                 ▼                                                   │
│           Response to User                                          │
│                                                                     │
│  ═══════════════════════════════════════════════════════════════   │
│                    Background Tasks                                 │
│  ═══════════════════════════════════════════════════════════════   │
│                                                                     │
│  ┌─────────────────────────────────┐                               │
│  │       Wake/Sleep Cycle          │                               │
│  │  - Memory Consolidation         │                               │
│  │  - Ebbinghaus Decay             │                               │
│  │  - Context Compression          │                               │
│  │  - Graph Pruning                │                               │
│  └─────────────────────────────────┘                               │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 💰 Kostenoptimierung Zusammenfassung

| Optimierung | Implementiert | Ersparnis | Priorität |
|-------------|---------------|-----------|-----------|
| Prompt Caching | ✅ | -90% Input | - |
| Response Caching | ✅ | -20% Overall | - |
| Model Routing | ✅ | -53% (vs nur Sonnet) | - |
| **Llama Compression** | ❌ | -30-40% Tokens | HOCH |
| **Token Pre-count** | ❌ | Budget Control | HOCH |
| **Vector Embeddings** | ❌ | +10% Accuracy | MITTEL |
| **Wake/Sleep** | ❌ | Background Opt | MITTEL |
| Batch Processing | ❌ | -40% Latenz | NIEDRIG |

---

## 🎯 Nächste Schritte

### Phase 1: Token Control (1-2 Tage)
1. `src/tokenizer.rs` - Token counting mit tiktoken
2. Pre-flight Budget-Check
3. Warnung bei Budget-Überschreitung

### Phase 2: Llama Worker (2-3 Tage)
1. `src/llama_worker.rs` - Compression + Entity Extraction
2. Router-Integration refactoren
3. Context-Management bei > 4K tokens

### Phase 3: Embeddings (2-3 Tage)
1. `src/embeddings.rs` - Ollama nomic-embed-text
2. SQLite VSS Extension oder eigene Cosine-Similarity
3. Hybrid Retrieval (FTS5 + Vector)

### Phase 4: Wake/Sleep (1-2 Tage)
1. `src/lifecycle.rs` - State Machine
2. Background Tasks (Consolidation, Decay)
3. Idle Detection

---

## 📚 Referenzen

- [Mem0 Benchmark](https://arxiv.org/abs/2504.19413) - LOCOMO J-Score Methodik
- [Letta Memory](https://www.letta.com/blog/benchmarking-ai-agent-memory) - MemGPT Architektur
- [Anthropic Prompt Caching](https://www.anthropic.com/news/prompt-caching) - 90% Kostenreduktion
- [Token-Efficient Data Prep](https://thenewstack.io/a-guide-to-token-efficient-data-prep-for-llm-workloads/) - 30-40% Einsparung
- [Cost-Effective LLM Apps](https://www.glukhov.org/post/2025/11/cost-effective-llm-applications/) - Model Routing

---

*Erstellt: 2026-01-28*
*Status: Architektur-Review*
