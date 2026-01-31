// https://nuxt.com/docs/api/configuration/nuxt-config
export default defineNuxtConfig({
  compatibilityDate: '2024-12-01',
  devtools: { enabled: true },

  // TypeScript
  typescript: {
    strict: true,
    typeCheck: true,
  },

  // CSS
  css: ['~/assets/scss/main.scss'],

  // Vite config for SCSS
  vite: {
    css: {
      preprocessorOptions: {
        scss: {
          additionalData: '@use "~/assets/scss/variables" as *; @use "~/assets/scss/mixins" as *;',
        },
      },
    },
  },

  // SSR disabled for static dashboard
  ssr: false,

  // Generate static files for embedding in Rust
  nitro: {
    output: {
      publicDir: '../src/dashboard/static',
    },
  },

  // App config
  app: {
    head: {
      title: 'ClaudeBot Dashboard',
      meta: [
        { charset: 'utf-8' },
        { name: 'viewport', content: 'width=device-width, initial-scale=1' },
        { name: 'description', content: 'ClaudeBot Management Dashboard' },
      ],
      link: [
        { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' },
      ],
    },
  },

  // Runtime config
  runtimeConfig: {
    public: {
      apiBase: 'http://127.0.0.1:8080',
    },
  },
})
