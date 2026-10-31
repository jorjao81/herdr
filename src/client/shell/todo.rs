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
    priority: i32,
    #[serde(default)]
    visible_from: Option<u64>,
    #[serde(default)]
    expires_at: Option<u64>,
    /// Executable followed by literal arguments; never interpreted by a shell.
    #[serde(default)]
    action: Vec<String>,
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
pub(super) fn mobile_entries() -> Vec<(String, Vec<String>)> {
    let state = cache().lock().unwrap_or_else(|e| e.into_inner());
    if state.error.is_some() {
        return vec![(" Todo feed error (see logs)".into(), Vec::new())];
    }
    let time = now();
    state
        .entries
        .iter()
        .filter(|e| visible(e, time))
        .map(|e| {
            (
                format!(" [{}] {} {}", e.source, e.title, e.detail),
                e.action.clone(),
            )
        })
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
    let style = Style::default().fg(config.palette.text);
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
    let count = state.entries.iter().filter(|e| visible(e, time)).count();
    hits.agent_max_scroll = count.saturating_sub(body.height as usize);
    *scroll = (*scroll).min(hits.agent_max_scroll);
    if body.is_empty() {
        return;
    }
    if count == 0 {
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
    for (index, entry) in state
        .entries
        .iter()
        .filter(|e| visible(e, time))
        .skip(*scroll)
        .take(body.height as usize)
        .enumerate()
    {
        let rect = Rect::new(body.x, body.y + index as u16, body.width, 1);
        let label = if collapsed {
            format!("{}", index + 1)
        } else if entry.detail.is_empty() {
            format!(" [{}] {}", entry.source, entry.title)
        } else {
            format!(" [{}] {} · {}", entry.source, entry.title, entry.detail)
        };
        put_text(buffer, rect.x, rect.y, rect.width, &label, style);
        if !entry.action.is_empty() {
            hits.todos.push((rect, entry.action.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(scroll, 1);
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
