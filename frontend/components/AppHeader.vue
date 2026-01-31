<script setup lang="ts">
import type { SSEStatus } from '~/composables/useSSE'

interface Props {
  status?: SSEStatus
  version?: string
}

const props = withDefaults(defineProps<Props>(), {
  status: 'disconnected',
  version: '0.1.0',
})

const statusText = computed(() => {
  switch (props.status) {
    case 'connected':
      return 'Connected'
    case 'connecting':
      return 'Connecting...'
    case 'error':
      return 'Error'
    default:
      return 'Disconnected'
  }
})

const statusClass = computed(() => {
  switch (props.status) {
    case 'connected':
      return 'status-badge__dot--online'
    case 'connecting':
      return 'status-badge__dot--degraded'
    default:
      return 'status-badge__dot--offline'
  }
})
</script>

<template>
  <header class="header">
    <div class="header__content container">
      <div class="header__brand">
        <span class="header__logo">🤖</span>
        <span class="header__title">ClaudeBot</span>
        <span class="header__version">v{{ version }}</span>
      </div>

      <div class="header__actions">
        <div class="status-badge">
          <span class="status-badge__dot" :class="statusClass" />
          <span>{{ statusText }}</span>
        </div>

        <button class="btn btn--ghost btn--sm" @click="$emit('refresh')">
          🔄
        </button>
      </div>
    </div>
  </header>
</template>

<style lang="scss" scoped>
.header {
  position: sticky;
  top: 0;
  z-index: $z-sticky;
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
  backdrop-filter: blur(12px);

  &__content {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 64px;
    gap: $spacing-4;
  }

  &__brand {
    display: flex;
    align-items: center;
    gap: $spacing-2;
  }

  &__logo {
    font-size: $font-size-2xl;
  }

  &__title {
    font-size: $font-size-xl;
    font-weight: $font-weight-bold;
    @include text-gradient;

    @media (max-width: $breakpoint-sm) {
      display: none;
    }
  }

  &__version {
    font-size: $font-size-xs;
    color: var(--text-muted);
    background: var(--bg-tertiary);
    padding: $spacing-1 $spacing-2;
    border-radius: $radius-sm;
  }

  &__actions {
    display: flex;
    align-items: center;
    gap: $spacing-3;
  }
}
</style>
