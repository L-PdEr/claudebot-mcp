# ClaudeBot Web Dashboard - Master Implementation Roadmap

> **Prompt Engineering:** Anthropic Best Practices + Industry Standards 2026
> **Method:** CIRCLE Development Pipeline + ReAct Reasoning
> **Architecture:** Zero-Trust, Local-First, Progressive Security
> **Quality:** Production-Ready, OWASP Compliant

---

## 📋 Executive Summary

| Attribute | Value |
|-----------|-------|
| **Current State** | No web interface - CLI/Telegram only |
| **Target State** | Secure dashboard with progressive security model |
| **Timeline** | 4 Epics, 16 Tasks, ~3 weeks |
| **Security Model** | Local-First → Zero-Trust Remote |
| **Tech Stack** | Axum + HTMx + Tailscale |

---

## 🎯 Updated Priority Matrix

```
┌──────────┬───────────────────────┬────────────────┬──────────────────┬───────────┐
│ Priority │        Feature        │ Business Value │ Technical Effort │    ROI    │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P0       │ WhatsApp Integration  │ HIGH           │ HIGH             │ HIGH      │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P0       │ Skill Framework       │ HIGH           │ HIGH             │ HIGH      │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P1       │ Self-Extending Skills │ HIGH           │ HIGH             │ VERY HIGH │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P1       │ Browser Automation    │ HIGH           │ MEDIUM           │ HIGH      │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P1       │ Calendar Integration  │ HIGH           │ MEDIUM           │ HIGH      │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P2       │ Email Integration     │ MEDIUM         │ MEDIUM           │ HIGH      │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P2       │ Voice Interface       │ MEDIUM         │ MEDIUM           │ MEDIUM    │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P2       │ Web Dashboard         │ MEDIUM         │ MEDIUM           │ HIGH      │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ P3       │ Docker Isolation      │ LOW            │ LOW              │ MEDIUM    │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ FUTURE   │ Discord Integration   │ LOW            │ MEDIUM           │ MEDIUM    │
├──────────┼───────────────────────┼────────────────┼──────────────────┼───────────┤
│ FUTURE   │ Slack Integration     │ LOW            │ MEDIUM           │ MEDIUM    │
└──────────┴───────────────────────┴────────────────┴──────────────────┴───────────┘
```

---

## 🔐 Security Architecture

### Access Level Decision Tree

```
                    ┌─────────────────────┐
                    │  Where is dashboard │
                    │     accessed from?  │
                    └──────────┬──────────┘
                               │
            ┌──────────────────┼──────────────────┐
            ▼                  ▼                  ▼
      ┌──────────┐      ┌──────────────┐   ┌───────────────┐
      │ Localhost│      │ Home Network │   │ Public Internet│
      │   Only   │      │    (LAN)     │   │   (Remote)     │
      └────┬─────┘      └──────┬───────┘   └───────┬───────┘
           │                   │                   │
           ▼                   ▼                   ▼
    ┌────────────┐      ┌────────────┐      ┌────────────────┐
    │ No Auth    │      │ Basic Auth │      │ Zero-Trust     │
    │ 127.0.0.1  │      │ + HTTPS    │      │ Tailscale +    │
    │ Epic 1     │      │ Epic 2     │      │ OAuth + MFA    │
    └────────────┘      └────────────┘      │ Epic 3         │
                                            └────────────────┘
```

### Layered Defense Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         INTERNET                                     │
└───────────────────────────────┬─────────────────────────────────────┘
                                │
                    ┌───────────▼───────────┐
                    │   Tailscale Funnel    │  ← Layer 1: Zero-Trust Network
                    │   (WireGuard VPN)     │    No open ports, encrypted
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   Caddy/Traefik       │  ← Layer 2: TLS Termination
                    │   (Auto HTTPS)        │    Certificate management
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   Rate Limiter        │  ← Layer 3: DoS Protection
                    │   (tower-governor)    │    Per-IP, per-user limits
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   Authentication      │  ← Layer 4: Identity
                    │   (OAuth 2.1 + MFA)   │    JWT sessions
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   Authorization       │  ← Layer 5: RBAC
                    │   (Viewer/Editor/     │    Least privilege
                    │    Admin roles)       │
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   Audit Logging       │  ← Layer 6: Accountability
                    │   (All actions)       │    Tamper-evident
                    └───────────┬───────────┘
                                │
                    ┌───────────▼───────────┐
                    │   ClaudeBot Core      │  ← Application Layer
                    │   (Rust Backend)      │
                    └───────────────────────┘
```

---

## 🏛️ Epic 1: MVP Dashboard (Localhost)

> **Priority:** P2 - Foundation
> **Security Level:** No auth (OS isolation sufficient)
> **Timeline:** Week 1-2

<epic id="E1" name="MVP Dashboard">
<overview>
Create the foundational dashboard infrastructure with localhost-only binding.
This epic establishes the server, API, and basic UI that all future features build upon.
</overview>

<security_model>
When binding to 127.0.0.1, authentication is unnecessary because:
1. Only processes on the same machine can connect
2. OS enforces user-level isolation
3. This is the standard for development servers (webpack, vite, etc.)
</security_model>

<success_metrics>
- Dashboard accessible at http://127.0.0.1:8080
- Status API responds in < 50ms
- SSE streaming works without reconnection issues
- Frontend renders correctly on mobile/desktop
</success_metrics>
</epic>

### Task D1.1: Dashboard Server Foundation

<task id="D1.1">
<prompt type="implementation">

<context>
You are implementing the web dashboard server for ClaudeBot, a Telegram bot with Claude CLI integration.
The dashboard must be secure by default, binding only to localhost.
</context>

<requirements>
1. Create Axum-based HTTP server on 127.0.0.1:8080
2. Embed static files using rust-embed
3. Implement health check endpoint
4. Configure CORS for localhost only
5. Support graceful shutdown on SIGTERM
</requirements>

<constraints>
- MUST bind to 127.0.0.1 by default (not 0.0.0.0)
- MUST use axum 0.7+ with tower ecosystem
- MUST NOT require authentication for localhost
- MUST handle shutdown gracefully
</constraints>

<output_format>
Create the following files:
- src/dashboard/mod.rs (module exports)
- src/dashboard/server.rs (HTTP server)
- src/dashboard/config.rs (configuration)

Modify:
- src/lib.rs (add dashboard module)
- Cargo.toml (add dependencies)
</output_format>

<example_api>
GET /api/health
Response: { "status": "ok", "version": "0.1.0" }
</example_api>

</prompt>

<acceptance_criteria>
- [ ] Server starts without errors
- [ ] Health endpoint returns 200
- [ ] Static files served correctly
- [ ] Cannot connect from external IP
- [ ] Graceful shutdown on Ctrl+C
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
</task>

### Task D1.2: Status & Metrics API

<task id="D1.2">
<prompt type="implementation">

<context>
The dashboard needs REST API endpoints to display system status and usage metrics.
These endpoints power the real-time dashboard UI.
</context>

<api_specification>
```yaml
endpoints:
  - path: /api/status
    method: GET
    description: System status and health
    response:
      version: string
      uptime_secs: integer
      memory_mb: integer
      bot_status: "running" | "stopped" | "error"
      api_status: "ok" | "degraded" | "down"
      last_message_at: timestamp | null

  - path: /api/metrics
    method: GET
    description: Usage statistics
    response:
      messages_today: integer
      messages_week: integer
      tokens_today: integer
      cost_today_usd: float
      cache_hit_rate: float
      avg_response_ms: integer

  - path: /api/conversations
    method: GET
    description: Recent conversations (paginated)
    query_params:
      limit: integer (default: 20, max: 100)
      offset: integer (default: 0)
    response:
      conversations: array
      total: integer
      has_more: boolean
```
</api_specification>

<performance_requirements>
- Response time: < 50ms (P95)
- Memory: No unbounded allocations
- Caching: Use in-memory cache for hot data (30s TTL)
</performance_requirements>

<error_handling>
All errors return JSON:
```json
{
  "error": "Error code",
  "message": "Human readable message",
  "details": {} // Optional additional context
}
```
</error_handling>

</prompt>

<acceptance_criteria>
- [ ] All endpoints return valid JSON
- [ ] Response times under 50ms
- [ ] Proper HTTP status codes
- [ ] Error responses follow format
- [ ] Pagination works correctly
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.1</depends_on>
</task>

### Task D1.3: Server-Sent Events (SSE)

<task id="D1.3">
<prompt type="implementation">

<context>
Real-time updates require streaming data from server to client.
SSE (Server-Sent Events) is chosen over WebSocket because:
1. Unidirectional (server → client) - perfect for dashboards
2. Native browser EventSource API with auto-reconnect
3. Works through proxies without configuration
4. Simpler to implement and maintain
</context>

<requirements>
Implement SSE endpoints:

```yaml
streams:
  - path: /api/stream/messages
    description: New messages as they arrive
    event_format:
      type: "message"
      data:
        id: string
        user: string
        content: string (truncated to 100 chars)
        timestamp: ISO8601

  - path: /api/stream/metrics
    description: Live metrics (1 event/second)
    event_format:
      type: "metrics"
      data:
        messages_today: integer
        tokens_today: integer
        active_users: integer

  - path: /api/stream/logs
    description: Log tail stream
    event_format:
      type: "log"
      data:
        level: "debug" | "info" | "warn" | "error"
        message: string
        timestamp: ISO8601
```
</requirements>

<implementation_notes>
- Send heartbeat every 30s to prevent timeout
- Use `retry: 3000` to set reconnection delay
- Limit concurrent connections per IP (max 5)
- Clean up connections on client disconnect
</implementation_notes>

<rust_example>
```rust
async fn sse_messages(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = state.message_rx.subscribe()
        .map(|msg| {
            Event::default()
                .event("message")
                .data(serde_json::to_string(&msg).unwrap())
        });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
}
```
</rust_example>

</prompt>

<acceptance_criteria>
- [ ] SSE endpoints stream data correctly
- [ ] Auto-reconnect works in browser
- [ ] Heartbeat prevents timeout
- [ ] Connections cleaned up on disconnect
- [ ] Rate limited to 5 connections/IP
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.1</depends_on>
</task>

### Task D1.4: Frontend MVP

<task id="D1.4">
<prompt type="implementation">

<context>
The dashboard frontend must be lightweight, fast-loading, and mobile-responsive.
Using HTMx for interactivity minimizes JavaScript while maintaining a modern UX.
</context>

<tech_stack>
- HTMx 2.0 (HTML over the wire, ~14KB)
- Preact (React-like, ~3KB)
- TailwindCSS (purged, ~10KB)
- Chart.js (charts, loaded async)
Total: ~30KB gzipped
</tech_stack>

<ui_specification>
```
┌─────────────────────────────────────────────────────────────────────┐
│  ClaudeBot Dashboard                              [⚙️ Settings]    │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────────┐  ┌──────────────────┐  ┌──────────────────┐  │
│  │     STATUS       │  │    MESSAGES      │  │     ACTIONS      │  │
│  │                  │  │                  │  │                  │  │
│  │  🟢 Bot: Online  │  │  Latest:         │  │  [🔄 Restart]    │  │
│  │  🟢 API: OK      │  │  @alice: Hello   │  │  [⏹️ Stop]       │  │
│  │  📊 RAM: 245 MB  │  │  @bob: Thanks!   │  │  [📝 Config]     │  │
│  │  ⏱️ Up: 3d 12h   │  │  @alice: ...     │  │  [📋 Logs]       │  │
│  │                  │  │                  │  │                  │  │
│  └──────────────────┘  └──────────────────┘  └──────────────────┘  │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                        TODAY'S METRICS                         │ │
│  │                                                                │ │
│  │   Messages  ████████████████████░░░░░░░░  847                 │ │
│  │   Tokens    ██████████████░░░░░░░░░░░░░░  125,430             │ │
│  │   Cost      ████░░░░░░░░░░░░░░░░░░░░░░░░  $0.45               │ │
│  │   Errors    █░░░░░░░░░░░░░░░░░░░░░░░░░░░  3                   │ │
│  │                                                                │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                      7-DAY TREND                               │ │
│  │      ▄                                                         │ │
│  │     ▄█▄    ▄                                                   │ │
│  │    ▄███▄  ▄█▄  ▄▄                                              │ │
│  │   ▄█████▄▄███▄▄██▄                                             │ │
│  │  ▄███████████████████▄                                         │ │
│  │  Mon  Tue  Wed  Thu  Fri  Sat  Sun                             │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```
</ui_specification>

<htmx_patterns>
```html
<!-- Auto-refresh status every 5 seconds -->
<div hx-get="/api/status"
     hx-trigger="load, every 5s"
     hx-swap="innerHTML">
  Loading...
</div>

<!-- SSE for real-time messages -->
<div hx-ext="sse"
     sse-connect="/api/stream/messages"
     sse-swap="message">
</div>

<!-- Button with loading state -->
<button hx-post="/api/restart"
        hx-indicator="#spinner"
        hx-confirm="Restart the bot?">
  <span id="spinner" class="htmx-indicator">⏳</span>
  🔄 Restart
</button>
```
</htmx_patterns>

<responsive_design>
- Mobile-first layout (320px minimum)
- Collapsible sidebar on mobile
- Touch-friendly buttons (44px minimum)
- Dark mode auto-detection via prefers-color-scheme
</responsive_design>

</prompt>

<acceptance_criteria>
- [ ] Dashboard loads in < 2 seconds
- [ ] Mobile layout works on 320px screens
- [ ] Dark mode toggles correctly
- [ ] SSE updates appear in real-time
- [ ] Charts render without errors
- [ ] All actions have feedback (loading/success/error)
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.2, D1.3</depends_on>
</task>

---

## 🏛️ Epic 2: Full Dashboard (LAN Access)

> **Priority:** P2
> **Security Level:** Basic auth + HTTPS
> **Timeline:** Week 3-4

<epic id="E2" name="Full Dashboard">
<overview>
Extend the MVP with authentication, full management capabilities, and LAN access.
This epic enables secure access from other devices on the home network.
</overview>

<security_model>
When binding to 0.0.0.0 (LAN access), authentication is REQUIRED because:
1. Other devices on the network can connect
2. Shared networks (guests, IoT) may have untrusted devices
3. Basic auth + HTTPS provides reasonable security for home use
</security_model>

<success_metrics>
- Secure login flow with password hashing
- All management features accessible via UI
- Config changes validated before applying
- Audit trail for all modifications
</success_metrics>
</epic>

### Task D2.1: Authentication System

<task id="D2.1">
<prompt type="implementation">

<context>
Authentication is required when the dashboard is accessible beyond localhost.
Implement JWT-based authentication with secure password storage.
</context>

<security_requirements>
1. Password hashing: Argon2id (memory-hard, GPU-resistant)
2. JWT tokens: 15-minute expiry, RS256 signing
3. Refresh tokens: 7-day expiry, single-use
4. Cookie storage: httpOnly, secure, sameSite=strict
5. Rate limiting: 5 login attempts per minute per IP
</security_requirements>

<api_specification>
```yaml
endpoints:
  - path: /api/auth/login
    method: POST
    body:
      username: string
      password: string
    response:
      success: boolean
      # JWT set in httpOnly cookie
    errors:
      - 401: Invalid credentials
      - 429: Too many attempts

  - path: /api/auth/logout
    method: POST
    response:
      success: boolean
    # Clears cookies, invalidates refresh token

  - path: /api/auth/refresh
    method: POST
    # Uses refresh token from cookie
    response:
      success: boolean
    errors:
      - 401: Invalid/expired refresh token

  - path: /api/auth/me
    method: GET
    headers:
      Authorization: Bearer <jwt>
    response:
      username: string
      role: "viewer" | "editor" | "admin"
      created_at: timestamp
```
</api_specification>

<password_policy>
- Minimum 12 characters
- No common passwords (have-i-been-pwned check optional)
- Stored as: Argon2id(password + per-user-salt)
</password_policy>

<jwt_structure>
```json
{
  "sub": "user_id",
  "role": "admin",
  "iat": 1706745600,
  "exp": 1706746500,
  "jti": "unique-token-id"
}
```
</jwt_structure>

</prompt>

<acceptance_criteria>
- [ ] Login sets httpOnly JWT cookie
- [ ] Invalid password returns 401
- [ ] 5+ failed logins triggers rate limit
- [ ] Logout invalidates session
- [ ] Refresh token works correctly
- [ ] Password hashed with Argon2id
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.1</depends_on>
</task>

### Task D2.2: Skill Management UI

<task id="D2.2">
<prompt type="implementation">

<context>
Enable managing installed skills through the web dashboard.
This integrates with the existing SkillRegistry in src/skills/.
</context>

<features>
1. List all installed skills with status indicators
2. Install new skill from TOML content or URL
3. Enable/disable individual skills
4. View skill execution history and stats
5. Uninstall skill with confirmation
6. Edit skill configuration (if supported)
</features>

<api_specification>
```yaml
endpoints:
  - path: /api/skills
    method: GET
    response:
      skills:
        - name: string
          version: string
          description: string
          enabled: boolean
          usage_count: integer
          success_rate: float
          last_used: timestamp | null

  - path: /api/skills
    method: POST
    body:
      type: "toml" | "url"
      content: string  # TOML content or URL
    response:
      skill:
        name: string
        version: string
    errors:
      - 400: Invalid TOML
      - 409: Skill already exists

  - path: /api/skills/:name
    method: DELETE
    response:
      success: boolean
    errors:
      - 404: Skill not found

  - path: /api/skills/:name
    method: PATCH
    body:
      enabled: boolean
    response:
      skill: { ... }
```
</api_specification>

<ui_mockup>
```
┌─────────────────────────────────────────────────────────────────────┐
│  Skills Manager                                    [+ Install New]  │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  🔍 Search skills...                                                │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 🌤️ weather                                           [ON] ⚙️ 🗑️ │ │
│  │ v1.2.0 • Get current weather for any location                  │ │
│  │ Used 89 times • 98% success • Last: 2 hours ago               │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 📅 calendar                                         [ON] ⚙️ 🗑️ │ │
│  │ v1.0.0 • Manage Google Calendar events                         │ │
│  │ Used 67 times • 100% success • Last: 5 hours ago              │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │ 🔍 web_search                                      [OFF] ⚙️ 🗑️ │ │
│  │ v0.9.0 • Search the web using DuckDuckGo                       │ │
│  │ Used 45 times • 87% success • Last: 1 day ago                 │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```
</ui_mockup>

</prompt>

<acceptance_criteria>
- [ ] Skills listed with correct status
- [ ] Install from TOML works
- [ ] Install from URL works
- [ ] Toggle enable/disable works
- [ ] Delete with confirmation works
- [ ] Error messages shown for invalid input
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.4, D2.1</depends_on>
</task>

### Task D2.3: Configuration Editor

<task id="D2.3">
<prompt type="implementation">

<context>
Allow editing bot configuration through the dashboard.
Must validate changes before applying and support hot-reload where possible.
</context>

<features>
1. View current configuration (TOML format)
2. Syntax-highlighted editor
3. Real-time validation as you type
4. Preview diff before saving
5. Automatic backup of previous config
6. Hot-reload for supported settings
</features>

<config_categories>
```yaml
hot_reloadable:  # Can change without restart
  - rate_limits
  - allowed_users
  - message_templates
  - log_level

requires_restart:  # Need bot restart
  - telegram_token
  - claude_api_key
  - database_path
  - dashboard_port

sensitive:  # Masked in UI, cannot edit via dashboard
  - telegram_token
  - claude_api_key
  - jwt_secret
```
</config_categories>

<validation_rules>
- TOML must parse correctly
- Required fields must be present
- Types must match schema
- Numeric limits enforced (e.g., rate_limit > 0)
- File paths must be valid
</validation_rules>

<dangerous_operations>
These require re-authentication:
- Changing allowed_users
- Modifying budget_limits
- Changing security settings
</dangerous_operations>

</prompt>

<acceptance_criteria>
- [ ] Config displayed correctly
- [ ] Syntax highlighting works
- [ ] Validation errors shown inline
- [ ] Diff preview before save
- [ ] Backup created on save
- [ ] Hot-reload works for supported settings
- [ ] Sensitive fields masked
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.1</depends_on>
</task>

### Task D2.4: User Management

<task id="D2.4">
<prompt type="implementation">

<context>
Manage Telegram users who interact with the bot.
Includes permissions, rate limits, and conversation history.
</context>

<user_model>
```rust
struct TelegramUser {
    user_id: i64,
    username: Option<String>,
    first_name: String,

    // Permissions
    allowed: bool,
    role: UserRole,  // user, power_user, admin

    // Limits
    daily_message_limit: u32,
    daily_token_limit: u32,

    // Stats
    total_messages: u64,
    total_tokens: u64,
    last_active: DateTime<Utc>,

    // Metadata
    created_at: DateTime<Utc>,
    notes: Option<String>,
}
```
</user_model>

<roles>
| Role | Capabilities |
|------|--------------|
| user | Basic chat, standard limits |
| power_user | Higher limits, skill usage |
| admin | Unlimited, system commands |
</roles>

<gdpr_compliance>
- Export all user data as JSON
- Delete user and all associated data
- Anonymize conversations (keep content, remove identifiers)
</gdpr_compliance>

</prompt>

<acceptance_criteria>
- [ ] User list with stats displayed
- [ ] Can block/unblock users
- [ ] Can adjust rate limits
- [ ] Can view conversation history
- [ ] Export user data works
- [ ] Delete user removes all data
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.1</depends_on>
</task>

### Task D2.5: Log Viewer

<task id="D2.5">
<prompt type="implementation">

<context>
Real-time log viewing for debugging and monitoring.
Uses SSE stream from D1.3 with added filtering capabilities.
</context>

<features>
1. Real-time log tail via SSE
2. Filter by log level (debug/info/warn/error)
3. Filter by component (telegram/claude/memory/skills)
4. Text search within logs
5. Download log file
6. Pause/resume auto-scroll
</features>

<ui_mockup>
```
┌─────────────────────────────────────────────────────────────────────┐
│  Logs                    [Level: All ▼] [Component: All ▼] [⏸️ ⬇️]  │
├─────────────────────────────────────────────────────────────────────┤
│  🔍 Search logs...                                                  │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  12:34:56 INFO  telegram  Received message from @alice              │
│  12:34:57 DEBUG claude    Building prompt (1,234 tokens)            │
│  12:34:58 INFO  claude    API response received (890 tokens)        │
│  12:34:58 DEBUG memory    Storing conversation turn                 │
│  12:34:59 INFO  telegram  Sent response to @alice                   │
│  12:35:00 WARN  skills    Skill 'weather' timeout (10s)             │
│  12:35:01 ERROR claude    API error: rate_limit_exceeded            │
│  12:35:02 INFO  claude    Retry attempt 1/3                         │
│  ...                                                                │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```
</ui_mockup>

<log_retention>
- Last 10,000 lines in memory
- Rotated log files on disk (7 days)
- Configurable retention period
</log_retention>

</prompt>

<acceptance_criteria>
- [ ] Logs stream in real-time
- [ ] Level filter works
- [ ] Component filter works
- [ ] Search highlights matches
- [ ] Download exports full log
- [ ] Pause/resume works
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.3</depends_on>
</task>

---

## 🏛️ Epic 3: Secure Remote Access

> **Priority:** P2
> **Security Level:** Zero-trust + OAuth + MFA
> **Timeline:** Week 5-6

<epic id="E3" name="Secure Remote Access">
<overview>
Enable secure dashboard access from anywhere using zero-trust networking.
This epic implements enterprise-grade security without requiring complex infrastructure.
</overview>

<security_model>
Remote access uses defense-in-depth:
1. **Network:** Zero-trust via Tailscale (no open ports)
2. **Transport:** TLS 1.3 encryption
3. **Identity:** OAuth 2.1 with PKCE
4. **MFA:** TOTP second factor
5. **Session:** Short-lived JWTs with refresh
6. **Audit:** All actions logged
</security_model>

<why_tailscale>
- No firewall configuration needed
- Works through NAT/CGNAT
- Automatic TLS certificates
- MagicDNS for easy URLs
- Free for personal use (100 devices)
</why_tailscale>
</epic>

### Task D3.1: Tailscale Integration

<task id="D3.1">
<prompt type="documentation_and_tooling">

<context>
Provide documentation and helper scripts for setting up Tailscale.
This is the recommended way to access the dashboard remotely.
</context>

<deliverables>
1. Setup guide: docs/REMOTE-ACCESS.md
2. Helper script: scripts/setup-tailscale.sh
3. Dashboard integration: Show Tailscale status and URL
</deliverables>

<setup_guide_outline>
```markdown
# Remote Access Setup Guide

## Option 1: Tailscale (Recommended)

### Prerequisites
- Tailscale account (free)
- Tailscale installed on server and client devices

### Setup Steps
1. Install Tailscale on server
2. Authenticate: `tailscale up`
3. Expose dashboard: `tailscale serve --https=443 8080`
4. Access from any device: `https://claudebot.your-tailnet.ts.net`

### Security Notes
- Traffic encrypted end-to-end
- No ports exposed to internet
- Access limited to your Tailnet

## Option 2: Cloudflare Tunnel
[...]

## Option 3: WireGuard Manual Setup
[...]
```
</setup_guide_outline>

<helper_script>
```bash
#!/bin/bash
# scripts/setup-tailscale.sh

set -e

echo "Setting up Tailscale for ClaudeBot Dashboard..."

# Check if Tailscale is installed
if ! command -v tailscale &> /dev/null; then
    echo "Installing Tailscale..."
    curl -fsSL https://tailscale.com/install.sh | sh
fi

# Check if logged in
if ! tailscale status &> /dev/null; then
    echo "Please authenticate with Tailscale:"
    sudo tailscale up
fi

# Configure serve
echo "Configuring HTTPS proxy..."
tailscale serve --bg --https=443 8080

# Get URL
HOSTNAME=$(tailscale status --json | jq -r '.Self.DNSName' | sed 's/\.$//')
echo ""
echo "✅ Dashboard available at: https://$HOSTNAME"
echo ""
echo "Access from any device on your Tailnet!"
```
</helper_script>

<dashboard_integration>
- Show Tailscale status in header
- Display external URL if available
- Warn if accessible without VPN
</dashboard_integration>

</prompt>

<acceptance_criteria>
- [ ] Documentation is clear and complete
- [ ] Helper script works on Linux
- [ ] Dashboard shows Tailscale status
- [ ] External URL displayed correctly
</acceptance_criteria>

<circle_pipeline>false</circle_pipeline>
<depends_on>D2.1</depends_on>
</task>

### Task D3.2: OAuth 2.1 + PKCE

<task id="D3.2">
<prompt type="implementation">

<context>
OAuth 2.1 with PKCE is the modern standard for authentication.
PKCE (Proof Key for Code Exchange) eliminates the need for client secrets.
</context>

<supported_providers>
1. Google (accounts.google.com)
2. GitHub (github.com)
3. Microsoft (login.microsoftonline.com) [optional]
</supported_providers>

<pkce_flow>
```
1. Client generates code_verifier (random 43-128 chars)
2. Client computes code_challenge = BASE64URL(SHA256(code_verifier))
3. Client redirects to provider with code_challenge
4. User authenticates with provider
5. Provider redirects back with authorization_code
6. Client exchanges code + code_verifier for tokens
7. Server validates code_verifier matches code_challenge
```
</pkce_flow>

<api_specification>
```yaml
endpoints:
  - path: /api/auth/oauth/start
    method: GET
    query_params:
      provider: "google" | "github"
    response:
      redirect_url: string
      state: string  # CSRF protection

  - path: /api/auth/oauth/callback
    method: GET
    query_params:
      code: string
      state: string
    response:
      # Sets JWT cookie, redirects to dashboard
    errors:
      - 400: Invalid state (CSRF detected)
      - 401: OAuth authentication failed

  - path: /api/auth/oauth/link
    method: POST
    description: Link OAuth identity to existing account
    body:
      provider: string
      code: string
    response:
      success: boolean
```
</api_specification>

<security_considerations>
- State parameter prevents CSRF
- code_verifier never leaves client until exchange
- Tokens are single-use (cannot replay authorization_code)
- Link OAuth to dashboard accounts (one user can have multiple providers)
</security_considerations>

</prompt>

<acceptance_criteria>
- [ ] Google OAuth login works
- [ ] GitHub OAuth login works
- [ ] PKCE flow implemented correctly
- [ ] State parameter validated
- [ ] Can link multiple providers to one account
- [ ] Error handling for failed auth
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.1</depends_on>
</task>

### Task D3.3: Multi-Factor Authentication

<task id="D3.3">
<prompt type="implementation">

<context>
MFA provides defense against credential theft.
TOTP (Time-based One-Time Password) is widely supported by authenticator apps.
</context>

<totp_implementation>
- Algorithm: SHA-1 (RFC 6238 standard)
- Digits: 6
- Period: 30 seconds
- Clock tolerance: ±1 period
</totp_implementation>

<api_specification>
```yaml
endpoints:
  - path: /api/auth/mfa/setup
    method: POST
    description: Generate new TOTP secret
    response:
      secret: string (base32)
      qr_code: string (data URL)
      backup_codes: string[] (10 codes)

  - path: /api/auth/mfa/enable
    method: POST
    description: Verify and enable MFA
    body:
      code: string (6 digits)
    response:
      success: boolean
    errors:
      - 400: Invalid code

  - path: /api/auth/mfa/verify
    method: POST
    description: Verify MFA during login
    body:
      code: string
    response:
      success: boolean
      # Completes login, sets JWT

  - path: /api/auth/mfa/disable
    method: POST
    description: Disable MFA (requires current code)
    body:
      code: string
    response:
      success: boolean
```
</api_specification>

<backup_codes>
- 10 single-use backup codes generated on setup
- Each code: 8 alphanumeric characters
- Stored hashed (bcrypt)
- Shown once, user must save them
</backup_codes>

<remember_device>
- Option to trust device for 30 days
- Stored as signed cookie
- Can revoke trusted devices
</remember_device>

<mfa_required_operations>
- Change password
- Add/remove OAuth provider
- Modify allowed users list
- Change budget limits
- Disable MFA
</mfa_required_operations>

</prompt>

<acceptance_criteria>
- [ ] TOTP secret generation works
- [ ] QR code displays correctly
- [ ] Verification accepts valid codes
- [ ] Verification rejects invalid codes
- [ ] Backup codes work (single-use)
- [ ] Remember device works
- [ ] Can disable MFA with valid code
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D3.2</depends_on>
</task>

### Task D3.4: Audit Logging

<task id="D3.4">
<prompt type="implementation">

<context>
Comprehensive audit logging for security compliance and debugging.
All authentication and configuration changes must be logged.
</context>

<log_schema>
```rust
struct AuditLog {
    id: Uuid,
    timestamp: DateTime<Utc>,

    // Who
    user_id: Option<String>,
    username: Option<String>,
    ip_address: IpAddr,
    user_agent: String,

    // What
    action: AuditAction,
    resource_type: String,
    resource_id: Option<String>,

    // Details
    old_value: Option<Value>,  // JSON
    new_value: Option<Value>,  // JSON

    // Result
    success: bool,
    error_message: Option<String>,
}

enum AuditAction {
    // Authentication
    Login,
    LoginFailed,
    Logout,
    MfaEnabled,
    MfaDisabled,
    PasswordChanged,

    // Configuration
    ConfigUpdated,
    SkillInstalled,
    SkillRemoved,
    UserBlocked,
    UserUnblocked,

    // Data
    ConversationDeleted,
    UserDataExported,
    BackupCreated,
}
```
</log_schema>

<storage>
- SQLite table for audit logs
- Indexed by timestamp, user_id, action
- 90-day retention (configurable)
- Append-only (no updates or deletes)
</storage>

<ui_integration>
- Audit log viewer in dashboard (admin only)
- Filter by user, action, date range
- Export as CSV/JSON
</ui_integration>

<tamper_evidence>
- SHA-256 hash of each entry
- Each entry includes hash of previous entry
- Allows verification of log integrity
</tamper_evidence>

</prompt>

<acceptance_criteria>
- [ ] All auth events logged
- [ ] All config changes logged
- [ ] Logs include IP and user agent
- [ ] Audit viewer shows entries
- [ ] Filters work correctly
- [ ] Export works
- [ ] Hash chain verified on startup
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.1</depends_on>
</task>

---

## 🏛️ Epic 4: Advanced Features

> **Priority:** P3 - Nice to Have
> **Timeline:** Week 7+

<epic id="E4" name="Advanced Features">
<overview>
Enterprise-grade features for power users and teams.
These are optional enhancements built on the solid foundation of Epics 1-3.
</overview>

<features>
- D4.1: Advanced analytics and reporting
- D4.2: Webhook configuration for integrations
- D4.3: Backup and restore functionality
- D4.4: Plugin marketplace browser
</features>
</epic>

### Task D4.1: Analytics Dashboard

<task id="D4.1">
<prompt type="implementation">

<context>
Advanced analytics with historical trends and cost analysis.
Helps users understand usage patterns and optimize costs.
</context>

<metrics>
```yaml
time_series:
  - messages_per_day
  - tokens_per_day
  - cost_per_day
  - response_time_p50
  - response_time_p95
  - error_rate

breakdowns:
  - cost_by_model
  - messages_by_user
  - usage_by_skill
  - errors_by_type
```
</metrics>

<visualizations>
1. Line chart: 7/30/90 day trends
2. Bar chart: Cost breakdown
3. Pie chart: Usage by category
4. Heatmap: Activity by hour/day
5. Table: Top users/skills
</visualizations>

<export_formats>
- CSV (raw data)
- PDF (formatted report)
- JSON (API response)
</export_formats>

</prompt>

<acceptance_criteria>
- [ ] All charts render correctly
- [ ] Date range selector works
- [ ] Data aggregates correctly
- [ ] Export works for all formats
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D1.2</depends_on>
</task>

### Task D4.2: Webhook Configuration

<task id="D4.2">
<prompt type="implementation">

<context>
Webhooks enable external integrations (Slack notifications, custom automation).
</context>

<webhook_events>
- message.received
- message.sent
- error.occurred
- budget.warning (75% of limit)
- budget.exceeded
- skill.executed
- user.blocked
</webhook_events>

<webhook_config>
```yaml
webhooks:
  - url: https://hooks.slack.com/...
    events: [error.occurred, budget.exceeded]
    secret: hmac_secret_for_verification
    enabled: true
    retry_policy:
      max_attempts: 3
      backoff_ms: [1000, 5000, 30000]
```
</webhook_config>

<security>
- HMAC signature header for verification
- TLS required (no HTTP URLs)
- Timeout: 10 seconds
- IP allowlist optional
</security>

</prompt>

<acceptance_criteria>
- [ ] Webhook creation works
- [ ] Events trigger webhooks
- [ ] HMAC signature correct
- [ ] Retry on failure works
- [ ] Test webhook feature works
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.3</depends_on>
</task>

### Task D4.3: Backup & Restore

<task id="D4.3">
<prompt type="implementation">

<context>
Full backup and restore for disaster recovery and migration.
</context>

<backup_contents>
```yaml
backup:
  config/
    - claudebot.toml
    - dashboard.toml
  skills/
    - *.toml (skill definitions)
  data/
    - memories.db
    - conversations.db
    - audit.db
  secrets/ (encrypted separately)
    - api_keys.enc
```
</backup_contents>

<encryption>
- Algorithm: AES-256-GCM
- Key derivation: Argon2id from user password
- File format: tar.gz.enc
</encryption>

<scheduled_backups>
- Daily/weekly/monthly options
- Retention: configurable (default 7 daily, 4 weekly)
- Storage: local, S3, Backblaze B2
</scheduled_backups>

</prompt>

<acceptance_criteria>
- [ ] Manual backup works
- [ ] Restore works correctly
- [ ] Encryption is secure
- [ ] Scheduled backups run
- [ ] Remote storage works
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.3</depends_on>
</task>

### Task D4.4: Plugin Marketplace

<task id="D4.4">
<prompt type="implementation">

<context>
Browse and install community skills from a central hub.
</context>

<hub_api>
```yaml
base_url: https://skills.claudebot.dev/api/v1

endpoints:
  - GET /skills?q={query}&tag={tag}&sort={popular|recent}
  - GET /skills/{id}
  - GET /skills/{id}/reviews
  - POST /skills/{id}/install (download)
```
</hub_api>

<skill_metadata>
```yaml
skill:
  id: "weather-v2"
  name: "Weather Pro"
  author: "community"
  version: "2.1.0"
  downloads: 12345
  rating: 4.8
  tags: ["utility", "weather", "api"]
  verified: true  # Reviewed by maintainers
```
</skill_metadata>

<security>
- Signature verification on download
- Sandbox execution by default
- User warning for unverified skills
- Report button for malicious skills
</security>

</prompt>

<acceptance_criteria>
- [ ] Search works correctly
- [ ] Skill details displayed
- [ ] One-click install works
- [ ] Signature verified
- [ ] Updates available shown
</acceptance_criteria>

<circle_pipeline>true</circle_pipeline>
<depends_on>D2.2</depends_on>
</task>

---

## 📊 Task Dependency Graph

```
Epic 1: MVP Dashboard (Localhost)
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│  D1.1   │────►│  D1.2   │────►│  D1.3   │────►│  D1.4   │
│ Server  │     │  API    │     │  SSE    │     │Frontend │
└─────────┘     └─────────┘     └─────────┘     └─────────┘
     │                                               │
     │          ┌────────────────────────────────────┘
     │          │
     ▼          ▼
Epic 2: Full Dashboard (LAN)
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│  D2.1   │────►│  D2.2   │     │  D2.3   │     │  D2.4   │
│  Auth   │     │ Skills  │     │ Config  │     │ Users   │
└─────────┘     └─────────┘     └─────────┘     └─────────┘
     │                                               │
     │          ┌────────────────────────────────────┤
     │          │                                    │
     │          ▼                                    ▼
     │    ┌─────────┐                          ┌─────────┐
     │    │  D2.5   │                          │  D4.2   │
     │    │  Logs   │                          │Webhooks │
     │    └─────────┘                          └─────────┘
     │
     ▼
Epic 3: Secure Remote Access
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│  D3.1   │     │  D3.2   │────►│  D3.3   │     │  D3.4   │
│Tailscale│     │ OAuth   │     │  MFA    │     │ Audit   │
└─────────┘     └─────────┘     └─────────┘     └─────────┘
                                                     │
     ┌───────────────────────────────────────────────┘
     │
     ▼
Epic 4: Advanced Features
┌─────────┐     ┌─────────┐     ┌─────────┐     ┌─────────┐
│  D4.1   │     │  D4.3   │     │  D4.4   │     │   ...   │
│Analytics│     │ Backup  │     │Marketplc│     │         │
└─────────┘     └─────────┘     └─────────┘     └─────────┘
```

---

## 🔐 Golden Rules: Dashboard Security

```
┌─────────────────────────────────┬─────────────────────────────────────────────────────┐
│              Rule               │                   Implementation                    │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 1. Local by Default             │ Bind to 127.0.0.1, not 0.0.0.0                      │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 2. No Auth ≠ No Security        │ Localhost doesn't need auth (OS provides isolation) │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 3. Zero-Trust for Remote        │ Use Tailscale/WireGuard, never expose directly      │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 4. HTTPS Always                 │ Caddy auto-HTTPS or Tailscale TLS                   │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 5. Short Sessions               │ 15 min timeout, re-auth for sensitive ops           │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 6. Rate Limit Everything        │ Per-IP, per-user limits                             │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 7. Audit All Actions            │ Log who did what, when                              │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 8. Principle of Least Privilege │ Viewer/Editor/Admin roles                           │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 9. No Secrets in Frontend       │ JWT in httpOnly cookies, not localStorage           │
├─────────────────────────────────┼─────────────────────────────────────────────────────┤
│ 10. Defense in Depth            │ Multiple layers (network → proxy → auth → app)      │
└─────────────────────────────────┴─────────────────────────────────────────────────────┘
```

---

## 🔄 Security Options Comparison

```
┌─────────────────────┬────────────┬────────────┬────────────────────────────┐
│       Option        │ Complexity │  Security  │          Best For          │
├─────────────────────┼────────────┼────────────┼────────────────────────────┤
│ Localhost Only      │ ⭐         │ ⭐⭐⭐⭐⭐ │ Single user, same machine  │
├─────────────────────┼────────────┼────────────┼────────────────────────────┤
│ Tailscale Serve     │ ⭐⭐       │ ⭐⭐⭐⭐⭐ │ Personal use, multi-device │
├─────────────────────┼────────────┼────────────┼────────────────────────────┤
│ Caddy + OAuth       │ ⭐⭐⭐     │ ⭐⭐⭐⭐   │ Team/family use            │
├─────────────────────┼────────────┼────────────┼────────────────────────────┤
│ Cloudflare Tunnel   │ ⭐⭐⭐     │ ⭐⭐⭐⭐   │ Public access with auth    │
├─────────────────────┼────────────┼────────────┼────────────────────────────┤
│ Direct Port Forward │ ⭐         │ ⭐         │ ⛔ NEVER DO THIS           │
└─────────────────────┴────────────┴────────────┴────────────────────────────┘
```

---

## 📅 Implementation Timeline

### Phase 1: MVP (Week 1-2)

| Day | Task | Description | Hours |
|-----|------|-------------|-------|
| 1 | D1.1 | Server foundation | 4h |
| 2 | D1.2 | Status & Metrics API | 4h |
| 3 | D1.3 | SSE streaming | 3h |
| 4-5 | D1.4 | Frontend MVP | 6h |
| | | **Subtotal** | **17h** |

### Phase 2: Full Dashboard (Week 3-4)

| Day | Task | Description | Hours |
|-----|------|-------------|-------|
| 6 | D2.1 | Authentication | 4h |
| 7 | D2.2 | Skill management | 4h |
| 8 | D2.3 | Config editor | 4h |
| 9 | D2.4 | User management | 4h |
| 10 | D2.5 | Log viewer | 3h |
| | | **Subtotal** | **19h** |

### Phase 3: Remote Access (Week 5-6)

| Day | Task | Description | Hours |
|-----|------|-------------|-------|
| 11 | D3.1 | Tailscale integration | 2h |
| 12-13 | D3.2 | OAuth 2.1 + PKCE | 6h |
| 14 | D3.3 | MFA | 4h |
| 15 | D3.4 | Audit logging | 4h |
| | | **Subtotal** | **16h** |

### Phase 4: Advanced (Week 7+)

| Task | Description | Hours |
|------|-------------|-------|
| D4.1 | Analytics dashboard | 6h |
| D4.2 | Webhooks | 4h |
| D4.3 | Backup & restore | 6h |
| D4.4 | Plugin marketplace | 8h |
| | **Subtotal** | **24h** |

**Total Estimated: ~76 hours**

---

## ✅ Success Criteria

**Dashboard is production-ready when:**

| # | Criterion | Epic |
|---|-----------|------|
| 1 | MVP serves on localhost without auth | E1 |
| 2 | Real-time updates via SSE work | E1 |
| 3 | Authentication required for LAN access | E2 |
| 4 | Skills can be managed via UI | E2 |
| 5 | Configuration can be edited safely | E2 |
| 6 | Tailscale provides secure remote access | E3 |
| 7 | OAuth + MFA protect remote sessions | E3 |
| 8 | All actions are audit logged | E3 |
| 9 | Rate limiting prevents abuse | E2 |
| 10 | No security warnings from OWASP checks | All |

---

## 🛠️ Tech Stack

### Backend
| Component | Technology | Purpose |
|-----------|------------|---------|
| Framework | Axum 0.7+ | Async HTTP, tower ecosystem |
| Auth | jsonwebtoken + argon2 | JWT, password hashing |
| Database | SQLite | Existing, embedded |
| Streaming | SSE | Real-time updates |
| Static Files | rust-embed | Embedded assets |

### Frontend
| Component | Technology | Purpose |
|-----------|------------|---------|
| Framework | HTMx 2.0 | HTML over the wire |
| Components | Preact | Lightweight React |
| Styling | TailwindCSS | Utility-first CSS |
| Charts | Chart.js | Visualizations |
| Total Size | ~30KB gzipped | Fast loading |

### Security
| Component | Technology | Purpose |
|-----------|------------|---------|
| TLS | Caddy / Tailscale | Auto certificates |
| Rate Limit | tower-governor | DoS protection |
| OAuth | oauth2 crate | External auth |
| MFA | totp-rs | TOTP implementation |

---

## 📁 File Structure

```
src/
├── dashboard/
│   ├── mod.rs              # Module exports
│   ├── server.rs           # Axum HTTP server
│   ├── config.rs           # Dashboard configuration
│   ├── auth.rs             # JWT + OAuth + MFA
│   ├── audit.rs            # Audit logging
│   ├── sse.rs              # Server-Sent Events
│   ├── api/
│   │   ├── mod.rs
│   │   ├── status.rs       # GET /api/status
│   │   ├── metrics.rs      # GET /api/metrics
│   │   ├── conversations.rs# Conversation CRUD
│   │   ├── skills.rs       # Skill management
│   │   ├── config.rs       # Configuration API
│   │   ├── users.rs        # User management
│   │   └── auth.rs         # Authentication endpoints
│   └── static/             # Embedded frontend
│       ├── index.html
│       ├── app.js
│       └── style.css
docs/
├── ROADMAP-WEB-DASHBOARD.md  # This file
├── REMOTE-ACCESS.md          # Tailscale/VPN setup
scripts/
├── setup-tailscale.sh        # Tailscale helper
```

---

## 📚 References

| Resource | URL |
|----------|-----|
| Open WebUI | https://github.com/open-webui/open-webui |
| LiteLLM | https://docs.litellm.ai/ |
| Tailscale Serve | https://tailscale.com/kb/1242/tailscale-serve |
| OAuth 2.1 | https://oauth.net/2.1/ |
| HTMx | https://htmx.org/ |
| Axum | https://docs.rs/axum/latest/axum/ |
| OWASP Top 10 | https://owasp.org/Top10/ |

---

*Created: 2026-01-31*
*Last Updated: 2026-01-31*
*Status: Ready for Execution*
*Version: 2.0*
