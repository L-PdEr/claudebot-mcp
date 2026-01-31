// ============================================
// API Composable for ClaudeBot Dashboard
// ============================================

import type {
  StatusResponse,
  HealthResponse,
  MetricsResponse,
  ConversationsResponse,
} from '~/types/api'

export function useApi() {
  const config = useRuntimeConfig()
  const baseUrl = config.public.apiBase as string

  // Fetch helper with error handling
  async function fetchApi<T>(
    endpoint: string,
    options?: RequestInit
  ): Promise<T> {
    const response = await fetch(`${baseUrl}${endpoint}`, {
      headers: {
        'Content-Type': 'application/json',
        ...options?.headers,
      },
      ...options,
    })

    if (!response.ok) {
      const error = await response.json().catch(() => ({
        error: 'UNKNOWN_ERROR',
        message: `HTTP ${response.status}`,
      }))
      throw new Error(error.message || 'Request failed')
    }

    return response.json()
  }

  // Health check
  async function getHealth(): Promise<HealthResponse> {
    return fetchApi<HealthResponse>('/api/health')
  }

  // System status
  async function getStatus(): Promise<StatusResponse> {
    return fetchApi<StatusResponse>('/api/status')
  }

  // Metrics
  async function getMetrics(): Promise<MetricsResponse> {
    return fetchApi<MetricsResponse>('/api/metrics')
  }

  // Conversations
  async function getConversations(
    limit = 20,
    offset = 0
  ): Promise<ConversationsResponse> {
    return fetchApi<ConversationsResponse>(
      `/api/conversations?limit=${limit}&offset=${offset}`
    )
  }

  return {
    getHealth,
    getStatus,
    getMetrics,
    getConversations,
    fetchApi,
  }
}
