use claudebot_mcp::MemoryStore;
use std::path::Path;

#[tokio::test]
async fn backfill_production() {
    let db_path = Path::new("/home/eliot/workspace/memory.db");
    
    println!("Opening production memory database...");
    let store = MemoryStore::open_with_embeddings(db_path).await.expect("Failed to open");
    
    let stats = store.embedding_stats().unwrap();
    println!("Before: {}/{} with embeddings ({:.0}%)", 
        stats.with_embeddings, stats.total_memories, stats.coverage_percent);
    
    if !store.has_embeddings() {
        println!("ERROR: Ollama not available!");
        return;
    }
    
    println!("\nBackfilling via Ollama...");
    match store.backfill_embeddings(100).await {
        Ok(count) => println!("Backfilled {} memories", count),
        Err(e) => println!("Error: {}", e),
    }
    
    let stats = store.embedding_stats().unwrap();
    println!("\nAfter: {}/{} with embeddings ({:.0}%)", 
        stats.with_embeddings, stats.total_memories, stats.coverage_percent);
    
    // Test hybrid search on production data
    println!("\n--- Testing Hybrid Search ---");
    let results = store.search_hybrid("Velofi trading", 3, 0.4).await.unwrap();
    for r in &results {
        println!("  [{:.3}] {} (kw: {:.3}, vec: {:.3})", 
            r.score,
            &r.entry.content[..60.min(r.entry.content.len())],
            r.keyword_score, r.vector_score);
    }
}
