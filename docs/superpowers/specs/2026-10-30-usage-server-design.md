# Claude Usage Server and Dashboard: Design

Date: 2026-10-30

## Intent

The Claude Usage Monitor desktop app shows each person's Claude Code context usage on their own machine. The team wants that data in one place. Every 30 minutes, the app sends it to a server. The server stores it in Postgres, and a web dashboard shows usage per person, per session and per model over time.

## Goals

- Every team member's app sends usage to the server every 30 minutes, without anyone logging in inside the app.
- **All Claude Code usage is counted**, including work done while the app was closed and the minutes before a shutdown, dated by when the requests happened.
- Postgres keeps history, so the dashboard can chart usage over days and weeks.
- Admins see everyone's usage in the dashboard; each person sees only their own.
- The dashboard has four views in v1: Team overview, Person detail, Session list and Model breakdown.
- Postgres runs locally in Docker. In v1, the whole system runs on one PC (localhost).

## Non-goals

- LAN or production deployment, TLS, Kubernetes, and CI for the new projects. v1 is localhost only; team access comes later.
- Multi-tenant companies. There is one team.
- Live, sub-minute updates. The data is at most 30 minutes old by design.
- Cost or billing estimates.
- Alerts, notifications and email, including password-reset email. Admins reset passwords.

## Anti-goals (failure even if it "works")

- Double counting. Restarts, retries, or two copies of the app sending the same data must not inflate the totals.
- Losing usage because the app was closed, the PC shut down or the server was down.
- Storing plaintext API keys.
- A member seeing another member's data.

## Projects and locations

| Piece | Location | Source |
|-------|----------|--------|
| Desktop app (existing) | `C:\Code\Claude Usage Monitor` | Rust, eframe/egui |
| Server | `C:\Code\claude-usage-server` | Copy of `C:\Code\venturo-skeleton-go`, its own git repo |
| Dashboard | `C:\Code\claude-usage-dashboard` | Copy of `C:\Code\venturo-skeleton-react`, its own git repo |

Copy the source files only, never `.git`, `node_modules`, `bin`, `.env`, `dist` or build output. Each new project gets a fresh `git init`.

## Decisions made with the user

- Senders are team members with **no login in the app**. Each person has a user account and an **API key**, and the key in the app identifies them.
- Each person's dashboard account is the same user that owns their API key. Admins see everyone; members see only their own data.
- **Count all usage**, not just usage while the app runs.
- **Keep history** so usage over time can be charted.
- Database: **Postgres**, run **locally in Docker**.
- v1 runs on **localhost only**.
- **Strip** the unused parts of the Go skeleton, including **Redis**.
- The dashboard has all four views.

## How usage is counted

The app already reads every transcript in full on startup and follows new lines as they are written. Besides the per-run New context it shows in its UI, it keeps a lifetime record per session that is never reset:

- `hours`: for each (UTC hour of the request timestamp, model), the context growth and the request count. Growth follows the app's existing rules: deduplicate by message id, skip sidechains, don't count the first request after `/compact` or `/model`, and treat a drop as a compaction.
- `compactions_lifetime`: all compactions in the session.

Each send carries **absolute values**, and the server **upserts** them (replaces the row). This is why:

- **Retries are harmless.** The same values overwrite the same rows.
- **Restarts lose nothing.** The app re-reads the transcripts and rebuilds the same lifetime values.
- **Server downtime loses nothing.** The next successful send carries everything.
- **Two copies of the app are harmless.** Both write identical values.
- **Backfill is dated correctly.** The first send includes past sessions, and their growth lands in the hours when it happened.

The in-app New context column (growth since the app started) is unchanged.

### What the app sends

The app remembers a **watermark**: the largest `latest_at` it has successfully sent, stored in `sync_state.json` in the app's config directory.

- Each tick, it sends every session whose `latest_at` is at or after the watermark. On the first run there is no watermark, so it sends everything (backfill).
- Only sessions with lifetime growth > 0 are sent.
- Sessions go in chunks of up to 200 per request.
- A request also stays under 20,000 hour rows in total; a chunk is cut early when the next session would exceed it.
- The watermark advances only after every chunk succeeds.

## Database (Postgres)

### Kept from the skeleton `core` schema

`users`, `refresh_tokens`, the RBAC tables, `api_keys` and `audit_logs`.

- `company_id` and everything else about companies, branches, approvals, translations and clients are removed.
- Migrations are renumbered into a clean sequence; there is no data to migrate.
- A seeded admin user's credentials come from `.env` (`SEED_ADMIN_EMAIL`, `SEED_ADMIN_PASSWORD`).

### `usage.sessions`

One row per Claude Code session, holding the latest state. It is upserted on every send.

```sql
CREATE SCHEMA IF NOT EXISTS usage;

CREATE TABLE usage.sessions (
    id                  TEXT PRIMARY KEY,
    user_id             UUID NOT NULL REFERENCES core.users(id) ON DELETE CASCADE,
    machine             VARCHAR(255) NOT NULL,
    project             TEXT NOT NULL DEFAULT '',
    title               TEXT NOT NULL DEFAULT '',
    model               VARCHAR(100) NOT NULL,
    context_window      BIGINT NOT NULL DEFAULT 0,
    start_context       BIGINT NOT NULL DEFAULT 0,
    latest_context      BIGINT NOT NULL DEFAULT 0,
    peak_context        BIGINT NOT NULL DEFAULT 0,
    new_context         BIGINT NOT NULL DEFAULT 0,
    requests            INT NOT NULL DEFAULT 0,
    compactions         INT NOT NULL DEFAULT 0,
    first_request_at    TIMESTAMPTZ NOT NULL,
    last_request_at     TIMESTAMPTZ NOT NULL,
    last_reported_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_usage_sessions_user_last ON usage.sessions (user_id, last_request_at DESC);
CREATE INDEX idx_usage_sessions_last ON usage.sessions (last_request_at DESC);
CREATE INDEX idx_usage_sessions_model ON usage.sessions (model);
```

- `id` is the Claude Code session UUID string.
- `new_context` is the session's lifetime growth, equal to the sum of its hourly rows.
- `start_context` is the session's first request context.

### `usage.session_hours`

Usage history: growth and request count per session, per UTC hour, per model.

```sql
CREATE TABLE usage.session_hours (
    session_id          TEXT NOT NULL REFERENCES usage.sessions(id) ON DELETE CASCADE,
    hour                TIMESTAMPTZ NOT NULL,
    model               VARCHAR(100) NOT NULL,
    user_id             UUID NOT NULL REFERENCES core.users(id) ON DELETE CASCADE,
    new_context         BIGINT NOT NULL DEFAULT 0,
    requests            INT NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, hour, model)
);

CREATE INDEX idx_usage_session_hours_user_hour ON usage.session_hours (user_id, hour);
CREATE INDEX idx_usage_session_hours_hour ON usage.session_hours (hour);
CREATE INDEX idx_usage_session_hours_model_hour ON usage.session_hours (model, hour);
```

- `hour` is the request timestamp truncated to the hour, in UTC.
- `user_id` is copied from the session so time-range queries need no join.
- Ingest uses `INSERT ... ON CONFLICT (session_id, hour, model) DO UPDATE SET new_context = EXCLUDED.new_context, requests = EXCLUDED.requests`.

### Example report query

New context per person per day:

```sql
SELECT h.user_id,
       date_trunc('day', h.hour AT TIME ZONE $tz) AS day,
       SUM(h.new_context) AS new_context
FROM usage.session_hours h
WHERE h.hour >= $from AND h.hour < $to
GROUP BY 1, 2
ORDER BY 2, 1;
```

## Server API (`claude-usage-server`)

Base path `/usage/v1`. Responses use the skeleton's `internal/shared/response` envelope. The server binds to localhost, and CORS allows `http://localhost:<dashboard port>`.

### Ingest (API key)

`POST /usage/v1/sessions`, header `X-API-Key: <key>`.

```json
{
  "machine": "DIMAS-PC",
  "app_version": "0.2.0",
  "sessions": [
    {
      "session_id": "9101114e-543c-4bb6-a36b-0d702dbc23d0",
      "project": "C:\\Code\\Claude Usage Monitor",
      "title": "Usage server design",
      "model": "claude-opus-5-5",
      "context_window": 1000000,
      "start_context": 21000,
      "latest_context": 180100,
      "peak_context": 210000,
      "new_context": 64000,
      "requests": 412,
      "compactions": 1,
      "first_at": "2026-10-30T02:11:09.123Z",
      "latest_at": "2026-10-30T06:58:40.456Z",
      "hours": [
        { "hour": "2026-10-30T02:00:00Z", "model": "claude-opus-5-5", "new_context": 40000, "requests": 120 },
        { "hour": "2026-10-30T06:00:00Z", "model": "claude-sonnet-5-5", "new_context": 8000, "requests": 30 }
      ]
    }
  ]
}
```

- Each session's upserts into `usage.sessions` and `usage.session_hours` run in one transaction.
- The response is `{ "accepted": n, "rejected": [{ "session_id": "...", "reason": "..." }] }`.
- Limits: at most 200 sessions per payload and 2,000 hour rows per session. String fields are length-checked against the column sizes. Numbers must be ≥ 0.
- If a `session_id` already belongs to a different user, that session is rejected and the rest of the payload is accepted.
- `api_keys.last_used_at` is updated on every call; the dashboard uses it for "last synced".

### Read (JWT login)

If the caller is not an admin, the server forces `user_id` to the caller's own ID and ignores any `user_id` they pass. All endpoints accept `from`, `to` (RFC 3339) and `tz` (IANA name, default `Asia/Jakarta`).

| Endpoint | Returns | View |
|----------|---------|------|
| `GET /usage/v1/overview` | Per person: new context, sessions touched, requests, last request time, last synced, current model. Plus team totals. | Team overview |
| `GET /usage/v1/timeseries?interval=hour\|day&group_by=user\|model&user_id=` | Buckets of `SUM(new_context)` and `SUM(requests)` | Overview chart, Person detail chart |
| `GET /usage/v1/models?user_id=` | Per model: new context, requests and number of sessions | Model breakdown |
| `GET /usage/v1/sessions?user_id=&project=&model=&q=&page=&per_page=&sort=` | Paged `usage.sessions` rows with the user's name | Session list |
| `GET /usage/v1/sessions/:id` | The session plus its hourly rows | Session drill-down |
| `GET /usage/v1/people` | List of users (admins only) | Person picker |

### Admin (JWT, admin)

- Create, list and deactivate users; reset a user's password.
- **Issue an API key for another user**, and list or revoke a user's keys. This is a new endpoint; the skeleton only lets callers create keys for themselves. The full key is shown once.
- Deactivating a user also revokes their keys.

## Dashboard (`claude-usage-dashboard`)

Built on the React skeleton's stack: Vite, TypeScript, MUI, `@mui/x-charts`, `@mui/x-data-grid` and axios. Its login flow is kept and pointed at the server. Company switching, Google sign-in and Firebase are removed.

- **Team overview** (admin home): KPI tiles for total new context, requests, people and sessions in range; a stacked bar chart of new context per day per person; a people table showing last synced, with a link to each person's detail page.
- **Person detail** (member home, showing themselves): their KPI tiles, a daily new-context chart split by model, and their recent sessions.
- **Session list**: a data grid filtered by person, project, model, date range and text search. A row opens the session drill-down (an hourly chart of new context split by model).
- **Model breakdown**: a donut chart of share per model plus a stacked chart of model usage over time.
- **Admin › People**: create users, reset passwords, deactivate users, and issue or revoke API keys. The new key is shown once with a copy button.
- A date-range picker (default: last 7 days) is shared across views. `tz` is the browser's time zone.

## Desktop app changes (`Claude Usage Monitor`)

- `Session` gains lifetime fields that `reset_baseline` never clears: `hours: BTreeMap<(String, String), HourUsage>` keyed by (UTC hour, model), with `new_context` and `requests`, plus `compactions_lifetime`. `record_request` and the compaction paths update them alongside the existing counters.
- **Settings panel** (a gear in the top bar) with **Server URL** and **API key** fields, saved to `config.json` in `dirs::config_dir()/claude-usage-monitor/`. The key field is masked. The environment variables `CLAUDE_USAGE_SERVER_URL` and `CLAUDE_USAGE_API_KEY` override the file.
- A new `sync.rs` builds the payload and sends it with `ureq` (blocking, rustls).
  - It runs on the existing 30-minute export tick in `watcher.rs`, plus once about 1 minute after startup to backfill.
  - The timeout is 15 seconds.
  - There is no retry queue: the watermark makes the next tick resend.
- The UI status line shows "Synced 12:30" or "Sync failed: <reason>".

## Local environment (Docker)

`claude-usage-server/docker-compose.yml`:

```yaml
services:
  postgres:
    image: postgres:17-alpine
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres
      POSTGRES_DB: claude_usage
      TZ: UTC
    ports: ["127.0.0.1:5432:5432"]
    volumes: ["pgdata:/var/lib/postgresql/data"]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres -d claude_usage"]
      interval: 5s
      retries: 10
volumes:
  pgdata:
```

- `.env.example` points at `DB_NAME=claude_usage`, and the `REDIS_*`, SMTP, GCS and Firebase variables are removed.
- Add `make up` / `make down` targets for `docker compose up -d` / `down`.
- `make db-setup` then runs migrations and seeders as in the skeleton.
- If port 5432 is already taken locally, use a different host port and set `DB_PORT` to match.

## Calls made by Claude (user can override)

- Removing the companies module means one team.
- Redis is dropped: permission checks read Postgres directly, which is enough at this scale. Remove `internal/shared/redis`, go-redis, the `REDIS_*` variables and the Redis connection in `router.go`.
- Map "admin" and "member" onto the skeleton's level-based RBAC rather than inventing new permission tables.
- History is stored as hourly buckets per session and model, not as raw 30-minute snapshots. They give exact dates for backfill and make upserts idempotent.
- Time grouping defaults to `Asia/Jakarta` when the dashboard doesn't pass `tz`.
- "Last synced" comes from `api_keys.last_used_at`; there is no heartbeat table.

## Left to the builder

- Exact file layout inside the new `usage` module, following the skeleton's `domain/dto/handler/repository/service/main.usage.go` pattern.
- Which skeleton files and env variables to delete when stripping. The test is "nothing references companies, branches, approvals, translations, clients, firebase, storage, email, discord or redis".
- How admin and member map onto the skeleton's RBAC levels.
- Chart styling, colors and empty states in the dashboard.
- Tests:
  - Ingest idempotency: the same payload twice gives the same totals.
  - The ownership conflict.
  - The member-only filter on read endpoints.
  - Rust tests for the hourly buckets (restart, `/model`, `/compact`) and for the watermark.
- API contract docs in `docs/api-contract/usage/`, following the skeleton's `docs/api-contract` style.
