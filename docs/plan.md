# readr plan

Planned on 2026-10-16.

## Goal
Read a small, curated set of books at least 20 minutes every day. Each book serves one goal (health, learning, communication, circle of competence). Track time automatically in every reader I use, keep my place in each book, remind me in the terminal during the day, and email me at night if I missed the goal. Store every reading session in SQLite.

## Readers and verified data sources
I read in bookokrat (in native Rio windows, not herdr, because image rendering in herdr has issues), Preview and Apple Books. All three already save the reading position, so readr reads their files and never stores positions itself.

| | bookokrat | Preview | Apple Books |
|---|---|---|---|
| Data | `~/Library/Application Support/bookokrat/libraries/<hash>_<dir>/bookmarks.json` | `~/Library/Containers/com.apple.Preview/Data/Library/Preferences/com.apple.Preview.ViewState.plist` | `~/Library/Containers/com.apple.iBooksX/Data/Documents/BKLibrary/BKLibrary-*.sqlite` + `AEAnnotation/AEAnnotation_*.sqlite` |
| Which book | `absolute_path` | AppleScript: path of front document | `ZBKLIBRARYASSET` (title, author, asset id), `ZLASTOPENDATE` |
| Position | `pdf_page` / `chapter_href` + `book_progress`, `last_read` | entry key `<Data volume UUID>.<inode>`; nested bplist `UI_Additions_Windowed_Properties.elementIndex` = 0-based physical page (confirmed) | `ZREADINGPROGRESS`; `ZAEANNOTATION` row with `ZANNOTATIONTYPE = 3` = last location (EPUB CFI) |
| Activity signal | `bookmarks.json` changes on page turns | front app + not idle | front app + not idle (type-3 row may update per page turn, unverified) |
| Resume | `rio -w "<book folder>" --title-placeholder "readr: <title>" -e bookokrat "<file>"` | `open -a Preview "<file>"` | `open "ibooks://assetid/<id>"` (unverified) |

Notes:
- Apple Books dates are Core Data timestamps: add `978307200` for Unix time. Open both Books databases read-only (`?mode=ro`); they use write-ahead logging, so reads stay fresh without locking Books.
- Preview progress = `(elementIndex + 1) / pages`, pages from `mdls -raw -name kMDItemNumberOfPages <file>`. Keys use inodes, so renames keep history; a re-download gets a new inode and goes to the Inbox for relinking.
- Bookokrat keeps one library per directory, so it must be launched from the book's folder (`-w`) to find its saved position.
- Idle time: `ioreg -c IOHIDSystem`, `HIDIdleTime` in nanoseconds.

## Architecture
```
launchd
 └─ readr daemon   (always running; schedules everything itself so times stay configurable)
     ├─ tracking         every 15s -> SQLite sessions
     ├─ reminders        every [remind.interval] from [remind.start] until [nightly.at]
     └─ nightly check    at [nightly.at] (default 21:00) -> email if under the daily goal

readr              (ratatui TUI + CLI)
~/.local/share/readr/readr.db
~/.config/readr/config.toml   (imported into the database on first run)
```
Cargo workspace:
- `readr-core`: database, session rules, config and goals; shared so the daemon and TUI never disagree on "is today done".
- `readr-daemon`
- `readr-tui`

Each reader is an adapter behind one trait; the session engine only sees events:
```rust
trait ReaderAdapter {
    fn poll(&mut self, now: Timestamp) -> Vec<ActivityEvent>; // PageTurn | FocusedTick, with book locator
    fn position(&self, locator: &Locator) -> Option<Position>;
    fn open(&self, locator: &Locator) -> Result<()>;
}
```

## Tracking rules
- **Page-turn layer** (default, no permissions): time between page turns, each gap capped at `max_page_minutes` (5), plus `last_page_minutes` (1) for the final page. Almost no false positives, since you can't turn pages in a reader you aren't looking at. A dense page longer than the cap is undercounted.
- **Focused-window layer** (optional, needs Accessibility): Rio frontmost, front window/tab is bookokrat, and not idle for `idle_seconds` (120) means each 15s check counts. Falls back to page turns when the permission is missing.
- **Preview and Books**: front app + book identified + not idle.
- A session ends after the gap passes the cap or the reader exits. Sessions under 1 min are dropped. Sessions crossing midnight are split.
- iCloud: reading on iPhone/iPad changes the Books databases with no matching Mac activity. Record it as "read elsewhere" (progress known, time unknown), shown separately from the daily goal unless I decide otherwise.

## Data model
```sql
books(id, title, author, goal, status,      -- active | queued | done | dropped
      priority, added_at, finished_at)
book_sources(id, book_id, reader,           -- bookokrat | preview | apple_books
             locator UNIQUE,                -- file path, or Books asset id
             last_progress, last_position, last_seen_at)
sessions(id, book_id, source_id, goal,      -- goal snapshot at reading time
         started_at, ended_at, active_secs,
         start_progress, end_progress, start_position, end_position,
         note, edited)                      -- edited sessions are never overwritten
notifications(day, kind, sent_at)           -- dedupe reminders and emails
```
One book can have several sources (the same PDF in bookokrat and Preview, a copy in Books); time adds up across them, and "resume" uses the most recently used reader.

## Config
```toml
daily_goal_minutes = 20
count_off_shelf = false        # off-shelf time is logged to the Inbox, not counted

[remind]
interval = "2h"
start = "09:00"                # reminders stop at nightly.at

[nightly]
at = "21:00"
email_to = "..."
smtp_user = "..."              # password in macOS Keychain (keyring crate)

[tracking]
max_page_minutes = 5
last_page_minutes = 1
idle_seconds = 120
use_window_focus = "auto"      # auto | on | off

[launch]
terminal = "rio"               # or "herdr" for text-only EPUBs

[[goal]]
key = "health"                 # stable id stored in the database
label = "Health"
color = "green"
weekly_minutes = 40            # optional
folders = ["health"]
books_collections = ["Health"] # Apple Books collections (only built-in ones exist today)

[[goal]]
key = "communication"
label = "Communication"
folders = ["communication"]

[[goal]]
key = "learning"
label = "Learning"
folders = ["learning"]

[[goal]]
key = "competence"
label = "Circle of competence"
folders = ["work"]
```
Invalid config: the daemon keeps the last good settings and shows the error as a herdr notification.

## Goals (categories)
- Defined only in config; the four above are the `readr init` defaults.
- When a book is added, readr suggests a goal from `folders` / `books_collections`; I confirm with `Enter` or change it with `g`. Rules never reassign silently.
- One goal per book, so minutes per goal add up to the total.

| Change | Effect |
|---|---|
| `label` / `color` | display only |
| `key` | `readr goal rename old new` migrates the database; editing the file alone is flagged by `readr doctor` |
| remove a goal | its books become unassigned and show in the Inbox; sessions kept |
| move a book | asks whether past sessions move too (sessions snapshot their goal) |

## Reminders
- Every 2h while today is under the goal: a notification such as "8 min to go" / "Walden p.4 · readr go", sent through `notify.channel` (see Implementation decisions). One "done for today" message, then silence.
- Nightly at `nightly.at`: email with today's minutes, streak at risk, and the book and page to resume. Sent with `lettre` over SMTP. A check missed during sleep runs on wake, same day only; `notifications` prevents duplicates.

## How I interact with it
Mostly passive: read in any reader and the daemon records it.

```
readr init                # config, launchd agent, permissions, Keychain password
readr doctor              # permissions, tracking layer in use, email test
readr                     # TUI
readr go [book]           # resume suggested (or named, fuzzy) book in its reader
readr status [--short]    # "12/20m" for zsh RPROMPT (Rio has no status bar)
readr log 25m "<book>"    # paper books / away from the Mac
readr stats --week
readr goal rename <old> <new>
readr config [set <key> <value>]
```
`readr go` picks the goal furthest behind its `weekly_minutes` (or least read this week), then the most recently read book in it.

TUI screens:
- **Today**: 0-20 min gauge, streak, today's sessions, "up next".
- **Shelf**: active books (WIP limit 3-4) grouped by goal with where you are (the saved place of the most recently used copy), last read, estimated time left; queue below. Finishing a book promotes the next.
- **Stats**: 12-week calendar heatmap, minutes per goal per week vs target, best streak, pace.
- **Inbox**: off-shelf sessions and unlinked files; add to shelf (past sessions then count), link, or ignore.

```
┌ readr ─ Thu 16 Oct ───────────────────────── streak 6 ┐
│ Today  ████████████░░░░░░░░  12 / 20 min              │
│  08:00  Moby Dick     Books       7m  0.4→1.1%  │
│  12:16  Walden         bookokrat   5m  p.4→p.9   │
│ Up next  ▸ Meditations  (health · least read this week) │
│ [Enter] resume  [s] shelf  [t] stats  [i] inbox  [?]  │
└───────────────────────────────────────────────────────┘
```
Keys: `j`/`k` move, `Enter` resume, `a` add, `f` finish, `g` goal, `e` edit/delete session, `Tab` switch screen, `?` help, `q` quit.

## TUI framework decision
Ratatui 0.30 + crossterm 0.29, the same stack as bookokrat and herdr. Hand-rolled Elm-style loop (`App`, `Msg`, `update`, `view`), synchronous code in the TUI.

| Option | Why not |
|---|---|
| Cursive 0.21 | retained mode with callbacks; last release Aug 2024; weak chart widgets |
| iocraft 0.9 | React-like and pleasant, but pre-1.0 with no chart/calendar widgets |
| tui-realm 4.1 | component framework on ratatui; overkill for 3-4 views |
| r3bl_tui, rat-salsa | heavyweight or niche |

Widgets used: `Gauge`, `calendar::Monthly` (feature `widget-calendar`, uses the `time` crate) for the heatmap, `BarChart`, `Sparkline`, `Table`/`List`.

Crates: `ratatui`, `crossterm`, `tui-input` (text fields), `rusqlite` (bundled), `time`, `lettre`, `keyring`, `insta` + ratatui `TestBackend` for GIVEN/WHEN/THEN snapshot tests, `lopdf` for PDF page labels, optionally `tachyonfx` for a small effect when the goal is hit.

## Permissions (one-time, checked by `readr doctor`)
- Full Disk Access for the `readr` binary: needed for the Apple Books container. It worked from herdr only because herdr already has it.
- Automation for Preview: read the front document's path.
- Accessibility (optional): window-focus layer; `osascript` currently gets "not allowed assistive access".

## Build order
| Phase | Deliverable | Done when |
|---|---|---|
| 0 | Book shortlist per goal from `~/Documents/Books` and Apple Books | 3-4 active books chosen |
| 1 | daemon tracking with the 3 adapters, SQLite, `readr status` | a week of real reading matches a stopwatch |
| 2 | reminders, nightly email, `readr init` / `doctor` | an under-20-min day gets reminders and one email |
| 3 | TUI: Today, Shelf, Stats, Inbox, resume | used daily instead of `status` |
| 4 | extras | reading line added to `DailyNotes/YYYY-MM-DD.md`, weekly summary email, per-book notes |

## Open questions to verify in Phase 1
- Does Preview write `ViewState` only when a document closes? (Timestamps suggest yes; fine for the end position, not for page-turn detection.)
- Does the Apple Books type-3 location row update on every page turn?
- Does `ibooks://assetid/<id>` open a book at its saved position?
- Is a file opened by path in bookokrat tied to the folder's library?
- Can a launchd process reach the herdr socket for notifications?
- Do "read elsewhere" iCloud sessions count toward the daily goal?
- Printed vs physical page numbers in PDFs: show page labels from the PDF.

## Implementation decisions
- One binary (`readr`) with `daemon` subcommands instead of separate daemon and TUI binaries. macOS grants Full Disk Access, Automation and Accessibility per binary, so one binary means one set of permission prompts. The workspace is `readr-core` (config, database, session engine, reader adapters) and `readr` (CLI, daemon loop, TUI).
- `jiff` for all dates and time zones instead of `time`. `time` cannot read the local UTC offset reliably in a multi-threaded process; `jiff` reads the system time zone database. Dates are converted to `time::Date` only at the ratatui calendar widget.
- Open sessions are written to the database on every poll, so `status`, reminders and the TUI include the session in progress and a daemon restart loses nothing.
- The Stats heatmap is drawn directly with ratatui spans instead of the `calendar::Monthly` widget, which would need the `time` crate next to `jiff`.
- The SMTP password is stored with the macOS `security` CLI (Keychain item `readr-smtp`) instead of the `keyring` crate. That means no extra dependency, and the item is visible in Keychain Access.
- Text input in the TUI (search, minutes) is a plain string buffer; `tui-input` was not needed.
- Reminders go to macOS Notification Center through `terminal-notifier` by default (`notify.channel = "system"`), falling back to `osascript`. herdr is optional (`notify.channel = "herdr"`): with herdr's "inside herdr" delivery, a toast in a window that isn't in front still reports `"shown":true`, so reminders could be missed while reading in another app, and with "via system" herdr only forwarded to terminal-notifier anyway. When herdr is used, its response is checked for `"shown":true` because a rate-limited notification still exits 0, and herdr finds its socket through `XDG_CONFIG_HOME`, so the daemon gets the same value as the shell.
- Verified in bookokrat's source: `pdf_page` is 0-based and `total_chapters` is the page count for PDFs.
- Apple Books collections are read from `ZBKCOLLECTIONMEMBER` / `ZBKCOLLECTION` and used for goal suggestions.
- Preview titles come from Spotlight (`kMDItemTitle`), falling back to a cleaned-up file name.

- Window focus (built after the first version): the daemon reads the front app's focused window title through the Accessibility API (`AXFocusedWindow` / `AXTitle`, small FFI in `readers/ax.rs`). bookokrat sets its terminal title to `bookokrat`, a dash (U+2014) and the file stem (verified in its source and on 0.3.12), Rio shows it as the window title, and the stem is matched to a known bookmark, preferring the most recently read when a PDF and an EPUB share a stem. `--test-mode` does not set the title.
- Away detection replaces the 2-minute idle cutoff for every reader. With a reader in front, focused checks continue without input up to `away_minutes`. Past that, an `Away` event trims the open session to `last input + max_page_minutes`: a long page counts in full when you keep reading, and leaving a book open over lunch counts no more than an ordinary page.
- macOS keys Accessibility and Full Disk Access to the code signature. Ad-hoc builds change it every time, so `make install CODESIGN_IDENTITY=<certificate>` signs with a stable identity.

- A saved place is not reading done: you can jump ahead, and anything before readr was untracked. The shelf shows the place as "at p.193/307", never as a percentage read, and time left uses only forward progress inside tracked sessions, capped at 1% of the book per minute so jumps don't count.

- Forgetting a book marks every copy of it untracked (`book_sources.untracked`) and records its normalised title in `untracked_titles`, so a new copy with the same title is skipped too. The rows stay, so readr knows what to skip; sessions are deleted unless `--keep-history`. `readr track` and `readr shelf add` clear both marks.

## Not built yet
- "Read elsewhere" sessions from iCloud-synced Apple Books progress.
- PDF page labels (printed page numbers) for display.
- Daily note line in the notes vault, weekly summary email.
