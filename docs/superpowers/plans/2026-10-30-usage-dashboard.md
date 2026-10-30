# Usage Dashboard and Full-Stack Docker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `C:\Code\claude-usage-dashboard`, a stripped copy of `venturo-skeleton-react`, with Team overview, Person detail, Session list with drill-down, Model breakdown and Admin › People. Then run Postgres, the API and the dashboard together with one `docker compose up -d`, so the user only opens `http://localhost:3000`.

**Architecture:**
- The React skeleton's conventions stay: the `src/module/<domain>/features/<feature>` layout, the axios instance with `endpoints`, `t()` i18n, MUI `sx`, React Context, and no data libraries.
- A new `usage` module calls the server's `/usage/v1/*` and `/core/v1/admin/*` routes.
- The full stack lives in `C:\Code\claude-usage-server\docker-compose.yml` with the services `postgres`, `api` and `dashboard`, all `restart: unless-stopped` and all published only on `127.0.0.1`.

**Tech Stack:** Vite, TypeScript, React, MUI 7, `@mui/x-charts`, `@mui/x-data-grid`, `@mui/x-date-pickers`, axios, vitest, yarn 1.22. Node 22 runs **only in Docker**: the host has Node 20.19 and no yarn, so every yarn command runs as `docker run --rm -v "C:/Code/claude-usage-dashboard:/app" -w /app node:22-alpine sh -c "corepack enable && yarn <cmd>"`, or an equivalent wrapper script.

**Spec:** `docs/superpowers/specs/2026-10-30-usage-server-design.md`, sections "Dashboard" and "Server API". The server API contract is `C:\Code\claude-usage-server\docs\api-contract\usage\README.md` plus `docs\api-contract\core\auth.md`.

## Global Constraints

- **New code has no comments of any kind.** The skeleton's own rules (`CLAUDE.md`, `.claude/rules/`, `docs/`) apply otherwise: `src/...` imports, every user-visible string through `t()`, no `console.log`, and Iconify names only from `icon-sets.ts`.
- Keep one locale, English. Remove the other language packs if that's simpler than maintaining them.
- One-line commit messages with no co-author trailer; commit only when the user asks.
- API base URL is `http://localhost:8080`. The dashboard is served at `http://localhost:3000`, which the server's CORS already allows.
- Sign-in posts `{email, password}` to `/core/v1/auth/signin`. The server has no signup, Google, Firebase, company switching or email reset, so their UI is removed.
- An admin is `user.is_super_admin === true` from `/core/v1/auth/me`. Admins get every view plus Admin › People. Members get Person detail (themselves), Session list and Model breakdown, all scoped by the server.
- The `tz` query parameter is `Intl.DateTimeFormat().resolvedOptions().timeZone`. The date-range picker defaults to the last 7 days, and `from`/`to` are sent as UTC `Z` strings.
- Token counts are shown like the desktop app: `48.0K`, `1.2M`. Timeseries buckets come back as offset instants such as `2026-10-30T00:00:00+07:00`.
- `yarn typecheck`, `yarn test` and `yarn build` pass inside Docker. `yarn lint` shows no errors in changed files.
- All Docker ports bind to `127.0.0.1`.

## Review Focus

- **A member opens an admin URL directly:** they're redirected away or get 403 UI, never other people's data. Test: `adminGuardRedirectsMembers` (Task 1).
- **An empty database (fresh install):** every view shows an empty state, not an error or a blank chart. Test: `overviewShowsEmptyState` (Task 3).
- **An expired access token:** the skeleton's axios refreshes, and a revoked or deactivated user goes to sign-in. Covered by the skeleton; check it manually in Task 6.
- **A new API key is shown only once:** after the dialog closes, it can't be retrieved again. Test: `issuedKeyShownOnce` (Task 5).
- **The API is down:** pages show the axios `err.message`, not a blank screen. Check manually in Task 6.

---

### Task 1: Copy, strip and wire auth to the server

**Files:**
- Create `C:\Code\claude-usage-dashboard\` by running `robocopy C:\Code\venturo-skeleton-react C:\Code\claude-usage-dashboard /E /XD .git node_modules dist .claude`.
- Modify `package.json` (name `claude-usage-dashboard`, version `0.1.0`), `.env*` and `.env.prod` (`VITE_SERVER_URL` or whatever key the skeleton uses = `http://localhost:8080`, app name "Claude Usage"), the auth module, the nav config, routes, and `src/shared/config`.
- Remove: `firebase` and Google sign-in; signup, forgot and reset password pages; company switching and branches features; `_mock` usage in live pages; the demo dashboard pages.

**Interfaces:**
- Produces: `useAuthContext()`, exposing `user` with `is_super_admin: boolean`, `full_name` and `email`.
- Produces: an `AdminGuard` route wrapper.
- Produces: route paths `paths.dashboard.overview`, `paths.dashboard.person(id?)`, `paths.dashboard.sessions`, `paths.dashboard.session(id)`, `paths.dashboard.models` and `paths.dashboard.adminPeople`.
- After sign-in, admins land on `overview` and members on `person`.

- [ ] **Step 1:** Copy the skeleton and `git init`. Run `yarn install` in Docker and confirm the untouched copy passes `yarn typecheck`.
- [ ] **Step 2: Write the failing test `adminGuardRedirectsMembers`.** Render a route under `AdminGuard` with a member user and expect a redirect to `person`. With an admin, expect the children to render.
- [ ] **Step 3:** Strip the listed parts and wire sign-in, `me` and refresh to the server. Implement `AdminGuard` and the nav (admin items hidden for members).
- [ ] **Step 4:** `yarn typecheck`, `yarn test` and `yarn build` are green. `grep -ri "firebase\|company\|branch" src` finds no live code.

### Task 2: Usage API layer and shared formatting

**Files:** Modify `src/shared/lib/axios.ts` (add `endpoints.usage.*` and `endpoints.admin.*`). Create `src/module/usage/api/usage.ts`, `src/module/usage/types.ts`, `src/module/usage/utils/format.ts`, `src/module/usage/hooks/use-report-range.tsx` (a context with `{from, to, tz}` and a setter, defaulting to the last 7 days), plus tests.

**Interfaces:**
- Produces: `formatTokens(n: number): string` (`0`, `950`, `48.0K`, `1.2M`) and `formatBucket(iso: string, interval: 'hour' | 'day'): string`.
- Produces: `getOverview(range)`, `getTimeseries(range, interval, groupBy, userId?)`, `getModels(range, userId?)`, `getSessions(range, query)`, `getSession(id)`, `getPeople()`, `adminListUsers()`, `adminCreateUser(body)`, `adminSetActive(id, active)`, `adminResetPassword(id, password)`, `adminIssueKey(userId, name)`, `adminListKeys(userId)` and `adminRevokeKey(keyId)`. Types mirror the API contract's JSON.

- [ ] **Step 1: Write the failing tests.**
  - `formatTokens` cases: `0 → "0"`, `950 → "950"`, `48000 → "48.0K"`, `1_200_000 → "1.2M"`.
  - `buildRangeParams`: given `{from, to, tz}`, it produces `from`/`to` as `Z` ISO strings and passes `tz` through.
- [ ] **Step 2:** Implement it. Tests pass.

### Task 3: Team overview and Person detail

**Files:** Create `src/module/usage/features/overview/*` and `src/module/usage/features/person/*`, plus the routes.

**What the pages show:**
- **Overview** (admin):
  - A date-range picker.
  - KPI tiles: total new context, requests, active people, sessions.
  - A stacked bar chart from `timeseries?interval=day&group_by=user` (x = day, series = person).
  - A people table with columns name, new context, requests, sessions, current model, last request and last synced. A row click opens Person detail.
- **Person detail:**
  - KPI tiles.
  - A daily chart from `timeseries?interval=day&group_by=model&user_id=`.
  - The 10 most recent sessions (`sessions?user_id=&per_page=10`), linking to the session list.
  - Members see themselves, and the person picker is admin-only.

- [ ] **Step 1: Write the failing test `overviewShowsEmptyState`.** With a mocked API returning zero people and no points, the page renders an empty-state message and no chart.
- [ ] **Step 2:** Implement both pages. Tests pass, and typecheck and build are green.

### Task 4: Session list, drill-down, Model breakdown

**Files:** Create `src/module/usage/features/sessions/*` and `src/module/usage/features/models/*`, plus the routes.

**What the pages show:**
- **Sessions:**
  - A DataGrid with server pagination and sorting (`last_request_at` or `new_context`, ±).
  - Filters: person (admin only), project, model, text `q`, and the date range.
  - Columns: title, person, project, model, new context, latest/peak context, requests, compactions, last request.
- **Session drill-down:**
  - Header facts.
  - An hourly stacked bar chart of `hours` by model.
- **Models:**
  - A donut chart of `new_context` share per model.
  - A stacked chart from `timeseries?group_by=model`.
  - A table with model, new context, requests and sessions.

- [ ] **Step 1: Write the failing test `sessionsQueryFromGrid`.** Grid state `{page: 2, pageSize: 25, sort: new_context desc, filters}` maps to API params `page=3&per_page=25&sort=-new_context&...`.
- [ ] **Step 2:** Implement it. Tests pass, and build is green.

### Task 5: Admin › People

**Files:** Create `src/module/usage/features/admin-people/*` and the route behind `AdminGuard`.

**What the page shows:**
- A user table with name, email, role, active and last synced.
- Actions: create member (a dialog with email, username, full name and password), reset password (a dialog), deactivate/activate (with a confirm), and API keys (a drawer listing keys with prefix, created, last used and status, plus a revoke action).
- "Issue key" opens a dialog that shows the full key **once**, with a copy button and the warning "This key will not be shown again". Closing it drops the key from state.

- [ ] **Step 1: Write the failing test `issuedKeyShownOnce`.** The mocked issue call returns `wiz_live_x_y`. The dialog shows it; after closing and reopening the drawer, no element contains `wiz_live_x_y`.
- [ ] **Step 2:** Implement it. Tests pass, and build is green.

### Task 6: Full-stack Docker and an end-to-end check

**Files:** Modify `C:\Code\claude-usage-server\docker-compose.yml`, `C:\Code\claude-usage-server\Dockerfile` (drop the `pkg/email/templates` COPY and fix the `EXPOSE`/healthcheck port to 8080), `deployments/docker/docker-entrypoint.sh` (make sure it runs migrations and then the idempotent seeders using `SEED_ADMIN_*` through `psql -v`), `C:\Code\claude-usage-server\README.md`, and the dashboard's `.env.prod`, `Dockerfile` and `nginx.conf` if needed.

**Interfaces (compose):**
- `postgres`: the same as now (`127.0.0.1:5432`), plus `restart: unless-stopped`.
- `api`:
  - `build: .`
  - `env_file: .env`, with the overrides `DB_HOST=postgres` and `SERVER_HOST=0.0.0.0`.
  - `ports: ["127.0.0.1:8080:8080"]`
  - `depends_on: postgres` with `condition: service_healthy`
  - `restart: unless-stopped`
- `dashboard`:
  - `build: ../claude-usage-dashboard`
  - `ports: ["127.0.0.1:3000:80"]`
  - `depends_on: api`
  - `restart: unless-stopped`

- [ ] **Step 1:** Stop any host-run server on port 8080. Run `docker compose up -d --build --wait`, and all 3 services are healthy or running.
- [ ] **Step 2: End-to-end check, in a browser or with curl.**
  - `http://localhost:3000` serves the app.
  - Signing in as the seeded admin works.
  - Admin › People creates a member and issues a key.
  - Ingest `testdata/ingest.json` with that key, and the overview chart and tables show 48.0K.
  - Signing in as the member shows only their data, and opening `/dashboard/admin/people` redirects.
  - Stopping the `api` container makes the pages show an error message, not a blank screen.
- [ ] **Step 3:** Clean the main DB of end-to-end data: no usage sessions, no keys, only the admin user. Document "open http://localhost:3000" in both READMEs.
