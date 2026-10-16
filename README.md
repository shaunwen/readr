# readr

Track at least 20 minutes of reading a day across bookokrat, Preview and Apple Books. Remind in the terminal during the day, email at night if the goal was missed, and keep every reading session in SQLite.

You keep reading in your usual readers. A background daemon notices page turns and which reader is in front, records sessions, and reads each reader's own saved position, so readr never asks where you stopped.

## Install

```bash
make install            # lint + tests + release build, copies to ~/workspace/bin/readr
readr init              # writes ~/.config/readr/config.toml
readr daemon install    # launchd agent com.shaunwen.readr, starts at login
readr doctor            # checks config, readers, permissions, daemon, email
```

Grant `~/workspace/bin/readr` two permissions in System Settings > Privacy & Security:

- **Full Disk Access**, to read the Apple Books library.
- **Accessibility**, to see which window is focused, so a page you read for longer than `max_page_minutes` still counts. The daemon asks for this on start.

macOS also asks once to let readr control Preview, to read which document is open.

macOS ties these grants to the binary's signature. An ad-hoc build changes it on every `make install`, so the grants would have to be given again. To keep them, create a code signing certificate once (Keychain Access > Certificate Assistant > Create a Certificate, Identity Type "Self Signed Root", Certificate Type "Code Signing", name `readr-dev`) and install with `make install CODESIGN_IDENTITY=readr-dev`. `make install` also restarts a running daemon.

## Daily use

```bash
readr                         # TUI: Today, Shelf, Stats, Inbox
readr go [book]               # open the suggested book, or the one named, at its saved place
readr status                  # Today 12/20 min · streak 6, sessions, up next
readr status --short          # 12/20m, for a zsh RPROMPT
readr shelf                   # active books by goal, queue, finished count
readr shelf add <title|path> [--goal health] [--queue]
readr shelf finish|drop|activate|queue <book>
readr shelf set-goal <book> <goal|none> [--move-history]
readr log 25m "Atomic Habits" # reading away from the Mac
readr inbox                   # reading in books not on the shelf
readr inbox link <source> <book> | readr inbox ignore <source>
readr stats                   # last 7 days, streaks, minutes per goal this week
readr goal rename <old> <new>
```

TUI keys: `Tab`/`1-4` screens, `j`/`k` move, `Enter` resume, `a` add, `f` finish, `d` drop, `u` queue or activate, `g`/`G` set goal (G also moves past sessions), `e`/`x` edit or delete a finished session, `n`/`i` link or ignore an inbox item, `?` help, `q` quit.

## Reminders

- Every `remind.interval` (default 2h) from `remind.start` (09:00) until `nightly.at`, while today is under the goal: a herdr notification such as `readr: 8 min to go today / Walden p.9/300 (bookokrat) · readr go`. If herdr is not reachable, a macOS notification.
- Once the goal is reached: one "done for today" notice, then nothing more that day.
- At `nightly.at` (default 21:00), if today is under the goal: a notification and an email. A check missed while the Mac slept runs on wake, the same day only. Each reminder is sent at most once.

Email is sent over SMTP with STARTTLS. Set `nightly.email_to` and `nightly.smtp_user` in the config, then store the password (for Gmail, an app password) in the Keychain:

```bash
readr email set-password      # Keychain item "readr-smtp"
readr email test
readr notify-test
```

## Config

`~/.config/readr/config.toml`, reloaded by the daemon when it changes. An invalid file is reported in the log and the last good config stays in use. See `crates/readr-core/src/default_config.toml` for every key:

- `daily_goal_minutes`
- `count_off_shelf`: whether reading outside the shelf counts toward the goal
- `library_dirs`
- `[remind]` interval and start
- `[nightly]` time and email
- `[tracking]` page gaps, `away_minutes`, `window_focus`
- `[launch] terminal = "rio" | "herdr"`
- `[[goal]]` entries with `key`, `label`, `color`, `weekly_minutes`, `folders`, `books_collections`

Goals are suggested for new books from their folder under `library_dirs`, or from their Apple Books collection.

## How tracking works

| Reader | Which book | Time | Position |
|---|---|---|---|
| bookokrat | `bookmarks.json` in each library under `~/Library/Application Support/bookokrat/libraries`; the focused window title, which bookokrat sets to its name, a dash and the file name | while its window is focused; otherwise time between page turns, each gap capped at `max_page_minutes`, plus `last_page_minutes` | page or chapter and `book_progress` |
| Preview | front document (AppleScript), only inside `library_dirs` | while it is the front app | `com.apple.Preview.ViewState.plist`, keyed by inode, so renames keep the page |
| Apple Books | most recently opened book in `BKLibrary-*.sqlite` (read-only) | while it is the front app | `ZREADINGPROGRESS` |

While a reader is in front, time counts even without keyboard or mouse input, so a long page counts in full. After `away_minutes` (default 15) without input, readr assumes you stepped away: the session is cut back to your last input plus `max_page_minutes`. Window focus works in any terminal whose window title follows the program's title (Rio does by default), but not inside herdr, whose window title is herdr's own. Turn it off with `tracking.window_focus = false`.

Sessions shorter than `min_session_seconds` are dropped. Sessions crossing midnight are split. Open sessions are saved on every poll, so `status` and the TUI include the session in progress, and a daemon restart loses nothing. Sessions you edit are never overwritten.

Data: `~/.local/share/readr/readr.db`. Daemon log: `~/Library/Logs/readr/daemon.log`.

## Development

```bash
make lint      # cargo fmt --check, clippy -D warnings
make test      # unit and CLI tests
cargo run -- daemon run   # tracker in the foreground
```

`readr-core` holds the config, database, session engine, reader adapters, schedule and suggestion logic. `readr` holds the CLI, daemon loop, notifications, email, launchd and the ratatui TUI. See `docs/plan.md` for the design.
