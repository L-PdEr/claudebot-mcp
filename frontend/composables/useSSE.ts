// ============================================
// Server-Sent Events Composable
// ============================================

import { ref, onMounted, onUnmounted } from 'vue'
import type { MetricsEvent, MessageEvent, LogEvent } from '~/types/api'

export type SSEStatus = 'connecting' | 'connected' | 'disconnected' | 'error'

export function useSSE() {
  const config = useRuntimeConfig()
  const baseUrl = config.public.apiBase as string

  const status = ref<SSEStatus>('disconnected')
  const lastHeartbeat = ref<Date | null>(null)

  let metricsSource: EventSource | null = null
  let messagesSource: EventSource | null = null
  let logsSource: EventSource | null = null

  // Metrics stream
  function connectMetrics(
    onMetrics: (data: MetricsEvent) => void,
    onError?: (error: Event) => void
  ): EventSource {
    metricsSource = new EventSource(`${baseUrl}/api/stream/metrics`)

    metricsSource.addEventListener('metrics', (event) => {
      try {
        const data = JSON.parse(event.data) as MetricsEvent
        onMetrics(data)
        status.value = 'connected'
      } catch (e) {
        console.error('Failed to parse metrics:', e)
      }
    })

    metricsSource.addEventListener('heartbeat', () => {
      lastHeartbeat.value = new Date()
      status.value = 'connected'
    })

    metricsSource.onerror = (error) => {
      status.value = 'error'
      onError?.(error)
    }

    metricsSource.onopen = () => {
      status.value = 'connected'
    }

    return metricsSource
  }

  // Messages stream
  function connectMessages(
    onMessage: (data: MessageEvent) => void,
    onError?: (error: Event) => void
  ): EventSource {
    messagesSource = new EventSource(`${baseUrl}/api/stream/messages`)

    messagesSource.addEventListener('message', (event) => {
      try {
        const data = JSON.parse(event.data) as MessageEvent
        onMessage(data)
      } catch (e) {
        console.error('Failed to parse message:', e)
      }
    })

    messagesSource.addEventListener('heartbeat', () => {
      lastHeartbeat.value = new Date()
    })

    messagesSource.addEventListener('warning', (event) => {
      console.warn('Messages stream warning:', event.data)
    })

    messagesSource.onerror = (error) => {
      onError?.(error)
    }

    return messagesSource
  }

  // Logs stream
  function connectLogs(
    onLog: (data: LogEvent) => void,
    onError?: (error: Event) => void
  ): EventSource {
    logsSource = new EventSource(`${baseUrl}/api/stream/logs`)

    logsSource.addEventListener('log', (event) => {
      try {
        const data = JSON.parse(event.data) as LogEvent
        onLog(data)
      } catch (e) {
        console.error('Failed to parse log:', e)
      }
    })

    logsSource.addEventListener('heartbeat', () => {
      lastHeartbeat.value = new Date()
    })

    logsSource.addEventListener('warning', (event) => {
      console.warn('Logs stream warning:', event.data)
    })

    logsSource.onerror = (error) => {
      onError?.(error)
    }

    return logsSource
  }

  // Disconnect all streams
  function disconnectAll() {
    metricsSource?.close()
    messagesSource?.close()
    logsSource?.close()
    metricsSource = null
    messagesSource = null
    logsSource = null
    status.value = 'disconnected'
  }

  return {
    status,
    lastHeartbeat,
    connectMetrics,
    connectMessages,
    connectLogs,
    disconnectAll,
  }
}

// Auto-managed SSE for metrics
export function useMetricsStream() {
  const metrics = ref<MetricsEvent | null>(null)
  const error = ref<string | null>(null)
  const { status, connectMetrics, disconnectAll } = useSSE()

  onMounted(() => {
    connectMetrics(
      (data) => {
        metrics.value = data
        error.value = null
      },
      () => {
        error.value = 'Connection lost'
      }
    )
  })

  onUnmounted(() => {
    disconnectAll()
  })

  return { metrics, status, error }
}

// Auto-managed SSE for messages
export function useMessagesStream(maxMessages = 50) {
  const messages = ref<MessageEvent[]>([])
  const { connectMessages, disconnectAll } = useSSE()

  onMounted(() => {
    connectMessages((data) => {
      messages.value = [data, ...messages.value.slice(0, maxMessages - 1)]
    })
  })

  onUnmounted(() => {
    disconnectAll()
  })

  return { messages }
}

// Auto-managed SSE for logs
export function useLogsStream(maxLogs = 100) {
  const logs = ref<LogEvent[]>([])
  const { connectLogs, disconnectAll } = useSSE()

  onMounted(() => {
    connectLogs((data) => {
      logs.value = [data, ...logs.value.slice(0, maxLogs - 1)]
    })
  })

  onUnmounted(() => {
    disconnectAll()
  })

  return { logs }
}
