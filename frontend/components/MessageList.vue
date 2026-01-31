<script setup lang="ts">
import type { MessageEvent } from '~/types/api'

interface Props {
  messages: MessageEvent[]
  loading?: boolean
}

withDefaults(defineProps<Props>(), {
  loading: false,
})

function formatTime(timestamp: string): string {
  const date = new Date(timestamp)
  return date.toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  })
}
</script>

<template>
  <div class="message-list">
    <div v-if="loading" class="message-list__loading">
      <div v-for="i in 5" :key="i" class="skeleton message-list__skeleton" />
    </div>

    <div v-else-if="messages.length === 0" class="message-list__empty">
      <span class="message-list__empty-icon">💬</span>
      <span class="message-list__empty-text">No messages yet</span>
    </div>

    <TransitionGroup v-else name="message" tag="div" class="message-list__items">
      <div
        v-for="msg in messages"
        :key="msg.id"
        class="message"
      >
        <div class="message__header">
          <span class="message__icon">📨</span>
          <span class="message__user">{{ msg.user }}</span>
          <span class="message__time">{{ formatTime(msg.timestamp) }}</span>
        </div>
        <div class="message__content">{{ msg.content }}</div>
      </div>
    </TransitionGroup>
  </div>
</template>

<style lang="scss" scoped>
.message-list {
  max-height: 400px;
  overflow-y: auto;

  &__loading {
    display: flex;
    flex-direction: column;
    gap: $spacing-3;
  }

  &__skeleton {
    height: 60px;
    border-radius: $radius-md;
  }

  &__empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: $spacing-8;
    color: var(--text-muted);
  }

  &__empty-icon {
    font-size: 2rem;
    margin-bottom: $spacing-2;
    opacity: 0.5;
  }

  &__empty-text {
    font-size: $font-size-sm;
  }

  &__items {
    display: flex;
    flex-direction: column;
    gap: $spacing-2;
  }
}

.message {
  padding: $spacing-3;
  border-radius: $radius-md;
  background: var(--bg-tertiary);
  border-left: 3px solid var(--primary);
  transition: all $transition-fast;

  &:hover {
    background: var(--bg-secondary);
  }

  &__header {
    display: flex;
    align-items: center;
    gap: $spacing-2;
    margin-bottom: $spacing-1;
  }

  &__icon {
    font-size: $font-size-sm;
  }

  &__user {
    font-weight: $font-weight-semibold;
    font-size: $font-size-sm;
  }

  &__time {
    margin-left: auto;
    font-size: $font-size-xs;
    color: var(--text-muted);
    font-family: var(--font-mono);
  }

  &__content {
    font-size: $font-size-sm;
    color: var(--text-secondary);
    line-height: 1.4;
    word-break: break-word;
  }
}

// Transition animations
.message-enter-active {
  transition: all 0.3s ease-out;
}

.message-leave-active {
  transition: all 0.2s ease-in;
}

.message-enter-from {
  opacity: 0;
  transform: translateX(-20px);
}

.message-leave-to {
  opacity: 0;
  transform: translateX(20px);
}

.message-move {
  transition: transform 0.3s ease;
}
</style>
