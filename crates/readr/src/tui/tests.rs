use std::collections::BTreeMap;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use readr_core::config::Config;
use readr_core::db::{Book, Move, SessionView, Source};
use readr_core::model::{BookStatus, Position, Reader};
use readr_core::settings::SETTINGS;

use super::data::{Data, GoalWeek, NextView};
use super::model::{Effect, Mode, Model, Screen, SettingsRow};
use super::view;
use crate::shelf::ShelfRow;

fn today() -> Date {
    Date::constant(2026, 10, 16)
}

fn at(time: &str) -> Timestamp {
    format!("2026-10-16T{time}+11:00").parse().unwrap()
}

fn book(id: i64, title: &str, goal: &str, status: BookStatus) -> Book {
    Book {
        id,
        title: title.into(),
        author: None,
        goal: Some(goal.into()),
        status,
        priority: 0,
        added_at: Timestamp::UNIX_EPOCH,
        finished_at: None,
    }
}

fn source(id: i64, book_id: Option<i64>, title: &str, reader: Reader) -> Source {
    Source {
        id,
        book_id,
        reader,
        locator: format!("/Books/{title}.pdf"),
        title: Some(title.into()),
        author: None,
        ignored: false,
        last_position: Some(Position::new(Some(0.2), Some("p.9/300".into()))),
        last_seen_at: Some(at("09:00")),
        collections: Vec::new(),
        untracked: false,
    }
}

fn session(id: i64, title: &str, minutes: i64, open: bool) -> SessionView {
    SessionView {
        id,
        source_id: id,
        book_id: Some(id),
        title: title.into(),
        reader: Reader::Bookokrat,
        goal: Some("communication".into()),
        day: today(),
        started_at: at("08:00"),
        ended_at: at("08:07"),
        active: SignedDuration::from_mins(minutes),
        start_position: Some(Position::new(None, Some("p.4".into()))),
        end_position: Some(Position::new(None, Some("p.9".into()))),
        open,
        edited: false,
        note: None,
    }
}

fn data() -> Data {
    let shelf = vec![
        ShelfRow {
            book: book(1, "Walden", "communication", BookStatus::Active),
            goal_label: "Communication".into(),
            source: Some(source(1, Some(1), "Walden", Reader::Bookokrat)),
            position: Some(Position::new(Some(0.2), Some("p.60/300".into()))),
            read: 0.05,
            time: SignedDuration::from_mins(40),
            last_read: Some(at("08:07")),
        },
        ShelfRow {
            book: book(2, "Ulysses", "communication", BookStatus::Queued),
            goal_label: "Communication".into(),
            source: None,
            position: None,
            read: 0.0,
            time: SignedDuration::ZERO,
            last_read: None,
        },
        ShelfRow {
            book: book(3, "Dune", "health", BookStatus::Queued),
            goal_label: "Health".into(),
            source: None,
            position: None,
            read: 0.0,
            time: SignedDuration::ZERO,
            last_read: None,
        },
    ];
    Data {
        tz: TimeZone::get("Australia/Sydney").unwrap(),
        today: today(),
        total: SignedDuration::from_mins(12),
        goal: SignedDuration::from_mins(20),
        streak: 6,
        best_streak: 9,
        sessions: vec![
            session(1, "Walden", 7, false),
            session(2, "Meditations", 5, true),
        ],
        next: Some(NextView {
            book_id: 1,
            text: "Walden p.9/300 (bookokrat)".into(),
            reason: "Communication · least read this week".into(),
        }),
        shelf,
        candidates: vec![
            source(10, None, "The Origin of Species", Reader::Preview),
            source(11, None, "Middlemarch", Reader::AppleBooks),
        ],
        inbox: vec![(
            source(12, None, "Random Paper", Reader::Preview),
            SignedDuration::from_mins(9),
        )],
        heat: BTreeMap::from([(today(), SignedDuration::from_mins(12))]),
        week_goals: vec![GoalWeek {
            key: "communication".into(),
            label: "Communication".into(),
            color: Some("blue".into()),
            read: SignedDuration::from_mins(40),
            target: Some(60),
        }],
        goals: vec![
            ("health".into(), "Health".into()),
            ("communication".into(), "Communication".into()),
        ],
        config: config(),
    }
}

fn config() -> Config {
    Config::parse(
        r#"
        daily_goal_minutes = 20

        [[goal]]
        key = "health"
        label = "Health"

        [[goal]]
        key = "communication"
        label = "Communication"
        color = "blue"
        weekly_minutes = 60
        "#,
    )
    .unwrap()
}

fn open_settings_at(model: &mut Model, row: &SettingsRow) {
    press(model, KeyCode::Char('s'));
    let index = model.settings_rows().iter().position(|r| r == row).unwrap();
    for _ in 0..index {
        press(model, KeyCode::Char('j'));
    }
}

fn setting_row(key: &str) -> SettingsRow {
    SettingsRow::Setting(SETTINGS.iter().find(|f| f.key == key).unwrap())
}

fn press(model: &mut Model, code: KeyCode) -> Option<Effect> {
    model.update(KeyEvent::from(code))
}

fn render(model: &Model) -> String {
    render_sized(model, 120, 32)
}

fn render_sized(model: &Model, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| view::draw(frame, model)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    text
}

#[test]
fn today_screen_shows_progress_sessions_and_up_next() {
    // given
    let model = Model::new(data());

    // when
    let screen = render(&model);

    // then
    assert!(screen.contains("12 / 20 min"), "{screen}");
    assert!(screen.contains("Walden"));
    assert!(screen.contains("p.4 → p.9"));
    assert!(screen.contains("reading"));
    assert!(screen.contains("Communication · least read this week"));
    assert!(screen.contains("streak 6"));
}

#[test]
fn enter_on_today_resumes_the_suggested_book() {
    // given
    let mut model = Model::new(data());

    // when
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert_eq!(effect, Some(Effect::Resume(1)));
}

#[test]
fn tab_cycles_through_the_screens() {
    // given
    let mut model = Model::new(data());

    // when
    let visited: Vec<Screen> = (0..4)
        .map(|_| {
            press(&mut model, KeyCode::Tab);
            model.screen
        })
        .collect();

    // then
    assert_eq!(
        visited,
        vec![Screen::Shelf, Screen::Stats, Screen::Inbox, Screen::Today]
    );
}

#[test]
fn shelf_keys_finish_queue_and_resume_the_selected_book() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('2'));

    // when
    let resume = press(&mut model, KeyCode::Enter);
    let finish = press(&mut model, KeyCode::Char('f'));
    press(&mut model, KeyCode::Char('j'));
    let activate = press(&mut model, KeyCode::Char('u'));

    // then
    assert_eq!(resume, Some(Effect::Resume(1)));
    assert_eq!(finish, Some(Effect::SetStatus(1, BookStatus::Done)));
    assert_eq!(activate, Some(Effect::SetStatus(2, BookStatus::Active)));
}

#[test]
fn shift_j_and_k_move_the_book_and_the_selection_follows_it() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('2'));
    press(&mut model, KeyCode::Char('j'));

    // when
    let down = press(&mut model, KeyCode::Char('J'));
    let selected_after_down = model.shelf_selected;
    let up = press(&mut model, KeyCode::Char('K'));

    // then
    assert_eq!(down, Some(Effect::MoveBook(2, Move::Down)));
    assert_eq!(selected_after_down, 2);
    assert_eq!(up, Some(Effect::MoveBook(3, Move::Up)));
}

#[test]
fn moving_stops_at_the_edge_of_the_section() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('2'));
    press(&mut model, KeyCode::Char('j'));

    // when
    let effect = press(&mut model, KeyCode::Char('K'));

    // then
    assert_eq!(effect, Some(Effect::MoveBook(2, Move::Up)));
    assert_eq!(model.shelf_selected, 1);
}

#[test]
fn goal_picker_starts_on_the_current_goal_and_sets_the_chosen_one() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('2'));
    press(&mut model, KeyCode::Char('g'));
    assert!(matches!(model.mode, Mode::Goal { selected: 1, .. }));

    // when
    press(&mut model, KeyCode::Char('k'));
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert_eq!(
        effect,
        Some(Effect::SetGoal {
            book_id: 1,
            goal: Some("health".into()),
            move_history: false
        })
    );
}

#[test]
fn add_picker_filters_as_you_type() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('a'));

    // when
    for c in "middle".chars() {
        press(&mut model, KeyCode::Char(c));
    }
    let screen = render(&model);
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert!(screen.contains("search: middle"), "{screen}");
    assert!(!screen.contains("The Origin of Species"));
    assert_eq!(effect, Some(Effect::AddSource(11)));
}

#[test]
fn editing_a_finished_session_sets_its_minutes() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('e'));

    // when
    press(&mut model, KeyCode::Backspace);
    press(&mut model, KeyCode::Char('4'));
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert_eq!(
        effect,
        Some(Effect::EditSession {
            session_id: 1,
            minutes: 4
        })
    );
}

#[test]
fn a_running_session_cannot_be_edited() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('j'));

    // when
    let effect = press(&mut model, KeyCode::Char('e'));

    // then
    assert_eq!(effect, None);
    assert_eq!(model.mode, Mode::Normal);
    assert_eq!(
        model.message.as_deref(),
        Some("this session is still running")
    );
}

#[test]
fn deleting_a_session_needs_confirmation() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('x'));

    // when
    let effect = press(&mut model, KeyCode::Char('y'));

    // then
    assert_eq!(effect, Some(Effect::DeleteSession(1)));
}

#[test]
fn inbox_items_can_be_added_or_ignored() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('4'));

    // when
    let screen = render(&model);
    let add = press(&mut model, KeyCode::Enter);
    let ignore = press(&mut model, KeyCode::Char('i'));

    // then
    assert!(screen.contains("Random Paper"), "{screen}");
    assert_eq!(add, Some(Effect::AddSource(12)));
    assert_eq!(ignore, Some(Effect::Ignore(12)));
}

#[test]
fn stats_screen_shows_streaks_and_goal_minutes() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('3'));

    // when
    let screen = render(&model);

    // then
    assert!(screen.contains("best streak  9 days"), "{screen}");
    assert!(screen.contains("40/60"));
    assert!(screen.contains("Last 16 weeks"));
}

#[test]
fn shelf_screen_lists_active_and_queued_books() {
    // given
    let mut model = Model::new(data());
    press(&mut model, KeyCode::Char('2'));

    // when
    let screen = render(&model);

    // then
    assert!(screen.contains("Shelf · 1 active"), "{screen}");
    assert!(screen.contains("Ulysses"));
    assert!(screen.contains("queued"));
    assert!(screen.contains("p.60/300"));
}

#[test]
fn q_quits() {
    // given
    let mut model = Model::new(data());

    // when
    press(&mut model, KeyCode::Char('q'));

    // then
    assert!(model.quit);
}

#[test]
fn s_opens_settings_with_every_section_and_the_goals() {
    // given
    let mut model = Model::new(data());

    // when
    press(&mut model, KeyCode::Char('s'));
    let screen = render_sized(&model, 120, 48);

    // then
    for text in [
        "Settings",
        "Daily goal (min)",
        "Reminders",
        "Notify via",
        "Email",
        "Tracking",
        "Goals",
        "60 min/week",
        "shares the daily goal",
        "+ add goal",
        "Minutes to read each day",
    ] {
        assert!(screen.contains(text), "{text} missing\n{screen}");
    }
}

#[test]
fn a_short_terminal_scrolls_settings_to_the_selected_row() {
    // given
    let mut model = Model::new(data());

    // when
    open_settings_at(&mut model, &SettingsRow::AddGoal);
    let screen = render(&model);

    // then
    assert!(screen.contains("+ add goal"), "{screen}");
    assert!(!screen.contains("Daily goal (min)"), "{screen}");
}

#[test]
fn enter_on_a_choice_switches_to_the_next_value() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &setting_row("notify.channel"));

    // when
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert_eq!(
        effect,
        Some(Effect::SetSetting {
            key: "notify.channel",
            value: "herdr".into()
        })
    );
    assert!(matches!(model.mode, Mode::Settings { input: None, .. }));
}

#[test]
fn enter_on_a_value_edits_it_starting_from_the_current_one() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &setting_row("daily_goal_minutes"));

    // when
    press(&mut model, KeyCode::Enter);
    let editing = render(&model);
    press(&mut model, KeyCode::Backspace);
    press(&mut model, KeyCode::Backspace);
    press(&mut model, KeyCode::Char('4'));
    press(&mut model, KeyCode::Char('5'));
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert!(editing.contains("20▏"), "{editing}");
    assert_eq!(
        effect,
        Some(Effect::SetSetting {
            key: "daily_goal_minutes",
            value: "45".into()
        })
    );
    assert!(matches!(model.mode, Mode::Settings { input: None, .. }));
}

#[test]
fn esc_cancels_an_edit_and_then_closes_settings() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &setting_row("nightly.at"));
    press(&mut model, KeyCode::Enter);

    // when
    let cancelled = press(&mut model, KeyCode::Esc);
    let still_open = matches!(model.mode, Mode::Settings { input: None, .. });
    press(&mut model, KeyCode::Esc);

    // then
    assert_eq!(cancelled, None);
    assert!(still_open);
    assert_eq!(model.mode, Mode::Normal);
    assert!(!model.quit);
}

#[test]
fn a_goal_opens_its_fields_and_esc_returns_to_it() {
    // given
    let mut model = Model::new(data());
    let goal = SettingsRow::Goal("communication".into());
    open_settings_at(&mut model, &goal);

    // when
    press(&mut model, KeyCode::Enter);
    let screen = render(&model);
    press(&mut model, KeyCode::Char('j'));
    press(&mut model, KeyCode::Char('j'));
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Char('0'));
    let effect = press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Esc);

    // then
    assert!(screen.contains("Goal · Communication"), "{screen}");
    assert_eq!(
        effect,
        Some(Effect::SetGoalField {
            goal: "communication".into(),
            field: "weekly_minutes",
            value: "600".into()
        })
    );
    let selected = model.settings_rows().iter().position(|r| *r == goal);
    assert!(matches!(model.mode, Mode::Settings { selected: s, .. } if Some(s) == selected));
}

#[test]
fn renaming_a_goal_key_keeps_editing_the_renamed_goal() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &SettingsRow::Goal("health".into()));
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Char('j'));
    press(&mut model, KeyCode::Enter);
    for c in "-body".chars() {
        press(&mut model, KeyCode::Char(c));
    }

    // when
    let effect = press(&mut model, KeyCode::Enter);
    let mut renamed = data();
    renamed.config = renamed
        .config
        .with_goal_field("health", "key", "health-body")
        .unwrap();
    model.replace_data(renamed);

    // then
    assert_eq!(
        effect,
        Some(Effect::SetGoalField {
            goal: "health".into(),
            field: "key",
            value: "health-body".into()
        })
    );
    assert!(matches!(&model.mode, Mode::GoalSettings { key, .. } if key == "health-body"));
}

#[test]
fn a_rejected_rename_stays_on_the_old_goal() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &SettingsRow::Goal("health".into()));
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Char('j'));
    press(&mut model, KeyCode::Enter);
    press(&mut model, KeyCode::Char('!'));
    press(&mut model, KeyCode::Enter);

    // when
    model.replace_data(data());

    // then
    assert!(matches!(&model.mode, Mode::GoalSettings { key, .. } if key == "health"));
}

#[test]
fn shift_j_moves_a_goal_and_the_selection_follows() {
    // given
    let mut model = Model::new(data());
    let health = SettingsRow::Goal("health".into());
    open_settings_at(&mut model, &health);
    let before = model
        .settings_rows()
        .iter()
        .position(|r| *r == health)
        .unwrap();

    // when
    let effect = press(&mut model, KeyCode::Char('J'));

    // then
    assert_eq!(
        effect,
        Some(Effect::MoveGoal {
            goal: "health".into(),
            delta: 1
        })
    );
    assert!(matches!(model.mode, Mode::Settings { selected, .. } if selected == before + 1));
}

#[test]
fn removing_a_goal_needs_confirmation() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &SettingsRow::Goal("health".into()));
    press(&mut model, KeyCode::Char('d'));

    // when
    let screen = render(&model);
    let effect = press(&mut model, KeyCode::Char('y'));

    // then
    assert!(screen.contains("Remove Health?"), "{screen}");
    assert_eq!(effect, Some(Effect::RemoveGoal("health".into())));
}

#[test]
fn a_new_goal_is_named_in_place() {
    // given
    let mut model = Model::new(data());
    open_settings_at(&mut model, &SettingsRow::AddGoal);

    // when
    press(&mut model, KeyCode::Enter);
    for c in "Horizon".chars() {
        press(&mut model, KeyCode::Char(c));
    }
    let effect = press(&mut model, KeyCode::Enter);

    // then
    assert_eq!(effect, Some(Effect::AddGoal("Horizon".into())));
}
