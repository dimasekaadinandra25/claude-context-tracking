# Claude Usage Server Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `C:\Code\claude-usage-server`, a stripped copy of `venturo-skeleton-go`. It stores Claude Code usage that desktop apps send with API keys in Postgres, running locally in Docker, and serves the dashboard's read endpoints.

**Architecture:** Copy the skeleton, then delete everything about companies, branches, approvals, translations, clients, firebase, storage, email, discord and Redis. Keep auth, users, roles and api_keys. Add a `usage` module (`domain/dto/handler/repository/service/main.usage.go`) holding two tables: `usage.sessions` and `usage.session_hours`. Ingest is an idempotent upsert of absolute values. Reads are scoped: an admin (`super_admin`) sees everyone, and anyone else sees only themselves.

**Tech Stack:** Go 1.26 (the toolchain auto-downloads), Gin, pgx/v5, golang-migrate CLI, Postgres 17 in Docker Compose.

**Spec:** `C:\Code\Claude Usage Monitor\docs\superpowers\specs\2026-10-30-usage-server-design.md`

## Global Constraints

- Project path `C:\Code\claude-usage-server`, Go module name `claude-usage-server`, fresh `git init`. Never copy `.git`, `bin`, `tmp`, `.env` or `.claude`.
- New code has **no comments of any kind**. Leave the comments in kept skeleton files as they are.
- Commit messages are one line with **no Co-Authored-By trailer**. Commit only when the user asks.
- `gofmt -l .` prints nothing, and `go vet ./...` and `go build ./...` pass after every task.
- DB: `postgres:17-alpine`, user and password `postgres`/`postgres`, database `claude_usage`, host port 5432, `TZ=UTC`. There is no Redis.
- The server binds `127.0.0.1:${SERVER_PORT}`. CORS allows only `http://localhost:5173` and `http://localhost:3000`.
- Migration **file names use timestamps** (`YYYYMMDDHHMMSS_name.up.sql`), not sequence numbers (user decision). `make migrate-create` uses `-format 20060102150405`. Core migrations are `20261030000001_init_schema` … `20261030000005_audit_logs`, and usage is `20261030000006_usage`. Columns stay `TIMESTAMPTZ`.
- `psql` isn't installed on the host. Every SQL seeder runs through `docker compose exec -T postgres psql`.
- Ingest limits: ≤ 200 sessions per payload, ≤ 2,000 hour rows per session, all numbers ≥ 0, strings within column sizes.
- The default `tz` is `Asia/Jakarta`.
- Roles: `super_admin` is the admin and bypasses all checks. `member` has permissions `{"usage":"viewer"}`.

## Review Focus

- **Same payload sent twice:** totals don't change, and the hour rows are replaced, not added. Test: `TestIngestIsIdempotent` (Task 4).
- **A session id already owned by another user:** that session is rejected, the rest is accepted, and the owner's data is untouched. Test: `TestIngestRejectsForeignSession` (Task 4).
- **A member passes `user_id=<other>` on any read:** they still get only their own data. Test: `TestMemberCannotReadOthers` (Task 5).
- **A deactivated user's API key:** returns 401 on ingest. Test: `TestInactiveUserKeyRejected` (Task 6).
- **A `tz` near day boundaries:** usage at 23:30 UTC lands on the next day for `Asia/Jakarta`, and an invalid `tz` returns 400. Test: `TestTimeseriesUsesTimeZone` (Task 5).

---

### Task 1: Copy the skeleton and run Postgres in Docker

**Files:**
- Create: `C:\Code\claude-usage-server\` (copy), `docker-compose.yml`, `.env` (from `.env.example`)
- Modify: `go.mod` (module name), every import path `venturo-skeleton-go/` → `claude-usage-server/`, `Makefile`, `.env.example`, `.air.toml` (binary name)

- [ ] **Step 1: Copy the skeleton without git or build output**

Run (PowerShell): `robocopy C:\Code\venturo-skeleton-go C:\Code\claude-usage-server /E /XD .git bin tmp .claude /XF .env`
Expected: exit code 1 (files copied).

- [ ] **Step 2: Rename the module**

In `go.mod`, set `module claude-usage-server`. Replace `"venturo-skeleton-go/` with `"claude-usage-server/` in every `*.go` file.
Run: `go build ./...` → exit 0.

- [ ] **Step 3: Add `docker-compose.yml`**

Use the compose file from the spec's "Local environment (Docker)" section exactly (Postgres only, `pgdata` volume, healthcheck).

- [ ] **Step 4: Add Makefile targets and switch seeding to Docker**

- Add `up:` (`docker compose up -d --wait`) and `down:` (`docker compose down`), and add both to `.PHONY` and `help`.
- In `seed-core`, replace the host `psql` call with `docker compose exec -T postgres psql -v ON_ERROR_STOP=1 -U $(DB_USER) -d $(DB_NAME) -v admin_email="$(SEED_ADMIN_EMAIL)" -v admin_password="$(SEED_ADMIN_PASSWORD)" < $$file`.
- Change `db-reset` the same way, running against database `postgres`.
- Change `build` to output `bin/claude-usage-server`.

- [ ] **Step 5: Write `.env.example` and copy it to `.env`**

Set `DB_NAME=claude_usage`, `SERVER_HOST=127.0.0.1`, `SERVER_PORT=8080`, `SEED_ADMIN_EMAIL=admin@claude-usage.local`, `SEED_ADMIN_PASSWORD=<SEED_ADMIN_PASSWORD>`, `TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/claude_usage_test?sslmode=disable`. Leave the other variables as they are; Task 2 removes them.

- [ ] **Step 6: Start the DB and confirm the copy runs unchanged**

Run: `make up`, then `make db-setup`.
Expected: the container is healthy and the migrations reach the latest version. The skeleton seeders still exist at this point, so they run too.

- [ ] **Step 7: Initialize git**

Run: `git init`, `git add -A`. Don't commit unless the user asks.

### Task 2: Strip the skeleton down to auth, users, roles and api_keys

**Files:**
- Delete: `internal/modules/core/{approval,branch,client,company,translation_overrides}`, `internal/middleware/{branch,client,company}.go`, `internal/shared/redis`, `pkg/{firebase,storage,email,discord}`, `pkg/logger/discord_core.go`, `docs/api-contract/core/*` for the deleted modules, `deployments/kubernetes`
- Delete and rewrite: `internal/database/migrations/core/*` → `000001_init_schema`, `000002_users`, `000003_rbac`, `000004_api_keys`, `000005_audit_logs` (up and down)
- Delete and rewrite: `internal/database/seeders/core/*` → `001_roles.sql`, `002_admin_user.sql`
- Modify: `internal/router/router.go`, `internal/config/config.go`, `internal/shared/authz/service.go`, `pkg/jwt/claims.go`, `internal/middleware/{context,role,auth}.go`, and the `auth`, `user`, `role` and `api_key` modules
- Test: `internal/shared/authz/service_test.go`

**Interfaces:**
- Produces: `authz.NewService(fetcher PermissionFetcher) *authz.Service` with `GetPermissions(ctx, userID string) ([]string, error)`, `Has`, `HasAny` and `HasAll` (no `companyID` parameter, no cache, no `Invalidate*`).
- Produces: `jwt.Claims{UserID, Email, Username, FullName string; IsSuperAdmin bool; Roles []string; ScopedPermissions []string}`. Remove `CompanyID`, `CompanyName`, `ClientID` and `ClientSlug`.
- Produces: role codes `super_admin` (id `00000000-0000-0000-0000-000000000001`) and `member` (id `00000000-0000-0000-0000-000000000003`, permissions `{"usage":"viewer"}`).
- Produces: auth routes kept under `/core/v1/auth`:
  - `POST signin`
  - `POST refresh`
  - `POST logout`
  - `POST logout-all`
  - `GET me`
  - `POST change-password` (keep, or add if missing)
  - `signup`, `google`, `switch-company`, `my-companies`, email verification and forgot or reset password by email are removed.

- [ ] **Step 1: Delete the listed directories and files**

- [ ] **Step 2: Rewrite the migrations without companies**

The schema is the same as the skeleton's, minus every `company_id` column and index. The `api_keys` and `audit_logs` tables have no company columns. `user_roles` has the primary key `(user_id, role_id)`. Keep `core.has_permission`. Drop migration 000014 (`google_auth_support`) and the `user_identities` table.

- [ ] **Step 3: Write the seeders**

- `001_roles.sql` inserts `super_admin` and `member` with the ids above, using `ON CONFLICT DO NOTHING`.
- `002_admin_user.sql`:
  - `CREATE EXTENSION IF NOT EXISTS pgcrypto;`
  - Insert a user with email `:'admin_email'`, username `admin`, full name `Administrator`, `password_hash = crypt(:'admin_password', gen_salt('bf', 10))`, `is_active` and `is_email_verified` set to TRUE. Use `ON CONFLICT DO NOTHING` on the email index.
  - Then assign `super_admin` to that user in `user_roles`.

pgcrypto bf hashes start with `$2a$` and work with `golang.org/x/crypto/bcrypt`.

- [ ] **Step 4: Remove Redis from authz, and write `TestGetPermissionsReadsFetcherEveryCall`**

```go
func TestGetPermissionsReadsFetcherEveryCall(t *testing.T) {
	f := &fakeFetcher{perms: []string{"usage:read"}}
	s := authz.NewService(f)
	_, _ = s.GetPermissions(context.Background(), "u1")
	got, _ := s.GetPermissions(context.Background(), "u1")
	if f.calls != 2 || !slices.Equal(got, []string{"usage:read"}) {
		t.Fatalf("calls=%d got=%v", f.calls, got)
	}
}
```

`fakeFetcher` implements whatever `PermissionFetcher` method the role repository exposes, minus the company argument. Run `go test ./internal/shared/authz/...` and expect FAIL, then implement and expect PASS.

- [ ] **Step 5: Fix compile errors module by module**

Remove company, branch, client and firebase wiring from `router.go`, `config.go`, `auth`, `user`, `role`, `api_key` and the middleware.

- `GetCompanyID`, `CompanyContext`, `BranchScope` and `SetCompanySyncer`-style setters disappear.
- User service no longer syncs companies or branches.
- `api_key` no longer reads `company_id`.
- `config.go` loses the Redis, SMTP, GCS, Firebase and frontend-URL sections and gains `Server.Host` (env `SERVER_HOST`, default `127.0.0.1`).
- `cmd/api/main.go` listens on `cfg.Server.Host + ":" + cfg.Server.Port`.
- `router.go` CORS origins become `http://localhost:5173` and `http://localhost:3000`. The health message is `"Claude Usage API is running"`.

Run: `go build ./... && go vet ./...` → exit 0.

- [ ] **Step 6: Remove the dropped env variables and dependencies**

Remove the `REDIS_*`, `SMTP_*`, `GCS_*`, `FIREBASE_*`, `FRONTEND_URL`, `EMAIL_VERIFICATION_*`, `RESET_PASSWORD_URL`, `SUPPORT_EMAIL` and `AUTH_DEFAULT_ADMIN_ROLE_ID` lines from `.env.example` and `.env`. Run `go mod tidy`.

- [ ] **Step 7: Verify nothing references the dropped parts**

Run: `grep -rniE "company|branch|approval|translation|client_id|firebase|redis|smtp|gcs|discord" --include=*.go --include=*.sql .`
Expected: no matches, apart from the gin-contrib `cors` import and the word `client` in `ClientIP()`.

- [ ] **Step 8: Reset the DB, sign in as the seeded admin, check the token**

- Run `make db-reset`, then `make run`.
- `POST http://127.0.0.1:8080/core/v1/auth/signin` with `{"email":"admin@claude-usage.local","password":"<SEED_ADMIN_PASSWORD>"}` → 200 with `access_token`.
- `GET /core/v1/auth/me` with that token → `is_super_admin: true`.

### Task 3: Usage tables and repository upserts

**Files:**
- Create: `internal/database/migrations/core/20261030000006_usage.up.sql` and `.down.sql`
- Create: `internal/modules/usage/domain/usage.go`, `internal/modules/usage/repository/usage.repository.go`, `internal/modules/usage/repository/usage.repository_test.go`, `internal/modules/usage/testdb/testdb.go`
- Modify: `Makefile` (target `test-db`)

**Interfaces:**
- Produces, in `domain`:
  - `type Session struct { ID, UserID, Machine, Project, Title, Model string; ContextWindow, StartContext, LatestContext, PeakContext, NewContext int64; Requests, Compactions int32; FirstRequestAt, LastRequestAt, LastReportedAt time.Time }`
  - `type HourUsage struct { Hour time.Time; Model string; NewContext int64; Requests int32 }`
- Produces: `repository.New(db *pgxpool.Pool) *Repository`.
- Produces: `(*Repository) SaveSession(ctx context.Context, s domain.Session, hours []domain.HourUsage) (owned bool, err error)`. One transaction:
  - Upsert `usage.sessions` with `ON CONFLICT (id) DO UPDATE SET ... , last_reported_at = NOW(), updated_at = NOW() WHERE usage.sessions.user_id = EXCLUDED.user_id RETURNING id`. If no row comes back, return `owned=false` and roll back.
  - Otherwise upsert each hour row with `ON CONFLICT (session_id, hour, model) DO UPDATE SET new_context = EXCLUDED.new_context, requests = EXCLUDED.requests`, using `pgx.Batch`.
- Produces: `testdb.Open(t *testing.T) *pgxpool.Pool`. It reads `TEST_DATABASE_URL` and calls `t.Skip` if it's unset. It truncates `usage.session_hours`, `usage.sessions` and `core.api_keys` and deletes users other than the seeded ones. It also provides `testdb.User(t, pool, email string, admin bool) string`, which inserts a user with role `member` or `super_admin` and returns the id.

- [ ] **Step 1: Write the migration**

Use the DDL from the spec's `usage.sessions` and `usage.session_hours` sections verbatim, including `CREATE SCHEMA IF NOT EXISTS usage`. The down migration is `DROP SCHEMA usage CASCADE`.

- [ ] **Step 2: Add the `test-db` Makefile target**

It creates the `claude_usage_test` database through `docker compose exec -T postgres psql`, ignoring the error if it already exists. It runs `migrate` on it with `-database "$(TEST_DATABASE_URL)&x-migrations-table=schema_migrations_core"`, then the seeders.

Run: `make migrate-up && make test-db` → exit 0.

- [ ] **Step 3: Write the failing repository tests**

```go
func TestSaveSessionUpsertsAbsoluteValues(t *testing.T)
```
- Save a session (NewContext 64000) with hours `[{02:00, opus, 40000, 120}, {06:00, sonnet, 8000, 30}]`.
- Save again with NewContext 70000 and the 06:00 row changed to 14000.
- Assert: one `usage.sessions` row with `new_context=70000`; two hour rows; `SUM(new_context)=54000`; the 06:00 sonnet row is 14000.

```go
func TestSaveSessionRefusesOtherOwner(t *testing.T)
```
- Save a session `s1` for user A.
- Save `s1` for user B.
- Assert: `owned=false`, `user_id` is still A, and A's hour rows are unchanged.

Run: `make test-db && go test ./internal/modules/usage/...` → FAIL (undefined `repository.New`).

- [ ] **Step 4: Implement `domain`, `testdb` and `SaveSession`**

- [ ] **Step 5: Run the tests**

Run: `go test ./internal/modules/usage/...` with `TEST_DATABASE_URL` set → PASS.

### Task 4: Ingest endpoint

**Files:**
- Create: `internal/modules/usage/dto/ingest.dto.go`, `internal/modules/usage/service/usage.service.go`, `internal/modules/usage/service/ingest_test.go`, `internal/modules/usage/handler/usage.handler.go`, `internal/modules/usage/main.usage.go`
- Modify: `internal/router/router.go` (mount `/usage/v1`)

**Interfaces:**
- Consumes: `repository.SaveSession` and `domain.*` from Task 3; `middleware.JWTAuth`, `middleware.IsApiKeyAuth` and `middleware.MustGetUserID` from the skeleton.
- Produces, in `dto`:
  - `IngestRequest{ Machine string \`json:"machine" binding:"required,max=255"\`; AppVersion string \`json:"app_version"\`; Sessions []SessionPayload \`json:"sessions" binding:"max=200,dive"\` }`
  - `SessionPayload` with the JSON fields of the spec's ingest example: `session_id`, `project`, `title`, `model`, `context_window`, `start_context`, `latest_context`, `peak_context`, `new_context`, `requests`, `compactions`, `first_at`, `latest_at` and `hours []HourPayload`.
  - `HourPayload{ Hour time.Time \`json:"hour"\`; Model string; NewContext int64; Requests int32 }`
  - `IngestResponse{ Accepted int \`json:"accepted"\`; Rejected []Rejection \`json:"rejected"\` }` and `Rejection{ SessionID, Reason string }`
- Produces: `service.New(repo *repository.Repository) *Service` and `(*Service) Ingest(ctx context.Context, userID string, req dto.IngestRequest) dto.IngestResponse`. `Rejected` is always a non-nil slice.
- Produces: `usage.Initialize(db *pgxpool.Pool) *Module` and `(*Module) SetupRoutes(rg *gin.RouterGroup)`.
- Produces: the route `POST /usage/v1/sessions`, which uses `middleware.JWTAuth()` and returns 403 when `!middleware.IsApiKeyAuth(c)`. The response is `response.Success(c, 200, "Usage saved", IngestResponse)`.

Per-session validation, before saving, in `func validate(s dto.SessionPayload) error`:
- `session_id` is 1–100 characters.
- `model` is 1–100 characters.
- Every number is ≥ 0.
- `len(hours) ≤ 2000`.
- Every hour has minute = second = nanosecond = 0.
- `first_at ≤ latest_at`.

An invalid session goes into `Rejected` with the error text. A session that isn't owned gets the reason `"session belongs to another user"`.

- [ ] **Step 1: Write the failing tests**

`TestValidateRejectsBadSessions` is a table test with no DB. Cases:
- a negative `new_context`;
- an hour of `02:30`;
- 2,001 hours;
- an empty `session_id`;
- `first_at` after `latest_at`.

Each case returns a non-nil error, and a valid session returns nil.

`TestIngestIsIdempotent` (DB): call `Ingest` twice with the same request of 2 sessions. Both calls return `Accepted=2`, and the DB totals are identical after each call.

`TestIngestRejectsForeignSession` (DB):
- Call `Ingest` for user A with `s1`.
- Call `Ingest` for user B with `s1` and `s2`.
- Assert B's call returns `Accepted=1` and `Rejected=[{s1,"session belongs to another user"}]`.

Run: `go test ./internal/modules/usage/...` → FAIL.

- [ ] **Step 2: Implement the dto, service, handler and module, and mount it in `router.go`**

- [ ] **Step 3: Run the tests** → PASS.

- [ ] **Step 4: Smoke-test through HTTP**

- Create a key for the admin through the existing self-service API key endpoint.
- `curl -X POST http://127.0.0.1:8080/usage/v1/sessions -H "X-API-Key: <key>" -H "Content-Type: application/json" -d @testdata/ingest.json`, where `testdata/ingest.json` is the spec's example payload.
- Expected: `"accepted":1`.

### Task 5: Read endpoints for the dashboard

**Files:**
- Create: `internal/modules/usage/dto/report.dto.go`, `internal/modules/usage/repository/report.repository.go`, `internal/modules/usage/service/report.service.go`, `internal/modules/usage/service/report_test.go`
- Modify: `internal/modules/usage/handler/usage.handler.go`, `internal/modules/usage/main.usage.go`

**Interfaces:**
- Produces, in `dto`:
  - `ReportQuery{ From, To time.Time; TZ string; UserID string }`, parsed from `from`, `to` (RFC 3339; default `to` = now, `from` = `to` − 7 days), `tz` (default `Asia/Jakarta`, checked with `time.LoadLocation`, 400 if invalid) and `user_id`.
  - `Caller{ UserID string; IsAdmin bool }`
  - `PersonSummary{ UserID, FullName, Email, CurrentModel string; NewContext, Requests int64; Sessions int64; LastRequestAt, LastSyncedAt *time.Time }`
  - `Overview{ People []PersonSummary; TotalNewContext, TotalRequests, TotalSessions int64; ActivePeople int64 }`
  - `Point{ Bucket time.Time; Key string; NewContext, Requests int64 }`
  - `ModelSummary{ Model string; NewContext, Requests, Sessions int64 }`
  - `SessionQuery{ ReportQuery; Project, Model, Q string; Page, PerPage int; Sort string }`, with `Sort` one of `last_request_at`, `-last_request_at`, `new_context`, `-new_context` (default `-last_request_at`) and `PerPage` ≤ 100 (default 25).
  - `SessionDetail{ Session domain.Session; UserName string; Hours []domain.HourUsage }`
  - `Person{ ID, FullName, Email string; IsActive bool }`
- Produces: `func (s *Service) scope(c dto.Caller, requested string) string`. It returns `requested` for an admin, where empty means everyone, and `c.UserID` otherwise.
- Produces: service methods, each applying `scope` first:
  - `Overview(ctx, c, q) (dto.Overview, error)`
  - `Timeseries(ctx, c, q, interval, groupBy string) ([]dto.Point, error)`, with `interval` in {`hour`, `day`} and `groupBy` in {`user`, `model`}, otherwise 400
  - `Models(ctx, c, q) ([]dto.ModelSummary, error)`
  - `Sessions(ctx, c, sq) ([]dto.SessionRow, int64, error)`
  - `Session(ctx, c, id) (dto.SessionDetail, error)`, which returns 404 when a member asks for someone else's session
  - `People(ctx, c) ([]dto.Person, error)`, which returns 403 for non-admins
- Produces: these routes, all `middleware.JWTAuth()` with 403 when `middleware.IsApiKeyAuth(c)`:
  - `GET /usage/v1/overview`
  - `GET /usage/v1/timeseries`
  - `GET /usage/v1/models`
  - `GET /usage/v1/sessions`, using `response.SuccessWithPagination`
  - `GET /usage/v1/sessions/:id`
  - `GET /usage/v1/people`
  - `Caller.IsAdmin` comes from `claims.IsSuperAdmin`.

How the queries work:
- Time filters apply to `session_hours.hour` (`hour >= from AND hour < to`). Buckets use `date_trunc($interval, hour AT TIME ZONE $tz)`.
- `Sessions` filters `last_request_at` within the range. `Q` matches `title ILIKE` or `project ILIKE`.
- `ActivePeople` counts people with any `new_context > 0` in the range.
- `LastSyncedAt` is `MAX(api_keys.last_used_at)` for that user.

- [ ] **Step 1: Write the failing tests**

`TestScope` (no DB):
- An admin with `""` gets `""`.
- An admin with `"b"` gets `"b"`.
- A member `a` with `"b"` gets `"a"`.

`TestMemberCannotReadOthers` (DB): seed sessions for A and B. As member A, with `UserID="B"`, call `Overview`, `Models`, `Sessions` and `Timeseries`. Each returns only A's data. `Session(ctx, A, "<B's session id>")` returns a not-found error.

`TestTimeseriesUsesTimeZone` (DB):
- Seed an hour row at `2026-10-29T23:00:00Z` (06:00 on Oct 30 in Jakarta) with 5000.
- `interval=day` with `tz=Asia/Jakarta` gives a bucket of `2026-10-30` and 5000.
- With `tz=UTC`, the bucket is `2026-10-29`.
- `tz=Mars/Base` gives an error.

`TestOverviewTotals` (DB): two users with known hour rows give the expected `TotalNewContext`, `TotalSessions` and per-person `NewContext`.

Run → FAIL.

- [ ] **Step 2: Implement the dto, report repository and service, and the handlers and routes**

- [ ] **Step 3: Run the tests** → PASS.

### Task 6: Admin user and API key management

**Files:**
- Modify: the `api_key` module (service, repository, handler, routes), the `user` module (service, handler, routes)
- Test: `internal/modules/core/api_key/service/api_key_admin_test.go`, `internal/modules/core/user/service/user_admin_test.go`

**Interfaces:**
- Produces these routes under `/core/v1/admin`, all `JWTAuth` + `RequireRole("super_admin")`:
  - `POST /admin/users`, body `{email, username, full_name, password}`. It creates the user and assigns the `member` role.
  - `GET /admin/users` lists users with `is_active` and their role codes.
  - `POST /admin/users/:id/deactivate` sets `is_active=false` and revokes all of the user's active keys with reason `"user deactivated"`.
  - `POST /admin/users/:id/activate`
  - `POST /admin/users/:id/reset-password`, body `{password}`.
  - `POST /admin/users/:id/api-keys`, body `{name}`, returns `{key, key_prefix, id}`. The full key is returned only here.
  - `GET /admin/users/:id/api-keys`
  - `POST /admin/api-keys/:id/revoke`
- Produces: `(*ApiKeyService) CreateForUser(ctx, actorID, userID, name string) (fullKey string, key *domain.ApiKey, err error)` and `(*ApiKeyService) RevokeAllForUser(ctx, actorID, userID, reason string) error`.
- Produces: `ValidateAndGetClaims` rejects keys whose owner has `is_active = false`, with the error `"user is inactive"`.
- Reuse the existing user-create and password-hash code. Delete skeleton user routes these duplicate, but keep the self-service `GET/POST/DELETE` API key routes under `/core/v1/api-keys` so members can list their own keys.

- [ ] **Step 1: Write the failing tests**

`TestCreateForUserKeyAuthenticatesAsThatUser`: create a key for member M through `CreateForUser`. `ValidateAndGetClaims` with its key id and secret returns `claims.UserID == M`.

`TestInactiveUserKeyRejected`: deactivate M through the user service. M's keys are now revoked, and `ValidateAndGetClaims` returns an error.

`TestResetPasswordAllowsSignin`: after a reset, `crypto.ComparePassword(newHash, "NewPass123!")` is true.

Run → FAIL.

- [ ] **Step 2: Implement it**

- [ ] **Step 3: Run the tests** → PASS. `go vet ./...` and `gofmt -l .` are clean.

### Task 7: Docs and end-to-end check

**Files:**
- Create: `docs/api-contract/usage/README.md`, `testdata/ingest.json`
- Modify: `README.md`, `CLAUDE.md` (project overview, commands, module list), `docs/api-contract/README.md`

- [ ] **Step 1: Write the usage API contract**

Use the skeleton's `docs/api-contract/core` style. Cover every route from Tasks 4–6, with request and response examples taken from the spec.

- [ ] **Step 2: Rewrite `README.md` and `CLAUDE.md` for this project**

Cover what the project is, `make up`, `make db-setup`, `make run`, the seeded admin, how to issue a key, `make test-db` and `go test ./...`.

- [ ] **Step 3: End-to-end check**

- Run `make db-reset`, then `make run`.
- Sign in as the admin.
- `POST /core/v1/admin/users` to create a member.
- `POST /core/v1/admin/users/:id/api-keys`.
- Ingest `testdata/ingest.json` twice with that key.
- `GET /usage/v1/overview` as the admin shows the member with `new_context` equal to the sum of the payload's hours.
- Sign in as the member: `GET /usage/v1/overview?user_id=<admin id>` returns only the member.

Expected: every step returns the stated result.

- [ ] **Step 4: Full verification**

Run: `gofmt -l . ; go vet ./... ; go test ./...` with `TEST_DATABASE_URL` set → no files listed, everything passes.
