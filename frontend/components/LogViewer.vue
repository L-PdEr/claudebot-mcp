<script setup lang="ts">
import type { LogEvent } from '~/types/api'

interface Props {
  logs: LogEvent[]
  loading?: boolean
  maxHeight?: string
}

withDefaults(defineProps<Props>(), {
  loading: false,
  maxHeight: '300px',
})

function formatTime(timestamp: string): string {
  const date = new Date(timestamp)
  return date.toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
}

function getLevelClass(level: string): string {
  return `log--${level.toLowerCase()}`
}

function getLevelIcon(level: string): string {
  switch (level.toLowerCase()) {
    case 'error':
      return '🔴'
    case 'warn':
      return '🟡'
    case 'info':
      return '🔵'
    case 'debug':
      return '⚪'
    default:
      return '⚫'
  }
}
</script>

<template>
  <div class="log-viewer" :style="{ maxHeight }">
    <div v-if="loading" class="log-viewer__loading">
      <div v-for="i in 8" :key="i" class="skeleton log-viewer__skeleton" />
    </div>

    <div v-else-if="logs.length === 0" class="log-viewer__empty">
      <span>No logs available</span>
    </div>

    <div v-else class="log-viewer__content">
      <div
        v-for="(log, index) in logs"
        :key="`${log.timestamp}-${index}`"
        class="log"
        :class="getLevelClass(log.level)"
      >
        <span class="log__time">{{ formatTime(log.timestamp) }}</span>
        <span class="log__icon">{{ getLevelIcon(log.level) }}</span>
        <span class="log__level">{{ log.level }}</span>
        <span class="log__message">{{ log.message }}</span>
      </div>
    </div>
  </div>
</template>

<style lang="scss" scoped>
.log-viewer {
  font-family: var(--font-mono);
  font-size: $font-size-xs;
  background: var(--bg-tertiary);
  border-radius: $radius-md;
  overflow-y: auto;
  padding: $spacing-2;

  &__loading {
    display: flex;
    flex-direction: column;
    gap: $spacing-1;
  }

  &__skeleton {
    height: 20px;
    border-radius: $radius-sm;
  }

  &__empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: $spacing-6;
    color: var(--text-muted);
  }

  &__content {
    display: flex;
    flex-direction: column;
  }
}

.log {
  display: flex;
  align-items: flex-start;
  gap: $spacing-2;
  padding: $spacing-1 $spacing-2;
  border-radius: $radius-sm;
  line-height: 1.4;

  &:hover {
    background: var(--bg-secondary);
  }

  &__time {
    color: var(--text-muted);
    flex-shrink: 0;
  }

  &__icon {
    flex-shrink: 0;
    font-size: 0.6rem;
  }

  &__level {
    flex-shrink: 0;
    width: 40px;
    text-transform: uppercase;
    font-weight: $font-weight-semibold;
  }

  &__message {
    flex: 1;
    word-break: break-word;
  }

  &--error {
    .log__level {
      color: var(--error);
    }
    .log__message {
      color: var(--error-light, #fca5a5);
    }
  }

  &--warn {
    .log__level {
      color: var(--warning);
    }
  }

  &--info {
    .log__level {
      color: var(--primary);
    }
  }

  &--debug {
    .log__level,
    .log__message {
      color: var(--text-muted);
    }
  }
}
</style>
