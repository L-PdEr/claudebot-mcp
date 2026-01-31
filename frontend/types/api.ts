// ============================================
// API Types for ClaudeBot Dashboard
// ============================================

// ===== Status Types =====

export type BotStatus = 'running' | 'stopped' | 'error'
export type ApiStatus = 'ok' | 'degraded' | 'down'
export type LogLevel = 'debug' | 'info' | 'warn' | 'error'

export interface StatusResponse {
  version: string
  uptime_secs: number
  memory_mb: number
  bot_status: BotStatus
  api_status: ApiStatus
  last_message_at: string | null
  timestamp: string
}

export interface HealthResponse {
  status: string
  version: string
  uptime_secs: number
  timestamp: string
}

// ===== Metrics Types =====

export interface LatencyStats {
  p50_ms: number
  p95_ms: number
  p99_ms: number
  max_ms: number
}

export interface CostBreakdown {
  input_cost_usd: number
  output_cost_usd: number
  cache_savings_usd: number
}

export interface MetricsResponse {
  messages_today: number
  messages_week: number
  tokens_today: number
  cost_today_usd: number
  cache_hit_rate: number
  avg_response_ms: number
  latency: LatencyStats
  cost: CostBreakdown
}

// ===== SSE Event Types =====

export interface MetricsEvent {
  messages_today: number
  tokens_today: number
  active_users: number
  cache_hit_rate: number
  cost_today_usd: number
}

export interface MessageEvent {
  id: string
  user: string
  content: string
  timestamp: string
}

export interface LogEvent {
  level: LogLevel
  message: string
  timestamp: string
}

export interface HeartbeatEvent {
  timestamp: string
  uptime_secs: number
}

// ===== Conversation Types =====

export interface ConversationItem {
  chat_id: number
  message_count: number
  first_message_at: number | null
  last_message_at: number | null
  preview: string | null
}

export interface ConversationsResponse {
  conversations: ConversationItem[]
  total: number
  has_more: boolean
}

// ===== Error Types =====

export interface ErrorResponse {
  error: string
  message: string
  details?: Record<string, unknown>
}
