# Claude Usage Monitor

A small desktop app that tracks **context usage of every Claude Code session** on your machine.

On launch it indexes all Claude Code sessions stored locally, then keeps watching them live. For each session it shows how full the context window is and how much new context has been added since the app started. It lives in the system tray and writes a snapshot to a text file every 30 minutes.

## Features

- Indexes every Claude Code session on the machine at startup (`~/.claude/projects`)
- Live updates: new requests show up within ~2 seconds
- Active sessions are highlighted as large cards; idle ones are listed as small grey rows
- Per session: name, model, start / latest / new context, compactions
- Aurora Glass look in Claude colors: dark by default, switch to light with the ☀ / 🌙 button (remembered in `config.json`)
- System tray icon: closing the window hides it to the tray instead of quitting
- Mini mode: a small always-on-top widget with the context bars of up to two active sessions
- Tray tooltip shows today's new context, active sessions and the last sync
- Context warnings: an active card turns amber at 80% and red at 90% of its window, with a one-time desktop notification at 90%
- Idle list sorts by most recent or largest context; sessions idle for more than 7 days fold into "Older than 7 days"
- Keyboard: `Ctrl+F` focuses search; `Esc` closes the details or settings window, clears the search, or leaves mini mode
- The whole UI scales up with the window size, up to 1.5×; `Ctrl+=` / `Ctrl+-` zoom on top of that and `Ctrl+0` resets (not remembered across restarts); zooming in stops where the layout would get narrower than the minimum window width
- Narrow windows drop the less important parts: idle rows hide the model, then the context column, and the bottom bar hides the session counts, then its label; on wide windows the content stays a centered column
- Runs as a single instance: launching it again while it runs does nothing
- Optional **Start with Windows** (in ⚙ Settings); started that way it stays hidden in the tray
- Exports a tab-separated snapshot to `usage.txt` every 30 minutes (only when something changed)
- Optionally syncs usage to a team server every 30 minutes, on demand with **Sync now**, and once more on **Quit**
- Shows your Claude plan's 5-hour and 7-day limits, plus any weekly limit Claude sets for a single model (such as Fable) as its own meter. Hover a weekly meter for the daily budget left and the projected use at reset. A Windows notification appears when a limit passes 80% and again at 90% (once per window; limits already past a step when the app starts stay quiet). Every 5 minutes it asks Anthropic's usage endpoint (the one `/usage` shows) with Claude Code's own sign-in from `~/.claude/.credentials.json`, so it works when you only use the VS Code panel. The token is only sent to Anthropic, never refreshed (an expired one is skipped until Claude Code renews it) and never sent to the team server. Running `claude` in a terminal also reports the limits through the status line
- Windows installer (Inno Setup) built by CI
- Runs on Windows, macOS and Linux

> Only **Claude Code** sessions (CLI and IDE extensions) are tracked. claude.ai web/desktop/mobile chats are stored on Anthropic's servers, not on your disk, so they can't be read.

---

## Building

### 1. Install the prerequisites

**All platforms:** install Rust from [rustup.rs](https://rustup.rs) (stable toolchain, Rust 1.89 or newer).

| OS | Also needed |
|---|---|
| Windows | [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload (its Windows SDK provides `rc.exe`, which `build.rs` uses to embed the app icon drawn in `src/icon.rs`) |
| macOS | Xcode Command Line Tools: `xcode-select --install` |
| Linux (Debian/Ubuntu) | `sudo apt install libgtk-3-dev libxdo-dev libayatana-appindicator3-dev` |

### 2. Build

```sh
git clone https://github.com/dimasekaadinandra25/claude-context-tracking.git
cd claude-context-tracking
cargo build --release
```

The first build takes a few minutes; later builds take seconds.

### 3. Find the app

| OS | Output |
|---|---|
| Windows | `target\release\claude-usage-monitor.exe` |
| macOS / Linux | `target/release/claude-usage-monitor` |

The binary is self-contained. Copy it anywhere and run it; Rust is only needed to build it, not to run it.

> **Windows:** close the running app (tray icon → **Quit**) before rebuilding, otherwise the build can't overwrite the `.exe`.

### Prebuilt binaries (GitHub Actions)

Every push runs [`.github/workflows/build.yml`](.github/workflows/build.yml), which tests and builds the app on Windows, macOS and Linux. Download the binaries from the run's **Artifacts** section on the repository's **Actions** tab. You can also start a build manually there with **Run workflow**.

To publish a release, bump `version` in `Cargo.toml`, commit, then push a matching tag (`git tag v0.3.0 && git push origin v0.3.0`). The workflow then creates a GitHub Release with the Windows installer and the Windows, macOS and Linux binaries attached. The release job fails if the tag and `Cargo.toml` disagree. When a teammate already runs a newer version, the app's bottom bar shows `· vX available`; clicking it installs the latest release.

### Windows installer

The Windows job also builds `claude-usage-monitor-setup-<version>.exe` (artifact **claude-usage-monitor-setup**) from [`installer/claude-usage-monitor.iss`](installer/claude-usage-monitor.iss) with [Inno Setup 6](https://jrsoftware.org/isinfo.php), using the version from `Cargo.toml`. It installs the app to `Program Files\Claude Usage Monitor`, adds a Start-menu shortcut (and a desktop shortcut if you tick it), registers an uninstaller, and offers **Launch now** at the end. Running it again upgrades in place; it closes a running copy first. Uninstalling also removes the **Start with Windows** entry.

To build it locally after `cargo build --release`:

```sh
iscc /DAppVersion=0.3.0 /DAppIcon=%CD%\target\release\app.ico installer\claude-usage-monitor.iss
```

The setup file lands in `target\installer\`.

### Other commands

```sh
cargo test               # run the tests
cargo build              # debug build (shows a console window on Windows)
cargo run -- --dump      # print the session table as text instead of opening the window
```

---

## Using the app

### Starting

Run the binary. A progress bar shows while it indexes your existing sessions; then the session list appears with a green **● Live** label, and the app keeps updating on its own.

With **Start with Windows** ticked, the app starts at login with `--hidden`: the window stays hidden and only the tray icon appears (click it to show the window). Launching it yourself opens the window as usual. Start-with-Windows entries from older versions, which lack the flag, are upgraded on the next start.

On the first launch without an API key, a Windows notification says "Add your API key to send usage to the team dashboard". It shows only once (`key_reminder_shown` in `config.json`).

### Reading the window

A session is **active** when its latest request, including requests of its subagents, was less than 10 minutes ago. Active sessions are shown first as glass cards with an orange-to-violet context bar; idle sessions follow in one panel with their model, current context (size and share of the window), new context (`—` when nothing was added) and how long ago they were last used. Your machine name is shown next to the app title.

| Field | Meaning |
|---|---|
| **Session** | The session's title (auto-generated by Claude Code), or its project folder if it has none. Hover to see the full project path and session id. |
| **Model** | Model of the session. Updates within seconds of `/model` |
| **Start context** | Context size when tracking started: at app launch for existing sessions, or the first request of a session started later |
| **Latest context** | The context bar: size of the latest request, as `used / window (percent)` |
| **New context** | Total context the session has added while the app was running. It only goes up: a compaction adds nothing, and growth after it adds on top. |
| **Compacted** | How many times the session was compacted (manual `/compact` or auto-compact) since tracking started. Shown on active cards and in the context details |

"Context" is the full prompt sent on a request: input + cache write + cache read tokens. Subagent requests are ignored here, since they run in their own context window; only the server sync counts them (see below). Context windows are 1M tokens for Claude 4.6 and newer models, 200K for older ones. An older model counts as 1M when it was picked with a 1M variant in `/model` (for example `claude-sonnet-4-5[1m]`), or as soon as one of its requests is larger than 200K.

**Example:** the app starts while a session is at 50K.

| Time | What happens | Latest context | New context | Compacted |
|---|---|---|---|---|
| 09:00 | App starts | 50K / 1M (5%) | +0 | — |
| 09:10 | Work continues | 120K / 1M (12%) | +70K | — |
| 09:25 | Work continues | 210K / 1M (21%) | +160K | — |
| 09:40 | `/compact` | 35K / 1M (4%) | +160K | 1× |
| 09:55 | Work continues | 90K / 1M (9%) | +215K | 1× |
| 10:15 | Auto-compact | 30K / 1M (3%) | +215K | 2× |
| 10:25 | Work continues | 70K / 1M (7%) | +255K | 2× |

A compaction shows up within seconds of `/compact` or an auto-compact finishing. Until the session's next request, **Latest context** is an estimate marked `≈`: the compaction summary plus the session's first request (system prompt, tools, memory). The next request replaces it with the measured size without adding to **New context**. A context drop between two requests without a compaction marker also counts as a compaction.

Switching models with `/model` works the same way: the new model and its context window show up within seconds, and **Latest context** is marked `≈` until the next request. Since models can count tokens differently, the first request on a new model sets a new baseline instead of adding to **New context** or counting as a compaction.

Restarting the app resets the start context of every session to its current value and its new context and compaction count to zero.

### Context details

Hover over a session's context bar (marked ⓘ) to see a breakdown; click it to open the same breakdown in a window that stays open and updates live.

| Section | Shows |
|---|---|
| Category | **System prompt & tools**, **Custom agents**, **Memory files**, **Skills**, **Messages** and **Free space**, with tokens and share of the context window |
| Memory files | Each loaded memory file (`CLAUDE.md` files, `CLAUDE.local.md`, the project's auto-memory `MEMORY.md`) with its estimated tokens |
| Custom agents | Each custom agent from your agent folders and enabled plugins with its estimated tokens |
| Latest request | Cache read, cache write, uncached input and output tokens of the most recent request |
| New context by model | How much of **New context** each model added, when the session grew (one row per model) |
| Session | Start, new and peak context, compactions, request count, first and last request time (local time), project path and session id |

How the categories are worked out:

- **Messages** is measured: the latest context minus the session's first request.
- **Memory files**, **Custom agents** and **Skills** (marked `≈`) are estimated from the files Claude Code loads: your `~/.claude` folder, the project's `.claude` folder, and the plugins enabled in `~/.claude/settings.json`. Estimates are refreshed every 30 seconds.
- **System prompt & tools** is the session's first request minus those estimates. It contains the system prompt, built-in and MCP tools, MCP server instructions and the first message.

Claude Code's `/context` measures each part directly while the session runs, but session files only record token totals per request, so these categories are close but not identical. MCP tools and the autocompact buffer can't be read from disk and stay inside **System prompt & tools**.

### New context since launch

The bar at the bottom sums **New context** across all sessions in the list (respecting the search filter) and shows how many of them are active. It counts from when the app started; the tray tooltip and mini mode instead show today's new context across all sessions, counted from local midnight.

### Search

Sessions are listed most recently used first (subagent requests count as use). Use the search box to filter by session name, model or project path. When nothing matches, the list says so; when no sessions exist at all, it shows the folder it reads them from.

### System tray

| Action | Result |
|---|---|
| Close button (X) | Hides the window to the tray; tracking and exporting continue |
| Minimize | Normal minimize to the taskbar/dock |
| Left-click tray icon | Shows the window again (Windows, macOS) |
| Right-click tray icon → **Show** | Shows the window |
| Right-click tray icon → **Mini mode** | Shows the window as a small always-on-top widget, or switches back (see below) |
| Right-click tray icon → **Sync now** | Sends usage to the server right away (see the sync section) |
| Right-click tray icon → **Open data file** | Opens `usage.txt` in your default text editor |
| Right-click tray icon → **Quit** | Sends one final sync (at most ~5 seconds), then exits the app completely |

The tray icon stays visible as long as the app is running.

### Mini mode

Click **🗗** in the top bar (or tray → **Mini mode**) to shrink the window to a small widget that stays on top of other windows. It shows one line per active session (up to two): the live dot, the session name and its context bar with the percentage. With no active session it shows `No active session` and today's new context. Double-click anywhere in the widget, press `Esc`, or use the tray item again to return to the full window at its previous size. The theme and backdrop are the same as in the full window.

### Data file

The app writes a tab-separated snapshot right after startup, then checks every **30 minutes** and rewrites the file only if the data changed. Each write replaces the whole file, so it always holds the latest state.

| OS | Location |
|---|---|
| Windows | `%LOCALAPPDATA%\ClaudeUsageMonitor\usage.txt` |
| macOS | `~/Library/Application Support/ClaudeUsageMonitor/usage.txt` |
| Linux | `~/.local/share/ClaudeUsageMonitor/usage.txt` |

Format:

```
machine	session	model	start_context	latest_context	new_context	compactions
MY-LAPTOP	RAG system with pgvector in Go	claude-opus-5-5	50.0K	90.0K / 1M (9%)	+215.0K	1
```

Text editors such as Notepad don't reload a file that changes while it is open; reopen it to see a new snapshot.

If the app ever crashes, details are appended to `crash.log` in the same folder. `app.log` there records every start and every **Quit**, and the app marks itself alive every 5 minutes. When a run ends any other way, the next start writes when it was last alive and the likely cause: a crash, Windows shutting down or restarting, or the process being ended without **Quit** (Task Manager, an installer, signing out).

### Sync to a usage server

The app can send each session's usage to a `claude-usage-server` so a team dashboard shows everyone's usage over time.

1. Ask your admin for an API key.
   Until a key is set, an amber banner at the top of the window says your usage isn't sent to the team dashboard yet; its **Add API key** button opens Settings. When the server refuses the key (HTTP 401 or 403), a red banner says "API key rejected. Check it in Settings." with an **Open Settings** button, and the bottom bar reads `Key rejected` in red. While sync is off or the key is rejected, clicking the sync status in the bottom bar also opens Settings.
2. Click the **⚙** button in the top bar to open **Settings**, paste the **API key** and click **Save**. The server URL is built in: `http://localhost:8080` by default, or the value of `CLAUDE_USAGE_SERVER_URL` at build time (`CLAUDE_USAGE_SERVER_URL=https://usage.example.com cargo build --release`). Setting `CLAUDE_USAGE_SERVER_URL` when the app runs, or `server_url` in `config.json`, overrides it.

The app sends about a minute after startup (until then the status reads `Waiting to sync`), right after you change the API key, server URL or privacy setting, and then every **30 minutes**. Click **⟳** next to the sync status in the bottom bar, or tray → **Sync now**, to send right away. Sending runs in the background and the status reads `Syncing…` meanwhile; the session list keeps updating live. Tray → **Quit** saves the session cache, then sends once more before exiting so the dashboard is current; if the server doesn't answer within about 5 seconds the app exits anyway and the data goes out on the next start. Closing the window only hides it, so it doesn't sync. The first send backfills every past session; later sends only carry sessions with new requests (including subagent requests). Large backfills go out in batches and the sync position advances after each accepted batch, so a failure part way only resends what wasn't accepted yet. Each send carries lifetime totals per session, bucketed by UTC hour and model, so resends and restarts never double count: context growth, requests and output tokens, plus the request count and output tokens of the session's subagents (read from `isSidechain` lines, including the `<session-id>/subagents/agent-*.jsonl` transcripts). Each hour row also carries the billed input tokens: uncached input, cache write and cache read, summed over every request of the main thread and its subagents (subagents run their own billed requests). Each session also carries its context overhead as `overhead`: system prompt & tools, memory files, custom agents and skills, the same estimates as in **Context details** (estimated once per project per send). It is left out for a session without a first request, and sent even with titles hidden since it's only numbers. Every send includes the app version in the `X-App-Version` header; the version is also shown at the bottom of **⚙ Settings**. The bottom bar shows `Sync off`, `Synced 12:30` (local time) or `Sync failed: <reason>` (hover for the full reason); a failed send is retried on the next 30-minute tick. When the server rejects some sessions (for example a session that belongs to another user) it reads `Synced 12:30 · 2 rejected`; hover for the reasons. A rejection does not hold the sync back: the next send moves on past it.

Every new plan limit reading (the app reads one every 5 minutes) is sent within 5 minutes instead of waiting for the 30-minute tick, even when no session changed, so the dashboard's daily limit chart and costs stay current. When a teammate already runs a newer app version, the bottom bar adds `· vX available` in amber. Clicking it downloads the installer from the latest GitHub release and runs it silently; it closes this app, upgrades it and starts it again. If the download fails (or on macOS and Linux) it opens the release page instead.

**Privacy:** tick **Hide session titles and project names** in Settings to send only models and token counts; the dashboard then lists those sessions as "Private session". Changing the setting re-sends every session so titles already on the server are cleared (or restored).

The sync position in `sync_state.json` remembers which server URL, API key (stored only as a hash) and privacy setting it belongs to. Changing any of them starts over with a full backfill, so a new account or server gets every session. Sessions whose transcripts Claude Code has already deleted (see **Session history** below) can't be re-sent.

The app keeps a cache of the parsed sessions in `index_cache.json` next to the settings, so later launches only read what changed in the transcripts instead of re-reading all of them.

Settings live in `config.json` and the sync position in `sync_state.json`:

| OS | Location |
|---|---|
| Windows | `%APPDATA%\claude-usage-monitor\` |
| macOS | `~/Library/Application Support/claude-usage-monitor/` |
| Linux | `~/.config/claude-usage-monitor/` |

The environment variables `CLAUDE_USAGE_SERVER_URL` and `CLAUDE_USAGE_API_KEY` override `config.json`, and `CLAUDE_USAGE_CONFIG_DIR` moves both files to another folder. When `CLAUDE_USAGE_API_KEY` is set, Settings shows the key field as **Set by environment variable** and never writes that key to `config.json`. If saving the settings fails, the Settings window shows the error.


### Configuration

- **`CLAUDE_CONFIG_DIR`**: if you've moved Claude Code's config directory, set this environment variable and the app reads sessions from `$CLAUDE_CONFIG_DIR/projects` instead of `~/.claude/projects`.
- **Session history:** by default Claude Code deletes session transcripts older than 30 days. Raise `cleanupPeriodDays` in `~/.claude/settings.json` to keep more history visible to the app.

### Platform notes

- **Linux:** the tray library can't report icon clicks, so use the tray menu's **Show**. On Wayland, windows can't be hidden programmatically, so the close button leaves the window open; use **Quit** in the tray menu to exit.
- **macOS:** the app also appears in the Dock while running.
