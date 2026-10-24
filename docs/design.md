# readr design

readr tracks daily reading time across bookokrat, Preview and Apple Books without asking the reader to start or stop anything. Each reader already saves where you are, so readr reads their files instead of keeping positions itself.

## Architecture

```
launchd (io.github.shaunwen.readr)
 └─ readr daemon run        always running; schedules everything itself
     ├─ tracking            every tracking.poll_seconds (15) -> sessions in SQLite
     ├─ reminders           every remind.interval from remind.start until nightly.at
     └─ nightly check       at nightly.at -> notification and email if under the daily goal

readr                       TUI and CLI over the same database
~/.local/share/readr/readr.db
~/Library/Logs/readr/daemon.log
```

One binary holds both the daemon and the TUI. macOS grants Full Disk Access, Automation and Accessibility per binary, so one binary means one set of permission prompts.

The workspace has two crates:

- `readr-core`: config and settings, database, session engine, reader adapters, reminder schedule, "up next" suggestion. No terminal or process code.
- `readr`: CLI (clap), daemon loop, notifications, email, launchd, and the ratatui TUI.

Each reader is an adapter behind `ReaderAdapter`. Adapters report activity events (a page turn, a focused tick, the user stepping away) and positions; the session engine in `session.rs` turns events into sessions and never looks at reader files.

## Reader data sources

| | bookokrat | Preview | Apple Books |
|---|---|---|---|
| Data | `~/Library/Application Support/bookokrat/libraries/<hash>_<dir>/bookmarks.json` | `~/Library/Containers/com.apple.Preview/Data/Library/Preferences/com.apple.Preview.ViewState.plist` | `~/Library/Containers/com.apple.iBooksX/Data/Documents/BKLibrary/BKLibrary-*.sqlite`, opened read-only |
| Which book | `absolute_path`; the focused window title (`bookokrat`, U+2014, file stem) via the Accessibility API | AppleScript: path of the front document, only inside `library_dirs` | most recent `ZLASTOPENDATE` in `ZBKLIBRARYASSET` |
| Position | `pdf_page` (0-based; `total_chapters` is the page count for PDFs) or `chapter_href`, plus `book_progress` | entry keyed `<Data volume UUID>.<inode>`; nested `UI_Additions_Windowed_Properties.elementIndex` is the 0-based page | `ZREADINGPROGRESS` |
| Activity | `bookmarks.json` changes on page turns; the window being focused | front app | front app |
| Open | `rio -w <book folder> -e bookokrat <file>`, or a herdr tab | `open -a Preview <file>` | `open ibooks://assetid/<id>` |

Notes:

- bookokrat keeps one library per directory, so a book is opened from its own folder to find its saved place.
- Preview keys entries by inode, so renaming a PDF keeps its position; a re-download gets a new inode and shows up in the Inbox for relinking. Page counts and titles come from Spotlight (`kMDItemNumberOfPages`, `kMDItemTitle`), falling back to a cleaned-up file name.
- Apple Books dates are Core Data timestamps (add 978307200 for Unix time). Its databases use write-ahead logging, so read-only connections stay fresh without locking Books. Collections (`ZBKCOLLECTION`, `ZBKCOLLECTIONMEMBER`) suggest goals.
- Input idle time comes from `ioreg -c IOHIDSystem` (`HIDIdleTime`, nanoseconds).

## Counting time

- While a reader is in front with a known book, each poll adds the time since the previous one, with or without input, so a long page counts in full.
- Without window focus (no Accessibility permission, or bookokrat inside herdr, whose window title is herdr's own), bookokrat time is the gap between page turns, each capped at `max_page_minutes`, plus `last_page_minutes` for the last page.
- After `away_minutes` without input, an `Away` event trims the open session to the last input plus `max_page_minutes`. Leaving a book open over lunch then counts no more than an ordinary page.
- Sessions shorter than `min_session_seconds` are dropped; sessions crossing midnight are split; open sessions are saved on every poll, so `status` and the TUI include the session in progress and a restart loses nothing; edited sessions are never overwritten.
- A saved place is not reading done: you can jump ahead, and anything before readr was untracked. The shelf shows the place ("at p.193/307"), and time left uses only forward progress inside tracked sessions, capped at 1% of the book per minute.

## Storage

SQLite with migrations keyed on `PRAGMA user_version`.

| Table | Holds |
|---|---|
| `books` | the shelf: title, goal, status (active, queued, done, dropped), `priority` (order within its status) |
| `book_sources` | one row per copy in a reader (`reader`, `locator` = path or Books asset id), last position, collections, `untracked` |
| `sessions` | start, end, active seconds, start and end position, the goal at reading time, `edited` |
| `notifications` | reminders and emails already sent per day |
| `untracked_titles` | normalised titles of forgotten books, so new copies are skipped too |
| `settings` | one row per setting, in the text form the settings popup shows |
| `goals` | goals in order, with weekly target, colour, folders and Books collections |

A book can have several sources (the same PDF in bookokrat and Preview); their time adds up. Sessions snapshot their goal, so moving a book to another goal leaves past time where it was unless asked otherwise.

Settings live in the database rather than a file, so the TUI can edit them and the daemon picks changes up on its next poll. `settings.rs` lists every setting once (key, label, help, kind); the popup, `readr config set`, the database and the TOML export all go through it. `~/.config/readr/config.toml`, or the built-in `default_config.toml`, seeds the database on first run only.

## Up next

`readr go` and the Today screen pick the goal furthest behind this week (minutes read since Monday divided by `weekly_minutes`, or by an even share of `daily_goal_minutes × 7` for goals without a target; ties go to the goal listed first), then within it the most recently read active book, then the first in shelf order.

## Reminders

- Every `remind.interval` from `remind.start` until `nightly.at`, while today is under the goal; one "done for today" notice once it is met.
- At `nightly.at`, if under the goal: a notification and an email (lettre, SMTP with STARTTLS; password in the Keychain item `readr-smtp` through the `security` CLI). A check missed while asleep runs on wake, the same day only. The `notifications` table prevents duplicates.
- Delivery: `notify.channel = "system"` uses `terminal-notifier` with a fresh group per reminder (macOS stays silent when a notification identifier is reused) and only `-sound default` (named sounds outside app bundles are not found), falling back to `osascript`. Older readr reminders are cleared once the newest is a minute old, because removing one right before posting delays the new banner. `"herdr"` tries `herdr notification show` first and checks its reply for `"shown":true`, since a rate-limited notification still exits 0.

## Choices

- **ratatui + crossterm**, with a small hand-rolled update/view loop and plain string buffers for text input.
- **jiff** for dates and time zones: it reads the system time zone database, while `time` cannot read the local offset reliably in a multi-threaded process.
- **Stats heatmap drawn with spans** rather than ratatui's calendar widget, which needs the `time` crate.
- **Code signing**: macOS keys Accessibility and Full Disk Access to the code signature. Ad-hoc builds change it on every build, so `make install` signs with a stable certificate when one exists.
- **Forgetting a book** marks every copy untracked and records its title, keeping the rows so readr knows what to skip; sessions are deleted unless `--keep-history`.

## Not built

- Reading done on iPhone or iPad (iCloud-synced Books progress with no Mac activity).
- PDF page labels (printed page numbers).
