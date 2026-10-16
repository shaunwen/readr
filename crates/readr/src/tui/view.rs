use std::str::FromStr;

use jiff::{SignedDuration, Span};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span as TextSpan};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row,
    Table, TableState, Tabs,
};
use readr_core::db::SessionView;
use readr_core::model::{BookStatus, Position, format_progress};
use readr_core::stats::{format_minutes, minutes};

use super::data::{Data, HEATMAP_WEEKS};
use super::model::{Mode, Model, Screen};

const ACCENT: Color = Color::Cyan;

pub fn draw(frame: &mut Frame, model: &Model) {
    let [tabs, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_tabs(frame, tabs, model);
    match model.screen {
        Screen::Today => draw_today(frame, body, model),
        Screen::Shelf => draw_shelf(frame, body, model),
        Screen::Stats => draw_stats(frame, body, &model.data),
        Screen::Inbox => draw_inbox(frame, body, model),
    }
    draw_footer(frame, footer, model);

    match &model.mode {
        Mode::Normal => {}
        Mode::Help => draw_help(frame),
        Mode::Add { query, selected } => draw_add(frame, model, query, *selected),
        Mode::Goal { selected, .. } => draw_goal_picker(frame, model, *selected),
        Mode::Link { selected, .. } => draw_link_picker(frame, model, *selected),
        Mode::EditMinutes { input, .. } => draw_prompt(
            frame,
            "Edit session",
            &format!("minutes: {input}▏"),
            "Enter save · Esc cancel",
        ),
        Mode::ConfirmDelete { .. } => draw_prompt(
            frame,
            "Delete session",
            "Delete this session?",
            "y delete · any other key cancels",
        ),
    }
}

fn draw_tabs(frame: &mut Frame, area: Rect, model: &Model) {
    let inbox = model.data.inbox.len();
    let titles = vec![
        "1 Today".to_string(),
        "2 Shelf".to_string(),
        "3 Stats".to_string(),
        if inbox > 0 {
            format!("4 Inbox ({inbox})")
        } else {
            "4 Inbox".to_string()
        },
    ];
    let [left, right] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(32)]).areas(area);
    frame.render_widget(
        Tabs::new(titles)
            .select(model.screen.index())
            .highlight_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
            .divider("│"),
        left,
    );
    let header = format!(
        "readr · {} · streak {} ",
        model.data.today.strftime("%a %d %b"),
        model.data.streak
    );
    frame.render_widget(Line::from(header).right_aligned().dim(), right);
}

fn draw_footer(frame: &mut Frame, area: Rect, model: &Model) {
    let line = match &model.message {
        Some(message) => Line::from(message.clone()).fg(Color::Yellow),
        None => {
            let keys = match model.screen {
                Screen::Today => "Enter resume up next · j/k select · e edit · x delete",
                Screen::Shelf => {
                    "Enter resume · a add · f finish · d drop · u queue/activate · g goal · G goal+history"
                }
                Screen::Stats => "",
                Screen::Inbox => "Enter add to shelf · n link to a book · i ignore",
            };
            Line::from(format!(" {keys} · Tab screens · ? help · q quit")).dim()
        }
    };
    frame.render_widget(line, area);
}

fn draw_today(frame: &mut Frame, area: Rect, model: &Model) {
    let data = &model.data;
    let [gauge_area, sessions_area, next_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(4),
        Constraint::Length(3),
    ])
    .areas(area);

    let ratio = (data.total.as_secs_f64() / data.goal.as_secs_f64().max(1.0)).clamp(0.0, 1.0);
    let done = data.total >= data.goal;
    let label = format!(
        "{} / {} min{}",
        minutes(data.total),
        minutes(data.goal),
        if done { "  done" } else { "" }
    );
    frame.render_widget(
        Gauge::default()
            .block(Block::bordered().title(" Today "))
            .gauge_style(Style::new().fg(if done { Color::Green } else { ACCENT }))
            .ratio(ratio)
            .label(label),
        gauge_area,
    );

    let rows: Vec<Row> = data
        .sessions
        .iter()
        .map(|session| session_row(data, session))
        .collect();
    let empty = rows.is_empty();
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Min(20),
            Constraint::Length(9),
            Constraint::Length(6),
            Constraint::Length(24),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(["start", "book", "reader", "time", "position", ""]).dim())
    .block(Block::bordered().title(" Sessions "))
    .row_highlight_style(Style::new().reversed());
    let mut state = TableState::default().with_selected((!empty).then_some(model.today_selected));
    frame.render_stateful_widget(table, sessions_area, &mut state);
    if empty {
        let inner = sessions_area.inner(ratatui::layout::Margin::new(2, 2));
        frame.render_widget(Line::from("no reading yet today").dim(), inner);
    }

    let next = match &data.next {
        Some(next) => Line::from(vec![
            TextSpan::from(next.text.clone()).bold(),
            TextSpan::from(format!("  {}", next.reason)).dim(),
        ]),
        None => Line::from("add a book to your shelf with a").dim(),
    };
    frame.render_widget(
        Paragraph::new(next).block(Block::bordered().title(" Up next (Enter) ")),
        next_area,
    );
}

fn session_row<'a>(data: &Data, session: &'a SessionView) -> Row<'a> {
    let started = session.started_at.to_zoned(data.tz.clone());
    let flag = if session.open {
        "reading".to_string()
    } else if session.book_id.is_none() {
        "off shelf".to_string()
    } else if session.edited {
        "edited".to_string()
    } else {
        String::new()
    };
    let style = match data.goal_color(session.goal.as_deref()) {
        Some(color) => Style::new().fg(parse_color(color)),
        None => Style::new(),
    };
    Row::new(vec![
        Cell::from(started.strftime("%H:%M").to_string()),
        Cell::from(session.title.clone()).style(style),
        Cell::from(session.reader.label()),
        Cell::from(format_minutes(session.active)),
        Cell::from(movement(&session.start_position, &session.end_position)),
        Cell::from(flag).dim(),
    ])
}

fn movement(start: &Option<Position>, end: &Option<Position>) -> String {
    match (start, end) {
        (Some(start), Some(end)) if start != end => {
            format!("{} → {}", start.display(), end.display())
        }
        (_, Some(end)) => end.display(),
        (Some(start), None) => start.display(),
        (None, None) => String::new(),
    }
}

fn draw_shelf(frame: &mut Frame, area: Rect, model: &Model) {
    let data = &model.data;
    let rows: Vec<Row> = data
        .shelf
        .iter()
        .map(|row| {
            let goal_style = match data.goal_color(row.book.goal.as_deref()) {
                Some(color) => Style::new().fg(parse_color(color)),
                None => Style::new().dim(),
            };
            let status_style = match row.book.status {
                BookStatus::Active => Style::new(),
                _ => Style::new().dim(),
            };
            let last = row
                .last_read
                .map(|at| relative(data, at))
                .unwrap_or_else(|| "never".into());
            Row::new(vec![
                Cell::from(row.goal_label.clone()).style(goal_style),
                Cell::from(row.book.title.clone()),
                Cell::from(row.book.status.as_str()),
                Cell::from(
                    row.progress
                        .map(format_progress)
                        .unwrap_or_else(|| "-".into()),
                ),
                Cell::from(row.source.as_ref().map(|s| s.reader.label()).unwrap_or("-")),
                Cell::from(last),
                Cell::from(format_minutes(row.time)),
                Cell::from(
                    row.time_left()
                        .map(|left| format!("~{}", format_minutes(left)))
                        .unwrap_or_default(),
                ),
            ])
            .style(status_style)
        })
        .collect();
    let empty = rows.is_empty();
    let active = data
        .shelf
        .iter()
        .filter(|r| r.book.status == BookStatus::Active)
        .count();
    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Min(24),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(7),
            Constraint::Length(7),
        ],
    )
    .header(
        Row::new([
            "goal",
            "book",
            "status",
            "done",
            "reader",
            "last read",
            "time",
            "left",
        ])
        .dim(),
    )
    .block(Block::bordered().title(format!(" Shelf · {active} active ")))
    .row_highlight_style(Style::new().reversed());
    let mut state = TableState::default().with_selected((!empty).then_some(model.shelf_selected));
    frame.render_stateful_widget(table, area, &mut state);
    if empty {
        let inner = area.inner(ratatui::layout::Margin::new(2, 2));
        frame.render_widget(
            Line::from("the shelf is empty; press a to add a book").dim(),
            inner,
        );
    }
}

fn relative(data: &Data, at: jiff::Timestamp) -> String {
    let day = at.to_zoned(data.tz.clone()).date();
    match (data.today - day).get_days() {
        0 => "today".into(),
        1 => "yesterday".into(),
        days @ 2..=13 => format!("{days}d ago"),
        _ => day.strftime("%d %b").to_string(),
    }
}

fn draw_stats(frame: &mut Frame, area: Rect, data: &Data) {
    let [top, bottom] = Layout::vertical([Constraint::Length(11), Constraint::Min(8)]).areas(area);
    let [heat_area, summary_area] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(30)]).areas(top);
    draw_heatmap(frame, heat_area, data);

    let week: SignedDuration = (0..7)
        .map(|offset| {
            let day = data.today.saturating_sub(Span::new().days(offset));
            data.heat.get(&day).copied().unwrap_or_default()
        })
        .sum();
    let met = data
        .heat
        .values()
        .filter(|total| **total >= data.goal)
        .count();
    let summary = vec![
        Line::from(format!("streak       {} days", data.streak)),
        Line::from(format!("best streak  {} days", data.best_streak)),
        Line::from(format!("last 7 days  {}", format_minutes(week))),
        Line::from(format!("goal met     {met} days")),
        Line::from(format!("daily goal   {} min", minutes(data.goal))),
    ];
    frame.render_widget(
        Paragraph::new(summary).block(Block::bordered().title(" Summary ")),
        summary_area,
    );

    let [goals_area, days_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(bottom);
    let goal_bars: Vec<Bar> = data
        .week_goals
        .iter()
        .map(|goal| {
            let read = minutes(goal.read).max(0) as u64;
            let text = match goal.target {
                Some(target) => format!("{read}/{target}"),
                None => read.to_string(),
            };
            let color = goal.color.as_deref().map(parse_color).unwrap_or(ACCENT);
            Bar::default()
                .value(read)
                .text_value(text)
                .label(Line::from(short_label(&goal.label)))
                .style(Style::new().fg(color))
        })
        .collect();
    frame.render_widget(
        BarChart::default()
            .block(Block::bordered().title(" This week by goal (min) "))
            .data(BarGroup::default().bars(&goal_bars))
            .bar_width(9)
            .bar_gap(2),
        goals_area,
    );

    let day_bars: Vec<Bar> = (0..7)
        .rev()
        .map(|offset| {
            let day = data.today.saturating_sub(Span::new().days(offset));
            let total = data.heat.get(&day).copied().unwrap_or_default();
            let color = if total >= data.goal {
                Color::Green
            } else {
                ACCENT
            };
            Bar::default()
                .value(minutes(total).max(0) as u64)
                .label(Line::from(day.strftime("%a").to_string()))
                .style(Style::new().fg(color))
        })
        .collect();
    frame.render_widget(
        BarChart::default()
            .block(Block::bordered().title(" Last 7 days (min) "))
            .data(BarGroup::default().bars(&day_bars))
            .bar_width(4)
            .bar_gap(1),
        days_area,
    );
}

fn short_label(label: &str) -> String {
    label.chars().take(9).collect()
}

fn draw_heatmap(frame: &mut Frame, area: Rect, data: &Data) {
    let this_monday = data
        .today
        .saturating_sub(Span::new().days(i64::from(data.today.weekday().to_monday_zero_offset())));
    let first_monday = this_monday.saturating_sub(Span::new().weeks(HEATMAP_WEEKS - 1));
    let names = ["Mon", "   ", "Wed", "   ", "Fri", "   ", "Sun"];
    let mut lines = Vec::new();
    for (weekday, name) in names.iter().enumerate() {
        let mut spans = vec![TextSpan::from(format!("{name} ")).dim()];
        for week in 0..HEATMAP_WEEKS {
            let day = first_monday.saturating_add(Span::new().weeks(week).days(weekday as i64));
            if day > data.today {
                spans.push(TextSpan::from("  "));
                continue;
            }
            let total = data.heat.get(&day).copied().unwrap_or_default();
            spans.push(heat_cell(total, data.goal));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(vec![
        TextSpan::from("    less ").dim(),
        heat_cell(SignedDuration::ZERO, data.goal),
        heat_cell(data.goal / 4, data.goal),
        heat_cell(data.goal * 3 / 4, data.goal),
        heat_cell(data.goal, data.goal),
        heat_cell(data.goal * 2, data.goal),
        TextSpan::from(" more").dim(),
    ]));
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(format!(" Last {HEATMAP_WEEKS} weeks "))),
        area,
    );
}

fn heat_cell(total: SignedDuration, goal: SignedDuration) -> TextSpan<'static> {
    let color = if total <= SignedDuration::ZERO {
        Color::DarkGray
    } else if total < goal / 2 {
        Color::Rgb(14, 68, 41)
    } else if total < goal {
        Color::Rgb(0, 109, 50)
    } else if total < goal * 2 {
        Color::Rgb(38, 166, 65)
    } else {
        Color::Rgb(57, 211, 83)
    };
    let glyph = if total <= SignedDuration::ZERO {
        "· "
    } else {
        "■ "
    };
    TextSpan::styled(glyph, Style::new().fg(color))
}

fn draw_inbox(frame: &mut Frame, area: Rect, model: &Model) {
    let rows: Vec<Row> = model
        .data
        .inbox
        .iter()
        .map(|(source, time)| {
            Row::new(vec![
                Cell::from(source.display_title()),
                Cell::from(source.reader.label()),
                Cell::from(format_minutes(*time)),
                Cell::from(source.locator.clone()).dim(),
            ])
        })
        .collect();
    let empty = rows.is_empty();
    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Percentage(40),
        ],
    )
    .header(Row::new(["book", "reader", "time", "where"]).dim())
    .block(Block::bordered().title(" Inbox · reading not on the shelf "))
    .row_highlight_style(Style::new().reversed());
    let mut state = TableState::default().with_selected((!empty).then_some(model.inbox_selected));
    frame.render_stateful_widget(table, area, &mut state);
    if empty {
        let inner = area.inner(ratatui::layout::Margin::new(2, 2));
        frame.render_widget(Line::from("nothing here").dim(), inner);
    }
}

fn popup(frame: &mut Frame, width: u16, height: u16, title: &str) -> Rect {
    let area = frame.area();
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(ACCENT));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}

fn draw_prompt(frame: &mut Frame, title: &str, text: &str, hint: &str) {
    let inner = popup(frame, 44, 5, title);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(text.to_string()),
            Line::from(hint.to_string()).dim(),
        ]),
        inner,
    );
}

fn draw_add(frame: &mut Frame, model: &Model, query: &str, selected: usize) {
    let inner = popup(frame, 90, 20, "Add to shelf");
    let [input, list, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);
    frame.render_widget(Line::from(format!("search: {query}▏")), input);
    let matches = model.add_matches(query);
    let items: Vec<ListItem> = matches
        .iter()
        .map(|source| {
            let position = source
                .last_position
                .as_ref()
                .map(|p| format!("  {}", p.display()))
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                TextSpan::from(source.display_title()),
                TextSpan::from(format!("  {}{position}", source.reader.label())).dim(),
            ]))
        })
        .collect();
    let mut state = ListState::default().with_selected((!items.is_empty()).then_some(selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(Style::new().reversed()),
        list,
        &mut state,
    );
    frame.render_widget(
        Line::from("type to filter · ↑/↓ choose · Enter add · Esc cancel").dim(),
        hint,
    );
}

fn draw_goal_picker(frame: &mut Frame, model: &Model, selected: usize) {
    let options = model.goal_options();
    let inner = popup(frame, 40, options.len() as u16 + 3, "Goal");
    let items: Vec<ListItem> = options
        .iter()
        .map(|(_, label)| ListItem::new(label.clone()))
        .collect();
    let [list, hint] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(Style::new().reversed()),
        list,
        &mut state,
    );
    frame.render_widget(Line::from("Enter set · Esc cancel").dim(), hint);
}

fn draw_link_picker(frame: &mut Frame, model: &Model, selected: usize) {
    let inner = popup(frame, 60, 16, "Link to book");
    let items: Vec<ListItem> = model
        .data
        .shelf
        .iter()
        .map(|row| ListItem::new(row.book.title.clone()))
        .collect();
    let [list, hint] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        List::new(items).highlight_style(Style::new().reversed()),
        list,
        &mut state,
    );
    frame.render_widget(Line::from("Enter link · Esc cancel").dim(), hint);
}

fn draw_help(frame: &mut Frame) {
    let lines = [
        ("Tab / 1-4 / h l", "switch screen"),
        ("j k / ↓ ↑", "move"),
        ("Enter", "resume book (Today: up next)"),
        ("a", "add a book to the shelf"),
        ("f / d", "finish / drop book"),
        ("u", "queue or activate book"),
        ("g / G", "set goal / set goal and move past sessions"),
        ("e / x", "edit / delete a finished session"),
        ("n / i", "inbox: link to a book / ignore"),
        ("q / Esc", "quit"),
    ];
    let inner = popup(frame, 62, lines.len() as u16 + 3, "Keys");
    let text: Vec<Line> = lines
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                TextSpan::from(format!("{key:<18}")).fg(ACCENT),
                TextSpan::from(*action),
            ])
        })
        .chain([Line::from("press any key").dim()])
        .collect();
    frame.render_widget(Paragraph::new(text), inner);
}

fn parse_color(name: &str) -> Color {
    Color::from_str(name).unwrap_or(ACCENT)
}
