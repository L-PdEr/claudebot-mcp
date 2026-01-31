<script setup lang="ts">
import type { StatusResponse, MetricsEvent, MessageEvent, LogEvent } from '~/types/api'

// API and SSE composables
const { getStatus, getHealth } = useApi()
const { status: sseStatus, connectMetrics, connectMessages, connectLogs, disconnectAll } = useSSE()

// State
const systemStatus = ref<StatusResponse | null>(null)
const metrics = ref<MetricsEvent | null>(null)
const messages = ref<MessageEvent[]>([])
const logs = ref<LogEvent[]>([])
const loading = ref(true)
const error = ref<string | null>(null)

// Computed values
const uptime = computed(() => {
  if (!systemStatus.value) return '—'
  const seconds = systemStatus.value.uptime_secs
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (hours > 0) return `${hours}h ${minutes}m`
  return `${minutes}m`
})

const memoryDisplay = computed(() => {
  if (!systemStatus.value) return '—'
  return `${systemStatus.value.memory_mb} MB`
})

const memoryPercent = computed(() => {
  // Estimate memory usage (assume 512MB is 100%)
  if (!systemStatus.value) return 0
  return Math.min(Math.round((systemStatus.value.memory_mb / 512) * 100), 100)
})

const cacheHitPercent = computed(() => {
  if (!metrics.value) return 0
  return Math.round(metrics.value.cache_hit_rate)
})

const botStatusDisplay = computed(() => {
  if (!systemStatus.value) return 'unknown'
  return systemStatus.value.bot_status === 'running' ? 'online' : 'offline'
})

const apiStatusDisplay = computed(() => {
  if (!systemStatus.value) return 'unknown'
  switch (systemStatus.value.api_status) {
    case 'ok':
      return 'online'
    case 'degraded':
      return 'degraded'
    default:
      return 'offline'
  }
})

// Actions
async function loadInitialData() {
  loading.value = true
  error.value = null
  try {
    const [statusRes] = await Promise.all([
      getStatus(),
      getHealth(),
    ])
    systemStatus.value = statusRes
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'Failed to load data'
  } finally {
    loading.value = false
  }
}

function handleRefresh() {
  loadInitialData()
}

// Lifecycle
onMounted(() => {
  loadInitialData()

  // Connect SSE streams
  connectMetrics((data) => {
    metrics.value = data
  })

  connectMessages((data) => {
    messages.value = [data, ...messages.value.slice(0, 49)]
  })

  connectLogs((data) => {
    logs.value = [data, ...logs.value.slice(0, 99)]
  })
})

onUnmounted(() => {
  disconnectAll()
})
</script>

<template>
  <div class="dashboard">
    <AppHeader
      :status="sseStatus"
      :version="systemStatus?.version || '0.1.0'"
      @refresh="handleRefresh"
    />

    <main class="dashboard__main container">
      <!-- Error Banner -->
      <div v-if="error" class="alert alert--error">
        <span>{{ error }}</span>
        <button class="btn btn--ghost btn--sm" @click="handleRefresh">
          Retry
        </button>
      </div>

      <!-- Stats Grid -->
      <section class="dashboard__section">
        <h2 class="section-title">System Overview</h2>
        <div class="stats-grid">
          <StatCard title="Messages Today" icon="💬" :loading="loading">
            <div class="stat-value">{{ metrics?.messages_today || 0 }}</div>
            <div class="stat-label">Processed today</div>
          </StatCard>

          <StatCard title="Active Users" icon="👥" :loading="loading">
            <div class="stat-value">{{ metrics?.active_users || 0 }}</div>
            <div class="stat-label">Currently active</div>
          </StatCard>

          <StatCard title="Uptime" icon="⏱️" :loading="loading">
            <div class="stat-value">{{ uptime }}</div>
            <div class="stat-label">Since restart</div>
          </StatCard>

          <StatCard title="Tokens Today" icon="🔤" :loading="loading">
            <div class="stat-value">{{ metrics?.tokens_today?.toLocaleString() || 0 }}</div>
            <div class="stat-label">Used today</div>
          </StatCard>
        </div>
      </section>

      <!-- System Resources -->
      <section class="dashboard__section">
        <h2 class="section-title">Resources</h2>
        <div class="card card--glow">
          <div class="card__body">
            <MetricBar
              label="Memory"
              :value="memoryDisplay"
              :percent="memoryPercent"
            />
            <MetricBar
              label="Cache Hit Rate"
              :value="`${cacheHitPercent}%`"
              :percent="cacheHitPercent"
              variant="success"
            />
          </div>
        </div>
      </section>

      <!-- Two Column Layout -->
      <div class="dashboard__columns">
        <!-- Services Status -->
        <section class="dashboard__section">
          <h2 class="section-title">Services</h2>
          <div class="card">
            <div class="card__body">
              <StatusRow
                label="Telegram Bot"
                :status="botStatusDisplay"
              />
              <StatusRow
                label="API Server"
                :status="apiStatusDisplay"
              />
              <StatusRow
                label="Dashboard"
                status="online"
              />
            </div>
          </div>
        </section>

        <!-- Cost Display -->
        <section class="dashboard__section">
          <h2 class="section-title">Usage Cost</h2>
          <div class="card card--glow">
            <div class="card__body cost-display">
              <div class="cost-display__amount">
                ${{ metrics?.cost_today_usd?.toFixed(4) || '0.0000' }}
              </div>
              <div class="cost-display__label">Today's API cost</div>
            </div>
          </div>
        </section>
      </div>

      <!-- Messages & Logs -->
      <div class="dashboard__columns">
        <section class="dashboard__section">
          <h2 class="section-title">Recent Messages</h2>
          <div class="card">
            <div class="card__body">
              <MessageList :messages="messages" :loading="loading" />
            </div>
          </div>
        </section>

        <section class="dashboard__section">
          <h2 class="section-title">Logs</h2>
          <div class="card">
            <div class="card__body card__body--flush">
              <LogViewer :logs="logs" :loading="loading" max-height="400px" />
            </div>
          </div>
        </section>
      </div>
    </main>

    <!-- Footer -->
    <footer class="dashboard__footer">
      <div class="container">
        <span>ClaudeBot Dashboard</span>
        <span class="dashboard__footer-dot">•</span>
        <span>{{ systemStatus?.version || 'v0.1.0' }}</span>
      </div>
    </footer>
  </div>
</template>

<style lang="scss" scoped>
.dashboard {
  min-height: 100vh;
  display: flex;
  flex-direction: column;

  &__main {
    flex: 1;
    padding: $spacing-6 $spacing-4;
  }

  &__section {
    margin-bottom: $spacing-6;
  }

  &__columns {
    display: grid;
    grid-template-columns: 1fr;
    gap: $spacing-6;

    @media (min-width: $breakpoint-lg) {
      grid-template-columns: 1fr 1fr;
    }
  }

  &__footer {
    padding: $spacing-4;
    text-align: center;
    font-size: $font-size-sm;
    color: var(--text-muted);
    border-top: 1px solid var(--border);

    .container {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: $spacing-2;
    }
  }

  &__footer-dot {
    opacity: 0.5;
  }
}

.section-title {
  font-size: $font-size-lg;
  font-weight: $font-weight-semibold;
  margin-bottom: $spacing-4;
  color: var(--text-primary);
}

.stats-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: $spacing-4;

  @media (min-width: $breakpoint-md) {
    grid-template-columns: repeat(4, 1fr);
  }
}

.stat-value {
  font-size: $font-size-3xl;
  font-weight: $font-weight-bold;
  font-family: var(--font-mono);
  @include text-gradient;
}

.stat-label {
  font-size: $font-size-sm;
  color: var(--text-muted);
  margin-top: $spacing-1;
}

.cost-display {
  text-align: center;
  padding: $spacing-4 0;

  &__amount {
    font-size: $font-size-4xl;
    font-weight: $font-weight-bold;
    font-family: var(--font-mono);
    @include text-gradient;
  }

  &__label {
    font-size: $font-size-sm;
    color: var(--text-muted);
    margin-top: $spacing-2;
  }
}

.alert {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: $spacing-3 $spacing-4;
  border-radius: $radius-md;
  margin-bottom: $spacing-4;

  &--error {
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid var(--error);
    color: var(--error);
  }
}

.card__body--flush {
  padding: 0;
}
</style>
