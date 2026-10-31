//! Client-local, provider-neutral Todo feed. No runtime protocol changes.
use super::render::put_text;
use super::*;
use serde::Deserialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, Once, OnceLock,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    source: String,
    title: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    kind: EntryKind,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    comment: Option<Comment>,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    visible_from: Option<u64>,
    #[serde(default)]
    expires_at: Option<u64>,
    /// Executable followed by literal arguments; never interpreted by a shell.
    #[serde(default)]
    action: Vec<String>,
}
/// Providers publish the relationship/event, rather than choosing presentation.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EntryKind {
    Meeting,
    MergeRequest,
    ReviewRequested,
    AssignedComments,
    AssignedApproved,
    CommentReply,
    ChangesRequested,
    Merged,
    CiFailed,
    Message,
    #[default]
    #[serde(other)]
    Generic,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Comment {
    #[serde(default)]
    author: String,
    text: String,
}

impl EntryKind {
    // Octicons from Nerd Fonts' glyphnames.json (git_pull_request, discussion,
    // check_circle, reply, git_merge, calendar, bell, comment).
    fn presentation(
        self,
        palette: &Palette,
    ) -> (&'static str, &'static str, ratatui::style::Color) {
        match self {
            Self::Meeting => ("\u{f455}", "Upcoming meeting", palette.mauve),
            Self::MergeRequest => ("\u{f407}", "Open MR", palette.blue),
            Self::ReviewRequested => ("\u{f4af}", "Review requested", palette.blue),
            Self::AssignedComments => ("\u{f442}", "Your MR · New threads", palette.peach),
            Self::AssignedApproved => ("\u{f49e}", "Your MR · Approved", palette.green),
            Self::CommentReply => ("\u{f4a8}", "Reply to your comment", palette.teal),
            Self::ChangesRequested => ("\u{f41f}", "Changes requested", palette.red),
            Self::Merged => ("\u{f419}", "Merged", palette.mauve),
            Self::CiFailed => ("\u{f52f}", "Pipeline failed", palette.red),
            Self::Message => ("\u{f41f}", "Unread message", palette.blue),
            Self::Generic => ("\u{f49a}", "", palette.accent),
        }
    }
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Two display-width-aware lines; the second ends with an ellipsis if needed.
fn preview(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    if width == 0 || text.is_empty() {
        return Vec::new();
    }
    let text = clean(text);
    if UnicodeWidthStr::width(text.as_str()) <= width {
        return vec![text];
    }
    let mut used = 0;
    let mut split = text.len();
    for (index, ch) in text.char_indices() {
        used += ch.width().unwrap_or(0);
        if used > width {
            split = index;
            break;
        }
    }
    if let Some(space) = text[..split].rfind(' ').filter(|space| *space > 0) {
        split = space;
    }
    vec![
        text[..split].to_owned(),
        crate::ui::truncate_end(text[split..].trim_start(), width),
    ]
}

fn entry_lines(entry: &Entry, width: usize, palette: &Palette) -> Vec<(String, Style)> {
    let (default_icon, label, color) = entry.kind.presentation(palette);
    let icon = entry.icon.as_deref().unwrap_or(default_icon);
    let mut lines = vec![(
        crate::ui::truncate_end(&format!(" {} {}", icon, clean(&entry.title)), width),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )];
    let metadata = if label.is_empty() {
        clean(&entry.detail)
    } else if entry.detail.is_empty() {
        label.to_owned()
    } else {
        format!("{label} · {}", clean(&entry.detail))
    };
    if !metadata.is_empty() {
        lines.push((
            crate::ui::truncate_end(&format!("   {metadata}"), width),
            Style::default().fg(palette.overlay0),
        ));
    }
    if let Some(comment) = &entry.comment {
        let text = if comment.author.is_empty() {
            clean(&comment.text)
        } else {
            format!("{}: {}", clean(&comment.author), clean(&comment.text))
        };
        for text in preview(&text, width.saturating_sub(3)) {
            lines.push((format!("   {text}"), Style::default().fg(palette.subtext0)));
        }
    }
    lines
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Feed {
    entries: Vec<Entry>,
}
#[derive(Default)]
struct Cache {
    entries: Vec<Entry>,
    error: Option<String>,
}
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static START: Once = Once::new();
static DIRTY: AtomicBool = AtomicBool::new(false);
fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn visible(entry: &Entry, time: u64) -> bool {
    entry.visible_from.is_none_or(|start| time >= start)
        && entry.expires_at.is_none_or(|end| time < end)
}
fn parse(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let feed: Feed = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let mut ids = HashSet::new();
    for entry in &feed.entries {
        if entry.id.is_empty()
            || entry.source.is_empty()
            || entry.title.is_empty()
            || !ids.insert((&entry.source, &entry.id))
        {
            return Err(
                "entries need nonempty source, id and title, and unique (source, id)".into(),
            );
        }
    }
    let mut entries = feed.entries;
    entries.sort_by_key(|e| std::cmp::Reverse(e.priority));
    Ok(entries)
}
pub(super) fn start() {
    START.call_once(|| {
        let path = std::env::var_os("HERDR_TODO_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| crate::config::config_dir().join("todo.json"));
        if let Err(error) = crate::thread_spawn::spawn_named("todo-feed", move || {
            let mut previous = None;
            let mut previous_visible = Vec::new();
            loop {
                // Bound input size; providers should publish with an atomic rename.
                use std::io::Read;
                let result = std::fs::File::open(&path).and_then(|file| {
                    let mut bytes = Vec::new();
                    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                    Ok(bytes)
                });
                let result = match result {
                    Ok(bytes) if bytes.len() > 1024 * 1024 => {
                        Err("Todo feed exceeds 1 MiB".to_owned())
                    }
                    Ok(bytes) => Ok(bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        Ok(b"{\"entries\":[]}".to_vec())
                    }
                    Err(e) => Err(e.to_string()),
                };
                let mut state = cache().lock().unwrap_or_else(|e| e.into_inner());
                if previous.as_ref() != Some(&result) {
                    match &result {
                        Ok(bytes) => match parse(bytes) {
                            Ok(entries) => {
                                state.entries = entries;
                                state.error = None;
                            }
                            Err(e) => {
                                state.entries.clear();
                                tracing::warn!(error = %e, "invalid Todo feed");
                                state.error = Some(e);
                            }
                        },
                        Err(e) => {
                            state.entries.clear();
                            tracing::warn!(error = %e, "cannot read Todo feed");
                            state.error = Some(e.clone());
                        }
                    }
                    previous = Some(result);
                    DIRTY.store(true, Ordering::Release);
                }
                let visible_ids = state
                    .entries
                    .iter()
                    .filter(|e| visible(e, now()))
                    .map(|e| (e.source.clone(), e.id.clone()))
                    .collect::<Vec<_>>();
                if previous_visible != visible_ids {
                    previous_visible = visible_ids;
                    DIRTY.store(true, Ordering::Release);
                }
                drop(state);
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }) {
            tracing::warn!(%error, "could not start Todo feed");
        }
    });
}
pub(super) fn changed() -> bool {
    DIRTY.swap(false, Ordering::AcqRel)
}
pub(super) fn activate(command: Vec<String>) {
    if command.is_empty() {
        return;
    }
    if let Err(error) = crate::thread_spawn::spawn_named("todo-action", move || {
        use std::process::{Command, Stdio};
        match Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {}
            result => tracing::warn!(?result, "Todo action failed"),
        }
    }) {
        tracing::warn!(%error, "could not start Todo action");
    }
}
type StyledLines = Vec<(String, Style)>;

pub(super) fn mobile_entries(width: u16, palette: &Palette) -> Vec<(StyledLines, Vec<String>)> {
    let state = cache().lock().unwrap_or_else(|e| e.into_inner());
    if state.error.is_some() {
        return vec![(
            vec![(
                " Todo feed error (see logs)".into(),
                Style::default().fg(palette.red),
            )],
            Vec::new(),
        )];
    }
    let time = now();
    state
        .entries
        .iter()
        .filter(|e| visible(e, time))
        .map(|e| (entry_lines(e, width as usize, palette), e.action.clone()))
        .collect()
}

pub(super) fn render(
    buffer: &mut Buffer,
    area: Rect,
    config: &ClientShellConfig,
    scroll: &mut usize,
    hits: &mut ShellHitMap,
    collapsed: bool,
) {
    let state = cache().lock().unwrap_or_else(|e| e.into_inner());
    draw(buffer, area, config, scroll, hits, collapsed, &state);
}

fn draw(
    buffer: &mut Buffer,
    area: Rect,
    config: &ClientShellConfig,
    scroll: &mut usize,
    hits: &mut ShellHitMap,
    collapsed: bool,
    state: &Cache,
) {
    if area.is_empty() {
        return;
    }
    let muted = Style::default().fg(config.palette.overlay0);
    let header = if collapsed { 0 } else { 2.min(area.height) };
    if !collapsed {
        put_text(
            buffer,
            area.x,
            area.y,
            area.width,
            &"─".repeat(area.width as usize),
            muted,
        );
        if area.height > 1 {
            put_text(
                buffer,
                area.x,
                area.y + 1,
                area.width,
                " todo",
                muted.add_modifier(Modifier::BOLD),
            );
        }
    }
    let body = Rect::new(area.x, area.y + header, area.width, area.height - header);
    hits.agent_body = body;
    hits.agent_sort_toggle = Rect::default();
    hits.agent_scrollbar = Rect::default();
    hits.agent_scroll_metrics = None;
    let time = now();
    let entries = state
        .entries
        .iter()
        .filter(|e| visible(e, time))
        .collect::<Vec<_>>();
    let cards = entries
        .iter()
        .map(|entry| {
            if collapsed {
                let (icon, _, color) = entry.kind.presentation(&config.palette);
                vec![(
                    format!(" {}", entry.icon.as_deref().unwrap_or(icon)),
                    Style::default().fg(color),
                )]
            } else {
                entry_lines(entry, body.width as usize, &config.palette)
            }
        })
        .collect::<Vec<_>>();
    let gap = usize::from(!collapsed);
    let total = cards.iter().map(Vec::len).sum::<usize>() + gap * cards.len().saturating_sub(1);
    hits.agent_max_scroll = total.saturating_sub(body.height as usize);
    *scroll = (*scroll).min(hits.agent_max_scroll);
    if body.is_empty() {
        return;
    }
    if entries.is_empty() {
        put_text(
            buffer,
            body.x,
            body.y,
            body.width,
            if state.error.is_some() {
                " Todo feed error (see logs)"
            } else {
                " Nothing pending"
            },
            muted,
        );
    }
    let mut offset = 0usize;
    for (entry, lines) in entries.iter().zip(cards.iter()) {
        let start = offset.max(*scroll);
        let end = (offset + lines.len()).min(*scroll + body.height as usize);
        if start < end {
            for row in start..end {
                let (text, style) = &lines[row - offset];
                put_text(
                    buffer,
                    body.x,
                    body.y + (row - *scroll) as u16,
                    body.width,
                    text,
                    *style,
                );
            }
            if !entry.action.is_empty() {
                hits.todos.push((
                    Rect::new(
                        body.x,
                        body.y + (start - *scroll) as u16,
                        body.width,
                        (end - start) as u16,
                    ),
                    entry.action.clone(),
                ));
            }
        }
        offset += lines.len() + gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn todo_feed_comment_preview_wraps_and_card_is_clickable() {
        let entries = parse(br#"{"entries":[{"source":"gitlab","id":"142","kind":"assigned_comments","title":"!142 Fix login","detail":"api","comment":{"author":"Avery","text":"Could we retry the request before failing? This needs a regression test."},"action":["open","https://example.com"]}]}"#).unwrap();
        let state = Cache {
            entries,
            error: None,
        };
        let area = Rect::new(0, 0, 30, 8);
        let config = ClientShellConfig::from_config(&Config::default());
        let mut buffer = Buffer::empty(area);
        let mut hits = ShellHitMap::default();
        draw(&mut buffer, area, &config, &mut 0, &mut hits, false, &state);
        assert_eq!(hits.todos[0].0, Rect::new(0, 2, 30, 4));
        let text = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Avery: Could we retry"));
        assert!(text.contains('…'));
        assert!(text.contains('\u{f442}'));
        assert_eq!(hits.agent_max_scroll, 0);
        for row in preview(
            "你好，世界。 This is a reply with emoji 👋 and more text",
            10,
        ) {
            assert!(UnicodeWidthStr::width(row.as_str()) <= 10);
        }
    }

    #[test]
    fn todo_feed_states_are_distinct_and_unknown_kinds_remain_usable() {
        let palette = Palette::catppuccin();
        let states = [
            EntryKind::ReviewRequested,
            EntryKind::AssignedComments,
            EntryKind::AssignedApproved,
            EntryKind::CommentReply,
        ];
        let icons = states
            .iter()
            .map(|kind| kind.presentation(&palette).0)
            .collect::<HashSet<_>>();
        assert_eq!(icons.len(), states.len());
        let entries = parse(br#"{"entries":[{"source":"plugin","id":"1","title":"Custom","kind":"future_event","icon":"*"}]}"#).unwrap();
        assert!(entry_lines(&entries[0], 30, &palette)[0].0.contains('*'));
    }

    #[test]
    fn todo_feed_renderer_clamps_scroll_and_maps_visible_actions() {
        let state = Cache { entries: parse(br#"{"entries":[{"source":"test","id":"one","title":"First"},{"source":"test","id":"two","title":"Second","action":["helper","arg"]}]}"#).unwrap(), error: None };
        let area = Rect::new(0, 0, 40, 3);
        let mut buffer = Buffer::empty(area);
        let config = ClientShellConfig::from_config(&Config::default());
        let mut hits = ShellHitMap::default();
        let mut scroll = 999;
        draw(
            &mut buffer,
            area,
            &config,
            &mut scroll,
            &mut hits,
            false,
            &state,
        );
        assert_eq!(scroll, 2);
        assert_eq!(hits.todos.len(), 1);
        assert_eq!(hits.todos[0].0, Rect::new(0, 2, 40, 1));
        assert_eq!(hits.todos[0].1, ["helper", "arg"]);
        let text = buffer
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("todo"));
        assert!(text.contains("Second"));
        assert!(!text.contains("First"));
        draw(
            &mut buffer,
            Rect::new(0, 0, 40, 1),
            &config,
            &mut scroll,
            &mut hits,
            false,
            &state,
        );
    }
    #[test]
    fn todo_feed_priority_visibility_and_literal_actions() {
        let entries = parse(br#"{"entries":[{"source":"gitlab","id":"mr1","title":"Review","action":["open","https://example.com/$literal"]},{"source":"calendar","id":"meeting","title":"Join","priority":10,"visible_from":100,"expires_at":200}]}"#).unwrap();
        assert_eq!(entries[0].id, "meeting");
        assert!(!visible(&entries[0], 99));
        assert!(visible(&entries[0], 100));
        assert!(!visible(&entries[0], 200));
        assert_eq!(entries[1].action[1], "https://example.com/$literal");
    }
    #[test]
    fn todo_feed_rejects_duplicates_and_malformed_input() {
        assert!(parse(br#"{"entries":[{"source":"a","id":"1","title":"x"},{"source":"a","id":"1","title":"y"}]}"#).is_err());
        assert!(parse(b"{}").is_err());
    }
}
