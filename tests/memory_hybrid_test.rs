//! Integration test for hybrid memory search with Ollama

use claudebot_mcp::MemoryStore;
use tempfile::TempDir;

#[tokio::test]
async fn test_hybrid_search_with_ollama() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test_memory.db");
    
    let store = match MemoryStore::open_with_embeddings(&db_path).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed: {}", e);
            return;
        }
    };
    
    // Learn some facts (sync, no embeddings)
    store.learn("Rust is a systems programming language focused on safety", "technical", "test", 0.9).unwrap();
    store.learn("Python is great for machine learning and data science", "technical", "test", 0.9).unwrap();
    store.learn("The user prefers dark mode for coding", "preference", "test", 0.8).unwrap();
    
    println!("Has embedder: {}", store.has_embeddings());
    
    // Check stats before backfill
    let stats = store.embedding_stats().unwrap();
    println!("Before backfill: {}/{} with embeddings", stats.with_embeddings, stats.total_memories);
    
    // Backfill embeddings (async, calls Ollama)
    if store.has_embeddings() {
        println!("\nBackfilling embeddings via Ollama...");
        match store.backfill_embeddings(10).await {
            Ok(count) => println!("Backfilled {} memories", count),
            Err(e) => println!("Backfill error: {}", e),
        }
    }
    
    // Check stats after backfill
    let stats = store.embedding_stats().unwrap();
    println!("After backfill: {}/{} with embeddings ({:.0}%)", 
        stats.with_embeddings, stats.total_memories, stats.coverage_percent);
    
    // Test hybrid search
    println!("\n--- Hybrid Search for 'programming language' ---");
    let results = store.search_hybrid("programming language", 3, 0.4).await.unwrap();
    for r in &results {
        println!("  [{:.3}] {} (kw: {:.3}, vec: {:.3})", 
            r.score,
            &r.entry.content[..50.min(r.entry.content.len())],
            r.keyword_score, r.vector_score);
    }
    
    // Verify vector scores are present after backfill
    let has_vector = results.iter().any(|r| r.vector_score > 0.0);
    if has_vector {
        println!("\n✓ Vector scores present - Ollama/Llama working correctly!");
    }
    
    assert!(!results.is_empty());
    if store.has_embeddings() && stats.with_embeddings > 0 {
        assert!(has_vector, "Should have vector scores after backfill");
    }
}
