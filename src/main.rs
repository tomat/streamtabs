use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::{self, BufRead, IsTerminal, Read, Stdout, Write};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};

const MAX_STORED_LINES_PER_TAB: usize = 5_000;
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const PAUSED_LABEL: &str = " (paused)";

#[derive(Debug)]
enum InputMessage {
    Line(String),
    Closed,
    Error(String),
}

#[derive(Debug)]
enum UiMessage {
    NextTab,
    SelectTab(usize),
    TogglePause,
    ClearSelection,
    SelectMiddleVisibleLine,
    MouseLeftDown { column: u16, row: u16 },
    Quit,
    Error(String),
}

#[derive(Debug)]
enum MatchMode {
    All,
    Contains(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LineRecord {
    seq: u64,
    text: String,
}

#[derive(Debug)]
struct Tab {
    label: String,
    mode: MatchMode,
    lines: VecDeque<LineRecord>,
    total_matches: u64,
    seen_matches: u64,
}

impl Tab {
    fn new(filter: String) -> Self {
        Self {
            label: filter.clone(),
            mode: MatchMode::Contains(filter),
            lines: VecDeque::new(),
            total_matches: 0,
            seen_matches: 0,
        }
    }

    fn unfiltered() -> Self {
        Self {
            label: "(all)".to_owned(),
            mode: MatchMode::All,
            lines: VecDeque::new(),
            total_matches: 0,
            seen_matches: 0,
        }
    }

    fn push_line(&mut self, seq: u64, line: &str) {
        self.lines.push_back(LineRecord {
            seq,
            text: line.to_owned(),
        });
        self.total_matches += 1;

        if self.lines.len() > MAX_STORED_LINES_PER_TAB {
            let _ = self.lines.pop_front();
        }
    }

    fn unread_matches(&self) -> u64 {
        self.total_matches.saturating_sub(self.seen_matches)
    }

    fn mark_seen_through(&mut self, max_match_index: u64) {
        let capped = max_match_index.min(self.total_matches);
        if capped > self.seen_matches {
            self.seen_matches = capped;
        }
    }

    fn matches(&self, line: &str) -> bool {
        match &self.mode {
            MatchMode::All => true,
            MatchMode::Contains(filter) => line.contains(filter),
        }
    }
}

#[derive(Debug)]
struct PauseSnapshot {
    line_cutoffs: Vec<usize>,
    match_cutoffs: Vec<u64>,
}

#[derive(Debug, Clone)]
struct SelectedLine {
    seq: u64,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedLine {
    seq: u64,
    text: String,
    selected: bool,
}

#[derive(Debug, Clone, Copy)]
struct TabHitbox {
    index: usize,
    left: u16,
    right: u16,
}

#[derive(Debug, Default, Clone)]
struct RenderState {
    tab_hitboxes: Vec<TabHitbox>,
    line_rows: Vec<Option<RenderedLine>>,
}

#[derive(Debug)]
enum InputParserState {
    Ground,
    Esc,
    Csi(Vec<u8>),
}

#[derive(Debug)]
struct InputParser {
    state: InputParserState,
}

impl InputParser {
    fn new() -> Self {
        Self {
            state: InputParserState::Ground,
        }
    }

    fn feed(&mut self, byte: u8) -> Option<UiMessage> {
        match &mut self.state {
            InputParserState::Ground => {
                if byte == 0x1b {
                    self.state = InputParserState::Esc;
                    return None;
                }

                key_message_from_byte(byte)
            }
            InputParserState::Esc => {
                if byte == b'[' {
                    self.state = InputParserState::Csi(Vec::new());
                } else {
                    self.state = InputParserState::Ground;
                }
                None
            }
            InputParserState::Csi(buf) => {
                buf.push(byte);
                if !(0x40..=0x7e).contains(&byte) {
                    return None;
                }

                let message = try_parse_sgr_mouse_message(buf);
                self.state = InputParserState::Ground;
                message
            }
        }
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter(stdout: &mut Stdout) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, Show, DisableMouseCapture, LeaveAlternateScreen);
    }
}

fn spawn_input_reader(tx: SyncSender<InputMessage>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut locked = stdin.lock();
        let mut buf = String::new();

        loop {
            buf.clear();
            match locked.read_line(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(InputMessage::Closed);
                    break;
                }
                Ok(_) => {
                    if buf.ends_with('\n') {
                        buf.pop();
                        if buf.ends_with('\r') {
                            buf.pop();
                        }
                    }

                    if tx.send(InputMessage::Line(buf.clone())).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(InputMessage::Error(err.to_string()));
                    break;
                }
            }
        }
    });
}

fn spawn_ui_reader(tx: SyncSender<UiMessage>) -> io::Result<()> {
    let mut tty = OpenOptions::new().read(true).open("/dev/tty")?;

    thread::spawn(move || {
        let mut parser = InputParser::new();
        let mut buf = [0u8; 64];

        loop {
            match tty.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(UiMessage::Quit);
                    break;
                }
                Ok(n) => {
                    for byte in &buf[..n] {
                        if let Some(message) = parser.feed(*byte)
                            && tx.send(message).is_err()
                        {
                            return;
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    let _ = tx.send(UiMessage::Error(err.to_string()));
                    break;
                }
            }
        }
    });

    Ok(())
}

fn key_message_from_byte(byte: u8) -> Option<UiMessage> {
    match byte {
        b'\t' => Some(UiMessage::NextTab),
        b'1'..=b'9' => Some(UiMessage::SelectTab((byte - b'0') as usize)),
        b'0' => Some(UiMessage::SelectTab(0)),
        b' ' => Some(UiMessage::TogglePause),
        b'd' | b'D' => Some(UiMessage::ClearSelection),
        b's' | b'S' => Some(UiMessage::SelectMiddleVisibleLine),
        b'q' | b'Q' | 0x03 => Some(UiMessage::Quit),
        _ => None,
    }
}

fn try_parse_sgr_mouse_message(sequence: &[u8]) -> Option<UiMessage> {
    let (final_byte, params) = sequence.split_last()?;
    if *final_byte != b'M' || !params.starts_with(b"<") {
        return None;
    }

    let payload = std::str::from_utf8(&params[1..]).ok()?;
    let mut parts = payload.split(';');
    let cb = parts.next()?.parse::<u16>().ok()?;
    let col = parts.next()?.parse::<u16>().ok()?;
    let row = parts.next()?.parse::<u16>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let is_left_button = (cb & 0b11) == 0;
    let is_motion = (cb & 0b0010_0000) != 0;
    let is_wheel = (cb & 0b0100_0000) != 0;
    if is_left_button && !is_motion && !is_wheel {
        return Some(UiMessage::MouseLeftDown {
            column: col.saturating_sub(1),
            row: row.saturating_sub(1),
        });
    }

    None
}

#[cfg(unix)]
fn terminate_pipeline_group_if_safe() {
    // In interactive shells with job control, pipeline commands are in a separate
    // process group from the shell. Signaling that group lets `q` stop upstream
    // producers like `tail -f` immediately.
    unsafe {
        let my_pgid = libc::getpgrp();
        if my_pgid <= 0 {
            return;
        }

        let parent_pgid = libc::getpgid(libc::getppid());
        if parent_pgid == my_pgid {
            return;
        }

        let _ = libc::signal(libc::SIGINT, libc::SIG_IGN);
        let _ = libc::killpg(my_pgid, libc::SIGINT);
    }
}

#[cfg(not(unix))]
fn terminate_pipeline_group_if_safe() {}

fn mark_tab_seen_live(tabs: &mut [Tab], index: usize) {
    if let Some(tab) = tabs.get_mut(index) {
        tab.mark_seen_through(tab.total_matches);
    }
}

fn mark_tab_seen_paused(tabs: &mut [Tab], index: usize, pause_match_cutoffs: &[u64]) {
    if let Some(tab) = tabs.get_mut(index) {
        let cutoff = pause_match_cutoffs
            .get(index)
            .copied()
            .unwrap_or(tab.total_matches);
        tab.mark_seen_through(cutoff);
    }
}

fn select_tab(
    tabs: &mut [Tab],
    active_index: &mut usize,
    next_index: usize,
    paused: bool,
    pause_snapshot: Option<&PauseSnapshot>,
) {
    if next_index >= tabs.len() {
        return;
    }

    *active_index = next_index;
    if paused {
        if let Some(snapshot) = pause_snapshot {
            mark_tab_seen_paused(tabs, *active_index, &snapshot.match_cutoffs);
        }
    } else {
        mark_tab_seen_live(tabs, *active_index);
    }
}

fn apply_line_to_tabs(tabs: &mut [Tab], active_index: usize, paused: bool, seq: u64, line: &str) {
    for (index, tab) in tabs.iter_mut().enumerate() {
        if tab.matches(line) {
            tab.push_line(seq, line);
            if index == active_index && !paused {
                tab.mark_seen_through(tab.total_matches);
            }
        }
    }
}

fn clip_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    text.chars().take(width).collect()
}

fn is_ansi_final_byte(ch: char) -> bool {
    ('@'..='~').contains(&ch)
}

fn clip_ansi_to_visible_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut visible = 0usize;
    let mut chars = text.chars().peekable();
    let mut saw_ansi = false;
    let mut clipped = false;

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            saw_ansi = true;
            out.push(ch);

            if let Some(next) = chars.next() {
                out.push(next);
                if next == '[' {
                    for seq_char in chars.by_ref() {
                        out.push(seq_char);
                        if is_ansi_final_byte(seq_char) {
                            break;
                        }
                    }
                }
            }
            continue;
        }

        if visible >= width {
            clipped = true;
            break;
        }

        out.push(ch);
        visible += 1;
    }

    if clipped && saw_ansi {
        out.push_str("\u{1b}[0m");
    }

    out
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if let Some(next) = chars.next() && next == '[' {
                for seq_char in chars.by_ref() {
                    if is_ansi_final_byte(seq_char) {
                        break;
                    }
                }
            }
            continue;
        }

        out.push(ch);
    }

    out
}

fn clip_with_ellipsis(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let char_count = text.chars().count();
    if char_count <= width {
        return text.to_owned();
    }

    if width <= 3 {
        return ".".repeat(width);
    }

    let mut out = text.chars().take(width - 3).collect::<String>();
    out.push_str("...");
    out
}

fn fit_tab_title(label: &str, width: usize) -> String {
    match width {
        0 => String::new(),
        1 => " ".to_owned(),
        2 => "  ".to_owned(),
        _ => {
            let clipped = clip_with_ellipsis(label, width - 2);
            let mut piece = format!(" {} ", clipped);
            let count = piece.chars().count();
            if count < width {
                piece.push_str(&" ".repeat(width - count));
            } else if count > width {
                piece = clip_to_width(&piece, width);
            }
            piece
        }
    }
}

fn format_unread_slot(unread: u64) -> String {
    if unread == 0 {
        return "      ".to_owned();
    }

    let badge = if unread > 999 {
        "•999+".to_owned()
    } else {
        format!("•{}", unread)
    };

    format!("{:>6}", badge)
}

fn first_body_row(body_start_row: usize, body_height: usize, visible_count: usize) -> usize {
    body_start_row + body_height.saturating_sub(visible_count)
}

fn tab_shortcut_label(index: usize) -> String {
    if index == 0 {
        "0".to_owned()
    } else {
        index.to_string()
    }
}

fn tab_columns_limit(total_cols: usize, paused: bool) -> usize {
    if paused {
        total_cols.saturating_sub(PAUSED_LABEL.chars().count())
    } else {
        total_cols
    }
}

fn draw_piece_clipped(
    stdout: &mut Stdout,
    x: &mut u16,
    y: u16,
    remaining: &mut usize,
    text: &str,
    color: Option<Color>,
) -> io::Result<()> {
    if *remaining == 0 {
        return Ok(());
    }

    let shown = clip_to_width(text, *remaining);
    if shown.is_empty() {
        return Ok(());
    }

    let width = shown.chars().count();
    queue!(stdout, MoveTo(*x, y))?;
    if let Some(color) = color {
        queue!(stdout, SetForegroundColor(color), Print(&shown), ResetColor)?;
    } else {
        queue!(stdout, Print(&shown))?;
    }

    *x = x.saturating_add(width as u16);
    *remaining = remaining.saturating_sub(width);
    Ok(())
}

fn prepare_visible_lines(
    tab: &Tab,
    cutoff_len: usize,
    selected_line: Option<&SelectedLine>,
) -> Vec<RenderedLine> {
    let mut lines = tab
        .lines
        .iter()
        .take(cutoff_len)
        .map(|line| RenderedLine {
            seq: line.seq,
            text: line.text.clone(),
            selected: false,
        })
        .collect::<Vec<_>>();

    if let Some(selected) = selected_line {
        if let Some(existing) = lines.iter_mut().find(|line| line.seq == selected.seq) {
            existing.selected = true;
        } else {
            let insert_at = lines
                .iter()
                .position(|line| line.seq > selected.seq)
                .unwrap_or(lines.len());
            lines.insert(
                insert_at,
                RenderedLine {
                    seq: selected.seq,
                    text: selected.text.clone(),
                    selected: true,
                },
            );
        }
    }

    lines
}

fn viewport_for_lines(
    body_start_row: usize,
    body_height: usize,
    lines: &[RenderedLine],
    paused: bool,
) -> (usize, usize, usize) {
    let visible_count = lines.len().min(body_height);
    if visible_count == 0 {
        return (0, 0, body_start_row);
    }

    if paused && let Some(selected_index) = lines.iter().position(|line| line.selected) {
        let half = body_height / 2;
        let mut start_index = selected_index.saturating_sub(half);
        let max_start = lines.len().saturating_sub(visible_count);
        if start_index > max_start {
            start_index = max_start;
        }

        let selected_row = selected_index.saturating_sub(start_index);
        let desired_selected_row = body_height / 2;
        let min_first_row = body_start_row;
        let max_first_row = body_start_row + body_height.saturating_sub(visible_count);
        let mut first_row = body_start_row + desired_selected_row.saturating_sub(selected_row);
        if first_row < min_first_row {
            first_row = min_first_row;
        }
        if first_row > max_first_row {
            first_row = max_first_row;
        }

        return (start_index, visible_count, first_row);
    }

    let start_index = lines.len().saturating_sub(visible_count);
    let first_row = first_body_row(body_start_row, body_height, visible_count);
    (start_index, visible_count, first_row)
}

fn tab_index_at_position(render_state: &RenderState, column: u16, row: u16) -> Option<usize> {
    if row > 2 {
        return None;
    }

    render_state
        .tab_hitboxes
        .iter()
        .find(|hitbox| column >= hitbox.left && column <= hitbox.right)
        .map(|hitbox| hitbox.index)
}

fn line_at_row(render_state: &RenderState, row: u16) -> Option<&RenderedLine> {
    render_state
        .line_rows
        .get(row as usize)
        .and_then(|line| line.as_ref())
}

fn toggle_selected_line(selected_line: &mut Option<SelectedLine>, line: &RenderedLine) {
    if selected_line.as_ref().map(|current| current.seq) == Some(line.seq) {
        *selected_line = None;
    } else {
        *selected_line = Some(SelectedLine {
            seq: line.seq,
            text: line.text.clone(),
        });
    }
}

fn middle_visible_line(render_state: &RenderState) -> Option<&RenderedLine> {
    let visible_lines = render_state
        .line_rows
        .iter()
        .filter_map(|line| line.as_ref())
        .collect::<Vec<_>>();
    if visible_lines.is_empty() {
        return None;
    }

    visible_lines.get(visible_lines.len() / 2).copied()
}

fn draw(
    stdout: &mut Stdout,
    tabs: &[Tab],
    active_index: usize,
    paused: bool,
    pause_line_cutoffs: Option<&[usize]>,
    selected_line: Option<&SelectedLine>,
) -> io::Result<RenderState> {
    let (cols, rows) = terminal::size()?;
    let cols_usize = cols as usize;
    let rows_usize = rows as usize;

    let mut render_state = RenderState {
        tab_hitboxes: Vec::new(),
        line_rows: vec![None; rows_usize],
    };

    queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

    if rows_usize == 0 || cols_usize == 0 {
        stdout.flush()?;
        return Ok(render_state);
    }

    let tab_cols_limit = tab_columns_limit(cols_usize, paused);

    let mut x = 0u16;
    let mut tabs_right: u16 = 0;
    for (i, tab) in tabs.iter().enumerate() {
        if x as usize >= tab_cols_limit {
            break;
        }

        let number_piece = format!(" {} ", tab_shortcut_label(i));
        let unread_piece = format_unread_slot(tab.unread_matches());
        let trailing_piece = " ";

        let fixed_inner_width = number_piece.chars().count()
            + unread_piece.chars().count()
            + trailing_piece.chars().count();
        let full_title_width = tab.label.chars().count() + 2;
        let desired_inner_width = fixed_inner_width + full_title_width;

        let remaining_cols = tab_cols_limit.saturating_sub(x as usize);
        if remaining_cols < 3 {
            break;
        }

        let inner_width = desired_inner_width.min(remaining_cols.saturating_sub(2));
        if inner_width == 0 {
            break;
        }

        let title_budget = inner_width.saturating_sub(fixed_inner_width);
        let title_piece = fit_tab_title(&tab.label, title_budget);

        let right = x + inner_width as u16 + 1;
        let border_color = if i == active_index {
            Color::White
        } else {
            Color::DarkGrey
        };
        let horiz = "─".repeat(inner_width);

        if rows_usize >= 1 {
            queue!(
                stdout,
                MoveTo(x, 0),
                SetForegroundColor(border_color),
                Print("╭"),
                Print(&horiz),
                Print("╮"),
                ResetColor
            )?;
        }

        if rows_usize >= 2 {
            queue!(
                stdout,
                MoveTo(x, 1),
                SetForegroundColor(border_color),
                Print("│"),
                ResetColor
            )?;

            let mut inner_x = x + 1;
            let mut remaining_inner = inner_width;
            draw_piece_clipped(
                stdout,
                &mut inner_x,
                1,
                &mut remaining_inner,
                &number_piece,
                Some(Color::DarkGrey),
            )?;
            let title_color = if matches!(tab.mode, MatchMode::All) {
                Some(Color::DarkGrey)
            } else {
                None
            };
            draw_piece_clipped(
                stdout,
                &mut inner_x,
                1,
                &mut remaining_inner,
                &title_piece,
                title_color,
            )?;
            draw_piece_clipped(
                stdout,
                &mut inner_x,
                1,
                &mut remaining_inner,
                &unread_piece,
                Some(Color::DarkCyan),
            )?;
            draw_piece_clipped(
                stdout,
                &mut inner_x,
                1,
                &mut remaining_inner,
                trailing_piece,
                None,
            )?;
            if remaining_inner > 0 {
                let pad = " ".repeat(remaining_inner);
                queue!(stdout, MoveTo(inner_x, 1), Print(pad))?;
            }

            queue!(
                stdout,
                MoveTo(right, 1),
                SetForegroundColor(border_color),
                Print("│"),
                ResetColor
            )?;
        }

        if rows_usize >= 3 {
            queue!(
                stdout,
                MoveTo(x, 2),
                SetForegroundColor(border_color),
                Print("╰"),
                Print(&horiz),
                Print("╯"),
                ResetColor
            )?;
        }

        render_state.tab_hitboxes.push(TabHitbox {
            index: i,
            left: x,
            right,
        });
        tabs_right = right;
        x = right.saturating_add(1);
        if i + 1 < tabs.len() && (x as usize) < tab_cols_limit {
            x = x.saturating_add(1);
        }
    }

    if paused {
        let start_col = if tabs_right > 0 {
            tabs_right.saturating_add(1)
        } else {
            0
        };
        if (start_col as usize) < cols_usize {
            let available = cols_usize - start_col as usize;
            let shown = clip_to_width(PAUSED_LABEL, available);
            if !shown.is_empty() {
                let paused_row = if rows_usize >= 2 { 1 } else { 0 };
                queue!(
                    stdout,
                    MoveTo(start_col, paused_row as u16),
                    SetForegroundColor(Color::Grey),
                    Print(shown),
                    ResetColor
                )?;
            }
        }
    }

    let body_start_row = if rows_usize >= 3 { 3usize } else { 2usize };
    if rows_usize <= body_start_row {
        stdout.flush()?;
        return Ok(render_state);
    }

    let body_height = rows_usize - body_start_row;
    let active_tab = &tabs[active_index];
    let cutoff_len = pause_line_cutoffs
        .and_then(|cutoffs| cutoffs.get(active_index).copied())
        .unwrap_or(active_tab.lines.len())
        .min(active_tab.lines.len());

    let visible_lines = prepare_visible_lines(active_tab, cutoff_len, selected_line);
    let (start_index, visible_count, first_row) =
        viewport_for_lines(body_start_row, body_height, &visible_lines, paused);

    for (screen_row, line) in visible_lines
        .iter()
        .skip(start_index)
        .take(visible_count)
        .enumerate()
    {
        let y = (first_row + screen_row) as u16;
        if line.selected {
            let plain = strip_ansi(&line.text);
            let clipped = clip_to_width(&plain, cols_usize);
            queue!(
                stdout,
                MoveTo(0, y),
                SetForegroundColor(Color::Yellow),
                Print(clipped),
                ResetColor
            )?;
        } else {
            let clipped = clip_ansi_to_visible_width(&line.text, cols_usize);
            queue!(stdout, MoveTo(0, y), Print(clipped))?;
        }

        if let Some(slot) = render_state.line_rows.get_mut(y as usize) {
            *slot = Some(line.clone());
        }
    }

    stdout.flush()?;
    Ok(render_state)
}

fn print_usage(binary: &str) {
    eprintln!(
        "Usage: {} <filter1> <filter2> ...\n\nExample:\n  tail -f app.log | {} error warn info",
        binary, binary
    );
}

fn run() -> io::Result<()> {
    if !io::stdout().is_terminal() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "stdout must be a TTY (run this in a terminal, not redirected)",
        ));
    }

    let binary = std::env::args()
        .next()
        .unwrap_or_else(|| "streamtabs".to_owned());
    let mut filters = std::env::args()
        .skip(1)
        .filter(|f| !f.is_empty())
        .collect::<Vec<_>>();

    if filters.is_empty() {
        print_usage(&binary);
        std::process::exit(2);
    }

    let mut tabs = Vec::with_capacity(filters.len() + 1);
    tabs.push(Tab::unfiltered());
    tabs.extend(filters.drain(..).map(Tab::new));
    let mut active_index = 0usize;
    let mut next_seq = 0u64;
    let mut selected_line: Option<SelectedLine> = None;

    let (tx, rx): (SyncSender<InputMessage>, Receiver<InputMessage>) = mpsc::sync_channel(1024);
    spawn_input_reader(tx);
    let (ui_tx, ui_rx): (SyncSender<UiMessage>, Receiver<UiMessage>) = mpsc::sync_channel(128);
    spawn_ui_reader(ui_tx)?;

    let mut stdout = io::stdout();
    {
        let _guard = TerminalGuard::enter(&mut stdout)?;

        let mut dirty = true;
        let mut paused = false;
        let mut pause_snapshot: Option<PauseSnapshot> = None;
        let mut last_size = terminal::size().unwrap_or((0, 0));
        let mut last_render_state = RenderState::default();

        'app: loop {
            while let Ok(message) = rx.try_recv() {
                match message {
                    InputMessage::Line(line) => {
                        apply_line_to_tabs(&mut tabs, active_index, paused, next_seq, &line);
                        next_seq = next_seq.saturating_add(1);
                        if !paused {
                            dirty = true;
                        }
                    }
                    InputMessage::Closed => {}
                    InputMessage::Error(err) => return Err(io::Error::other(err)),
                }
            }

            while let Ok(message) = ui_rx.try_recv() {
                match message {
                    UiMessage::NextTab => {
                        let next_index = (active_index + 1) % tabs.len();
                        select_tab(
                            &mut tabs,
                            &mut active_index,
                            next_index,
                            paused,
                            pause_snapshot.as_ref(),
                        );
                        dirty = true;
                    }
                    UiMessage::SelectTab(tab_index) => {
                        if tab_index < tabs.len() {
                            select_tab(
                                &mut tabs,
                                &mut active_index,
                                tab_index,
                                paused,
                                pause_snapshot.as_ref(),
                            );
                            dirty = true;
                        }
                    }
                    UiMessage::TogglePause => {
                        paused = !paused;
                        if paused {
                            pause_snapshot = Some(PauseSnapshot {
                                line_cutoffs: tabs.iter().map(|tab| tab.lines.len()).collect(),
                                match_cutoffs: tabs.iter().map(|tab| tab.total_matches).collect(),
                            });
                            if let Some(snapshot) = pause_snapshot.as_ref() {
                                mark_tab_seen_paused(
                                    &mut tabs,
                                    active_index,
                                    &snapshot.match_cutoffs,
                                );
                            }
                        } else {
                            pause_snapshot = None;
                            mark_tab_seen_live(&mut tabs, active_index);
                        }
                        dirty = true;
                    }
                    UiMessage::ClearSelection => {
                        if selected_line.take().is_some() {
                            dirty = true;
                        }
                    }
                    UiMessage::SelectMiddleVisibleLine => {
                        if let Some(line) = middle_visible_line(&last_render_state) {
                            toggle_selected_line(&mut selected_line, line);
                            dirty = true;
                        }
                    }
                    UiMessage::MouseLeftDown { column, row } => {
                        if let Some(tab_index) =
                            tab_index_at_position(&last_render_state, column, row)
                        {
                            select_tab(
                                &mut tabs,
                                &mut active_index,
                                tab_index,
                                paused,
                                pause_snapshot.as_ref(),
                            );
                            dirty = true;
                            continue;
                        }

                        if let Some(line) = line_at_row(&last_render_state, row) {
                            toggle_selected_line(&mut selected_line, line);
                            dirty = true;
                        }
                    }
                    UiMessage::Quit => {
                        break 'app;
                    }
                    UiMessage::Error(err) => return Err(io::Error::other(err)),
                }
            }

            if let Ok(current_size) = terminal::size()
                && current_size != last_size
            {
                last_size = current_size;
                dirty = true;
            }

            if dirty {
                last_render_state = draw(
                    &mut stdout,
                    &tabs,
                    active_index,
                    paused,
                    pause_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.line_cutoffs.as_slice()),
                    selected_line.as_ref(),
                )?;
                dirty = false;
            }

            thread::sleep(POLL_INTERVAL);
        }
    }

    terminate_pipeline_group_if_safe();
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("streamtabs failed: {}", err);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RenderedLine, SelectedLine, Tab, UiMessage, apply_line_to_tabs, clip_to_width,
        clip_with_ellipsis, clip_ansi_to_visible_width, fit_tab_title, key_message_from_byte,
        mark_tab_seen_live, mark_tab_seen_paused, middle_visible_line, prepare_visible_lines,
        strip_ansi, toggle_selected_line,
        try_parse_sgr_mouse_message,
        viewport_for_lines,
    };

    #[test]
    fn filters_are_applied_independently() {
        let mut tabs = vec![Tab::new("foo".into()), Tab::new("bar".into())];

        apply_line_to_tabs(&mut tabs, 0, false, 0, "foo only");
        apply_line_to_tabs(&mut tabs, 0, false, 1, "bar only");
        apply_line_to_tabs(&mut tabs, 0, false, 2, "foo and bar");

        assert_eq!(tabs[0].total_matches, 2);
        assert_eq!(tabs[1].total_matches, 2);
        assert_eq!(
            tabs[0].lines.back().map(|line| line.text.as_str()),
            Some("foo and bar")
        );
        assert_eq!(
            tabs[1].lines.back().map(|line| line.text.as_str()),
            Some("foo and bar")
        );
        assert_eq!(tabs[1].unread_matches(), 2);
        assert_eq!(tabs[0].unread_matches(), 0);
    }

    #[test]
    fn all_tab_matches_every_line() {
        let all = Tab::unfiltered();
        assert!(all.matches("anything"));
        assert!(all.matches(""));
    }

    #[test]
    fn unread_count_clears_when_tab_is_seen() {
        let mut tabs = vec![Tab::new("foo".into()), Tab::new("bar".into())];

        apply_line_to_tabs(&mut tabs, 0, false, 0, "foo and bar");
        apply_line_to_tabs(&mut tabs, 0, false, 1, "bar only");
        assert_eq!(tabs[1].unread_matches(), 2);

        mark_tab_seen_live(&mut tabs, 1);
        assert_eq!(tabs[1].unread_matches(), 0);
    }

    #[test]
    fn paused_switch_keeps_post_pause_unread() {
        let mut tabs = vec![Tab::new("foo".into()), Tab::new("bar".into())];

        apply_line_to_tabs(&mut tabs, 0, false, 0, "bar before pause");
        let pause_match_cutoffs = tabs.iter().map(|tab| tab.total_matches).collect::<Vec<_>>();

        apply_line_to_tabs(&mut tabs, 0, true, 1, "bar after pause");
        assert_eq!(tabs[1].unread_matches(), 2);

        mark_tab_seen_paused(&mut tabs, 1, &pause_match_cutoffs);
        assert_eq!(tabs[1].unread_matches(), 1);
    }

    #[test]
    fn active_tab_accumulates_unread_while_paused() {
        let mut tabs = vec![Tab::new("foo".into()), Tab::new("bar".into())];

        apply_line_to_tabs(&mut tabs, 0, false, 0, "foo visible");
        assert_eq!(tabs[0].unread_matches(), 0);

        apply_line_to_tabs(&mut tabs, 0, true, 1, "foo hidden while paused");
        assert_eq!(tabs[0].unread_matches(), 1);
    }

    #[test]
    fn clip_limits_char_count() {
        assert_eq!(clip_to_width("abcdef", 0), "");
        assert_eq!(clip_to_width("abcdef", 3), "abc");
        assert_eq!(clip_to_width("abc", 10), "abc");
    }

    #[test]
    fn ansi_clip_uses_visible_width() {
        let text = "\u{1b}[2m2026-02-06\u{1b}[0m INFO module message";
        let clipped = clip_ansi_to_visible_width(text, 10);
        assert_eq!(clipped.replace("\u{1b}[2m", "").replace("\u{1b}[0m", ""), "2026-02-06");
    }

    #[test]
    fn ansi_clip_resets_if_cut_mid_styled_content() {
        let text = "\u{1b}[31mERROR something happened\u{1b}[0m";
        let clipped = clip_ansi_to_visible_width(text, 5);
        assert!(clipped.ends_with("\u{1b}[0m"));
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        let text = "\u{1b}[2m2026-02-06\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m line";
        assert_eq!(strip_ansi(text), "2026-02-06 ERROR line");
    }

    #[test]
    fn clip_with_ellipsis_marks_truncation() {
        assert_eq!(clip_with_ellipsis("abcdef", 6), "abcdef");
        assert_eq!(clip_with_ellipsis("abcdef", 5), "ab...");
        assert_eq!(clip_with_ellipsis("abcdef", 3), "...");
    }

    #[test]
    fn tab_title_fits_budget() {
        assert_eq!(fit_tab_title("hello", 8), " hello  ");
        assert_eq!(fit_tab_title("very-long-label", 8), " ver... ");
        assert_eq!(fit_tab_title("ignored", 2), "  ");
    }

    #[test]
    fn body_is_bottom_anchored_when_not_full() {
        assert_eq!(super::first_body_row(3, 10, 1), 12);
        assert_eq!(super::first_body_row(3, 10, 10), 3);
    }

    #[test]
    fn unread_slot_is_fixed_width_and_caps() {
        assert_eq!(super::format_unread_slot(0), "      ");
        assert_eq!(super::format_unread_slot(7), "    •7");
        assert_eq!(super::format_unread_slot(999), "  •999");
        assert_eq!(super::format_unread_slot(1000), " •999+");
    }

    #[test]
    fn key_mapping_handles_supported_keys() {
        assert!(matches!(
            key_message_from_byte(b'\t'),
            Some(UiMessage::NextTab)
        ));
        assert!(matches!(
            key_message_from_byte(b'5'),
            Some(UiMessage::SelectTab(5))
        ));
        assert!(matches!(
            key_message_from_byte(b'0'),
            Some(UiMessage::SelectTab(0))
        ));
        assert!(matches!(
            key_message_from_byte(b' '),
            Some(UiMessage::TogglePause)
        ));
        assert!(matches!(
            key_message_from_byte(b'd'),
            Some(UiMessage::ClearSelection)
        ));
        assert!(matches!(
            key_message_from_byte(b'D'),
            Some(UiMessage::ClearSelection)
        ));
        assert!(matches!(
            key_message_from_byte(b's'),
            Some(UiMessage::SelectMiddleVisibleLine)
        ));
        assert!(matches!(
            key_message_from_byte(b'S'),
            Some(UiMessage::SelectMiddleVisibleLine)
        ));
        assert!(matches!(key_message_from_byte(b'q'), Some(UiMessage::Quit)));
        assert!(matches!(key_message_from_byte(0x03), Some(UiMessage::Quit)));
        assert!(key_message_from_byte(b'\n').is_none());
    }

    #[test]
    fn sgr_mouse_parser_decodes_left_click() {
        assert!(matches!(
            try_parse_sgr_mouse_message(b"<0;12;7M"),
            Some(UiMessage::MouseLeftDown { column: 11, row: 6 })
        ));
        assert!(try_parse_sgr_mouse_message(b"<35;12;7M").is_none());
        assert!(try_parse_sgr_mouse_message(b"<64;12;7M").is_none());
    }

    #[test]
    fn selected_line_is_injected_into_non_matching_tabs() {
        let mut tab = Tab::new("foo".into());
        tab.push_line(1, "foo first");
        tab.push_line(3, "foo second");
        let selected = SelectedLine {
            seq: 2,
            text: "picked elsewhere".to_owned(),
        };

        let visible = prepare_visible_lines(&tab, tab.lines.len(), Some(&selected));
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].seq, 1);
        assert_eq!(visible[1].seq, 2);
        assert_eq!(visible[1].text, "picked elsewhere");
        assert!(visible[1].selected);
        assert_eq!(visible[2].seq, 3);
    }

    #[test]
    fn paused_viewport_centers_selected_line() {
        let lines = (0..20)
            .map(|idx| RenderedLine {
                seq: idx as u64,
                text: idx.to_string(),
                selected: idx == 10,
            })
            .collect::<Vec<_>>();
        let (start, count, first_row) = viewport_for_lines(3, 10, &lines, true);
        assert_eq!(start, 5);
        assert_eq!(count, 10);
        assert_eq!(first_row, 3);
    }

    #[test]
    fn clicking_selected_line_toggles_selection_off() {
        let clicked = RenderedLine {
            seq: 42,
            text: "selected".to_owned(),
            selected: false,
        };
        let mut selected = Some(SelectedLine {
            seq: 42,
            text: "selected".to_owned(),
        });

        toggle_selected_line(&mut selected, &clicked);
        assert!(selected.is_none());

        toggle_selected_line(&mut selected, &clicked);
        assert_eq!(selected.as_ref().map(|line| line.seq), Some(42));
    }

    #[test]
    fn middle_visible_line_picks_middle_rendered_row() {
        let mut render_state = super::RenderState {
            tab_hitboxes: Vec::new(),
            line_rows: vec![None; 8],
        };
        render_state.line_rows[2] = Some(RenderedLine {
            seq: 10,
            text: "a".to_owned(),
            selected: false,
        });
        render_state.line_rows[3] = Some(RenderedLine {
            seq: 20,
            text: "b".to_owned(),
            selected: false,
        });
        render_state.line_rows[4] = Some(RenderedLine {
            seq: 30,
            text: "c".to_owned(),
            selected: false,
        });

        let picked = middle_visible_line(&render_state).expect("middle line should exist");
        assert_eq!(picked.seq, 20);
    }
}
