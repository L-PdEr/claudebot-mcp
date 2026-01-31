//! Streaming Response Engine
//!
//! Provides real-time response streaming capabilities:
//! - Chunked response delivery
//! - Progress indicators
//! - Cancellation support
//! - Buffer management
//!
//! Industry standard: OpenAI streaming, Server-Sent Events

use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, RwLock};

/// A chunk of streaming response
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// Chunk index (0-based)
    pub index: usize,
    /// Content of this chunk
    pub content: String,
    /// Is this the final chunk?
    pub is_final: bool,
    /// Timestamp when chunk was generated
    pub timestamp: Instant,
    /// Optional metadata
    pub metadata: Option<ChunkMetadata>,
}

/// Metadata for stream chunks
#[derive(Debug, Clone)]
pub struct ChunkMetadata {
    /// Token count in this chunk
    pub tokens: usize,
    /// Generation time for this chunk
    pub generation_ms: u64,
    /// Source (e.g., "llm", "tool", "cache")
    pub source: String,
}

impl StreamChunk {
    /// Create a new content chunk
    pub fn content(index: usize, content: String) -> Self {
        Self {
            index,
            content,
            is_final: false,
            timestamp: Instant::now(),
            metadata: None,
        }
    }

    /// Create the final chunk
    pub fn final_chunk(index: usize, content: String) -> Self {
        Self {
            index,
            content,
            is_final: true,
            timestamp: Instant::now(),
            metadata: None,
        }
    }

    /// Add metadata
    pub fn with_metadata(mut self, metadata: ChunkMetadata) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Handle for controlling a stream
#[derive(Clone)]
pub struct StreamHandle {
    /// Stream ID
    pub id: String,
    /// Cancellation flag
    cancelled: Arc<AtomicBool>,
    /// Pause flag
    paused: Arc<AtomicBool>,
    /// Progress (bytes sent)
    bytes_sent: Arc<AtomicUsize>,
    /// Start time
    started_at: Instant,
}

impl StreamHandle {
    /// Create a new stream handle
    pub fn new(id: String) -> Self {
        Self {
            id,
            cancelled: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            bytes_sent: Arc::new(AtomicUsize::new(0)),
            started_at: Instant::now(),
        }
    }

    /// Cancel the stream
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Check if cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Pause the stream
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    /// Resume the stream
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    /// Check if paused
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Update bytes sent
    pub fn add_bytes(&self, bytes: usize) {
        self.bytes_sent.fetch_add(bytes, Ordering::SeqCst);
    }

    /// Get bytes sent
    pub fn bytes_sent(&self) -> usize {
        self.bytes_sent.load(Ordering::SeqCst)
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }

    /// Get throughput (bytes per second)
    pub fn throughput(&self) -> f64 {
        let elapsed = self.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.bytes_sent() as f64 / elapsed
        } else {
            0.0
        }
    }
}

/// Streaming response builder and sender
pub struct StreamingResponse {
    /// Stream handle
    pub handle: StreamHandle,
    /// Chunk sender
    sender: mpsc::Sender<StreamChunk>,
    /// Current chunk index
    index: AtomicUsize,
    /// Buffer for accumulating content
    buffer: Arc<RwLock<String>>,
    /// Minimum chunk size before sending
    min_chunk_size: usize,
}

impl StreamingResponse {
    /// Create a new streaming response
    pub fn new(buffer_size: usize) -> (Self, mpsc::Receiver<StreamChunk>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        let handle = StreamHandle::new(uuid::Uuid::new_v4().to_string());

        let response = Self {
            handle,
            sender: tx,
            index: AtomicUsize::new(0),
            buffer: Arc::new(RwLock::new(String::new())),
            min_chunk_size: 10,
        };

        (response, rx)
    }

    /// Set minimum chunk size
    pub fn with_min_chunk_size(mut self, size: usize) -> Self {
        self.min_chunk_size = size;
        self
    }

    /// Send a chunk immediately
    pub async fn send(&self, content: String) -> Result<()> {
        if self.handle.is_cancelled() {
            return Err(anyhow::anyhow!("Stream cancelled"));
        }

        while self.handle.is_paused() {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if self.handle.is_cancelled() {
                return Err(anyhow::anyhow!("Stream cancelled"));
            }
        }

        let index = self.index.fetch_add(1, Ordering::SeqCst);
        let chunk = StreamChunk::content(index, content.clone());

        self.handle.add_bytes(content.len());
        self.sender.send(chunk).await?;

        Ok(())
    }

    /// Buffer content (send when buffer is large enough)
    pub async fn buffer(&self, content: &str) -> Result<()> {
        {
            let mut buffer = self.buffer.write().await;
            buffer.push_str(content);

            if buffer.len() >= self.min_chunk_size {
                let to_send = std::mem::take(&mut *buffer);
                drop(buffer);
                return self.send(to_send).await;
            }
        }
        Ok(())
    }

    /// Flush buffer and send final chunk
    pub async fn finish(self) -> Result<()> {
        // Send any remaining buffer
        let remaining = {
            let buffer = self.buffer.read().await;
            buffer.clone()
        };

        let index = self.index.fetch_add(1, Ordering::SeqCst);
        let chunk = StreamChunk::final_chunk(index, remaining.clone());

        if !remaining.is_empty() {
            self.handle.add_bytes(remaining.len());
        }

        self.sender.send(chunk).await?;
        Ok(())
    }

    /// Send error and close stream
    pub async fn error(self, error: String) -> Result<()> {
        let index = self.index.fetch_add(1, Ordering::SeqCst);
        let chunk = StreamChunk {
            index,
            content: format!("Error: {}", error),
            is_final: true,
            timestamp: Instant::now(),
            metadata: Some(ChunkMetadata {
                tokens: 0,
                generation_ms: 0,
                source: "error".to_string(),
            }),
        };

        self.sender.send(chunk).await?;
        Ok(())
    }
}

/// Collector for reconstructing streamed response
pub struct StreamCollector {
    chunks: Vec<StreamChunk>,
    total_content: String,
}

impl StreamCollector {
    /// Create a new collector
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            total_content: String::new(),
        }
    }

    /// Add a chunk
    pub fn add(&mut self, chunk: StreamChunk) {
        self.total_content.push_str(&chunk.content);
        self.chunks.push(chunk);
    }

    /// Check if complete
    pub fn is_complete(&self) -> bool {
        self.chunks.last().map(|c| c.is_final).unwrap_or(false)
    }

    /// Get accumulated content
    pub fn content(&self) -> &str {
        &self.total_content
    }

    /// Get chunk count
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Get total tokens (if metadata available)
    pub fn total_tokens(&self) -> usize {
        self.chunks
            .iter()
            .filter_map(|c| c.metadata.as_ref())
            .map(|m| m.tokens)
            .sum()
    }

    /// Consume and return final content
    pub fn into_content(self) -> String {
        self.total_content
    }
}

impl Default for StreamCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Progress indicator for long operations
pub struct ProgressIndicator {
    /// Current progress (0.0 - 1.0)
    progress: Arc<RwLock<f64>>,
    /// Status message
    message: Arc<RwLock<String>>,
    /// Watch channel for updates
    sender: watch::Sender<(f64, String)>,
    /// Receiver for subscribers
    receiver: watch::Receiver<(f64, String)>,
}

impl ProgressIndicator {
    /// Create a new progress indicator
    pub fn new() -> Self {
        let (tx, rx) = watch::channel((0.0, String::new()));
        Self {
            progress: Arc::new(RwLock::new(0.0)),
            message: Arc::new(RwLock::new(String::new())),
            sender: tx,
            receiver: rx,
        }
    }

    /// Update progress
    pub async fn set_progress(&self, progress: f64, message: &str) {
        let progress = progress.clamp(0.0, 1.0);
        *self.progress.write().await = progress;
        *self.message.write().await = message.to_string();
        let _ = self.sender.send((progress, message.to_string()));
    }

    /// Increment progress
    pub async fn increment(&self, delta: f64, message: &str) {
        let current = *self.progress.read().await;
        self.set_progress(current + delta, message).await;
    }

    /// Get current progress
    pub async fn get_progress(&self) -> (f64, String) {
        let progress = *self.progress.read().await;
        let message = self.message.read().await.clone();
        (progress, message)
    }

    /// Subscribe to progress updates
    pub fn subscribe(&self) -> watch::Receiver<(f64, String)> {
        self.receiver.clone()
    }

    /// Format progress bar
    pub async fn format_bar(&self, width: usize) -> String {
        let (progress, message) = self.get_progress().await;
        let filled = (progress * width as f64) as usize;
        let empty = width - filled;

        format!(
            "[{}{}] {:.0}% {}",
            "█".repeat(filled),
            "░".repeat(empty),
            progress * 100.0,
            message
        )
    }
}

impl Default for ProgressIndicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Typing indicator for chat-style responses
pub struct TypingIndicator {
    active: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl TypingIndicator {
    /// Create new typing indicator
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    /// Start showing typing indicator
    pub fn start<F>(&mut self, callback: F)
    where
        F: Fn() + Send + 'static,
    {
        self.active.store(true, Ordering::SeqCst);
        let active = self.active.clone();

        self.handle = Some(tokio::spawn(async move {
            while active.load(Ordering::SeqCst) {
                callback();
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }));
    }

    /// Stop typing indicator
    pub fn stop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }

    /// Check if active
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

impl Default for TypingIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TypingIndicator {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stream_chunk_creation() {
        let chunk = StreamChunk::content(0, "Hello".to_string());
        assert_eq!(chunk.index, 0);
        assert_eq!(chunk.content, "Hello");
        assert!(!chunk.is_final);

        let final_chunk = StreamChunk::final_chunk(1, "World".to_string());
        assert!(final_chunk.is_final);
    }

    #[tokio::test]
    async fn test_stream_handle() {
        let handle = StreamHandle::new("test".to_string());

        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());

        assert!(!handle.is_paused());
        handle.pause();
        assert!(handle.is_paused());
        handle.resume();
        assert!(!handle.is_paused());
    }

    #[tokio::test]
    async fn test_streaming_response() {
        let (response, mut rx) = StreamingResponse::new(10);

        // Send chunks
        response.send("Hello ".to_string()).await.unwrap();
        response.send("World".to_string()).await.unwrap();
        response.finish().await.unwrap();

        // Collect chunks
        let mut collector = StreamCollector::new();
        while let Some(chunk) = rx.recv().await {
            let is_final = chunk.is_final;
            collector.add(chunk);
            if is_final {
                break;
            }
        }

        assert!(collector.is_complete());
        assert_eq!(collector.content(), "Hello World");
    }

    #[tokio::test]
    async fn test_stream_cancellation() {
        let (response, _rx) = StreamingResponse::new(10);

        response.handle.cancel();
        let result = response.send("Test".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_progress_indicator() {
        let progress = ProgressIndicator::new();

        progress.set_progress(0.5, "Halfway").await;
        let (p, m) = progress.get_progress().await;
        assert_eq!(p, 0.5);
        assert_eq!(m, "Halfway");

        progress.increment(0.25, "More").await;
        let (p, _) = progress.get_progress().await;
        assert_eq!(p, 0.75);
    }

    #[tokio::test]
    async fn test_progress_bar_format() {
        let progress = ProgressIndicator::new();
        progress.set_progress(0.5, "Loading").await;

        let bar = progress.format_bar(10).await;
        assert!(bar.contains("█████"));
        assert!(bar.contains("░░░░░"));
        assert!(bar.contains("50%"));
    }

    #[test]
    fn test_stream_collector() {
        let mut collector = StreamCollector::new();

        collector.add(StreamChunk::content(0, "Hello".to_string()));
        assert!(!collector.is_complete());
        assert_eq!(collector.chunk_count(), 1);

        collector.add(StreamChunk::final_chunk(1, "!".to_string()));
        assert!(collector.is_complete());
        assert_eq!(collector.content(), "Hello!");
    }
}
