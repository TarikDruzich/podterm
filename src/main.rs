//! podterm-rs 0.4.2 — Rust CLIAMP-style podcast player, with downloads and recent history.
//! Data compatibility: ~/.local/share/podterm/{library,progress}.json
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{self, BufRead, BufReader, Stdout, Write},
    os::unix::{fs::{OpenOptionsExt, PermissionsExt}, net::UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{atomic::{AtomicU64, Ordering}, mpsc::{self, Receiver, Sender}},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event as TermEvent, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use quick_xml::{events::{BytesStart, Event as XmlEvent}, Reader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use unicode_width::UnicodeWidthChar;

const APP_VERSION: &str = "0.4.2";
const DOWNLOAD_MAX_SIZE: u64 = 536_870_912; // 512 MiB per episode
const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(600);
const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX_FILES: usize = 40;
const CACHE_MAX_FILE_SIZE: u64 = 2_097_152;
static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);
const MPV_INTERVAL: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Podcast {
    #[serde(default)] title: String,
    #[serde(default)] author: String,
    #[serde(default)] feed: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Episode {
    #[serde(default)] title: String,
    #[serde(default)] date: String,
    #[serde(default)] url: String,
    #[serde(default)] description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DownloadedEpisode {
    url: String,
    title: String,
    podcast: String,
    feed: String,
    date: String,
    filename: String,
    downloaded_at: u64,
}

#[derive(Clone, Debug)]
struct RecentEpisode {
    episode: Episode,
    podcast: Podcast,
    position: f64,
    played_at: u64,
}

#[derive(Clone)]
enum Item {
    Action(&'static str, &'static str),
    Podcast(Podcast),
    Episode(Episode),
    Recent(RecentEpisode),
    Downloaded(DownloadedEpisode),
    Label(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen { Home, Library, Search, Episodes, EpisodeSearch, Recent, Downloads, Help }

#[derive(Clone)]
struct View {
    screen: Screen,
    items: Vec<Item>,
    index: usize,
    scroll: usize,
    podcast: Option<Podcast>,
    query: String,
    episode_source: Vec<Episode>,
    busy: bool,
}

enum TaskResult {
    Search(Vec<Podcast>),
    Episodes(String, Vec<Episode>),
    Rss(Podcast),
    DownloadCompleted(String, Result<DownloadedEpisode, String>),
}
struct TaskReply { seq: u64, outcome: Result<TaskResult, String> }

fn home_dir() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
        .join(".local/share/podterm")
}
fn library_path() -> PathBuf { home_dir().join("library.json") }
fn history_path() -> PathBuf { home_dir().join("progress.json") }
fn downloads_path() -> PathBuf { home_dir().join("downloads.json") }
fn downloads_dir() -> PathBuf { home_dir().join("downloads") }
fn media_extension(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url).to_ascii_lowercase();
    for ext in ["mp3", "m4a", "ogg", "opus", "aac", "flac", "mp4", "webm"] {
        if path.ends_with(&format!(".{ext}")) {
            return match ext {
                "mp3" => "mp3", "m4a" => "m4a", "ogg" => "ogg",
                "opus" => "opus", "aac" => "aac", "flac" => "flac",
                "mp4" => "mp4", _ => "webm",
            };
        }
    }
    "audio"
}
fn download_filename(url: &str) -> String {
    let key = cache_key(url);
    format!("{}.{}", key.trim_end_matches(".json"), media_extension(url))
}
fn local_download(entry: &DownloadedEpisode) -> Option<PathBuf> {
    // Entries store a basename only, never an absolute path or path traversal.
    if Path::new(&entry.filename).file_name()?.to_str()? != entry.filename.as_str()
        || entry.filename.contains('/') || entry.filename.contains('\\') { return None; }
    let file = downloads_dir().join(&entry.filename);
    let meta = fs::symlink_metadata(&file).ok()?;
    if meta.file_type().is_file() && meta.len() > 0 { Some(file) } else { None }
}
fn download_to_disk(episode: Episode, podcast: Podcast) -> Result<DownloadedEpisode, String> {
    if !valid_url(&episode.url) { return Err("Invalid episode URL".into()); }
    let dir = downloads_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create download folder: {e}"))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("Cannot secure download folder: {e}"))?;
    let filename = download_filename(&episode.url);
    let target = dir.join(&filename);
    // Never follow a previous symlink as an output file.
    if fs::symlink_metadata(&target).map(|m| m.file_type().is_file() && m.len() > 0).unwrap_or(false) {
        return Ok(DownloadedEpisode { url: episode.url, title: episode.title,
            podcast: podcast.title, feed: podcast.feed, date: episode.date,
            filename, downloaded_at: now_seconds() });
    }
    let tmp = dir.join(format!(".{filename}.{}.part", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(&tmp).map_err(|e| format!("Cannot create temporary download: {e}"))?;
    drop(file);
    let status = Command::new("curl")
        .args(["-fLsS", "--connect-timeout", "10", "--max-time", "3600",
            "--max-filesize", "536870912", "--output"])
        .arg(&tmp)
        .args(["--", &episode.url])
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status();
    let completed = match status {
        Ok(s) if s.success() => fs::metadata(&tmp).ok()
            .map(|m| m.len() > 0 && m.len() <= DOWNLOAD_MAX_SIZE).unwrap_or(false),
        _ => false,
    };
    if !completed {
        let _ = fs::remove_file(&tmp);
        return Err("Download failed, was empty, or exceeded 512 MiB".into());
    }
    if fs::symlink_metadata(&target).is_ok() {
        // The target should not have appeared during this download.
        let _ = fs::remove_file(&tmp);
        return Err("Download target already exists".into());
    }
    if let Err(e) = fs::rename(&tmp, &target) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Cannot save download: {e}"));
    }
    Ok(DownloadedEpisode { url: episode.url, title: episode.title,
        podcast: podcast.title, feed: podcast.feed, date: episode.date,
        filename, downloaded_at: now_seconds() })
}

// Disk cache stores feed metadata and episode URLs. Some private RSS URLs contain
// credentials, so JSON cache files are created with owner-only permissions.
// Existing subscriptions/progress remain in their original files.
fn cache_dir() -> PathBuf {
    let base = env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".cache"));
    base.join("podterm-rs/episodes")
}

// Stable FNV-1a hash: std::hash's default hashing algorithm is not guaranteed to
// remain stable between Rust versions, and raw feed URLs make unsafe filenames.
fn cache_key(feed: &str) -> String {
    let hash = feed.as_bytes().iter().fold(0xcbf29ce484222325_u64, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}.json")
}
fn cache_path(feed: &str) -> PathBuf { cache_dir().join(cache_key(feed)) }
fn now_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
#[derive(Serialize, Deserialize)]
struct CachedEpisodes {
    version: u8,
    feed: String,
    fetched_at: u64,
    episodes: Vec<Episode>,
}
impl CachedEpisodes {
    fn age(&self) -> Duration { Duration::from_secs(now_seconds().saturating_sub(self.fetched_at)) }
    fn is_fresh(&self) -> bool { self.age() < CACHE_TTL }
}
fn read_cached_episodes(feed: &str) -> Option<CachedEpisodes> {
    let path = cache_path(feed);
    if fs::metadata(&path).ok()?.len() > CACHE_MAX_FILE_SIZE { return None; }
    let contents = fs::read(path).ok()?;
    let value: CachedEpisodes = serde_json::from_slice(&contents).ok()?;
    if value.version != 1 || value.feed != feed || value.episodes.len() > 500
        || value.episodes.iter().any(|ep| !valid_url(&ep.url)) {
        return None;
    }
    Some(value)
}
fn prune_episode_cache() {
    let Ok(list) = fs::read_dir(cache_dir()) else { return; };
    let mut files: Vec<_> = list.flatten().filter_map(|entry| {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") { return None; }
        let mtime = entry.metadata().ok()?.modified().ok()?;
        Some((mtime, path))
    }).collect();
    if files.len() <= CACHE_MAX_FILES { return; }
    files.sort_unstable_by_key(|item| item.0);
    let excess = files.len().saturating_sub(CACHE_MAX_FILES);
    for (_, path) in files.into_iter().take(excess) { let _ = fs::remove_file(path); }
}
fn save_cached_episodes(feed: &str, episodes: &[Episode]) -> Result<(), String> {
    let record = CachedEpisodes { version: 1, feed: feed.to_owned(),
        fetched_at: now_seconds(), episodes: episodes.to_vec() };
    write_json(&cache_path(feed), &record)?;
    prune_episode_cache();
    Ok(())
}
fn fetch_and_cache_episodes(feed: String) -> Result<TaskResult, String> {
    let episodes = load_episodes(&feed)?;
    // A disk or cache permission error must never prevent audio playback.
    if !episodes.is_empty() { let _ = save_cached_episodes(&feed, &episodes); }
    Ok(TaskResult::Episodes(feed, episodes))
}


fn read_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> T {
    fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}
fn write_json<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let dir = path.parent().ok_or("Missing data directory")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let id = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{id}", std::process::id()));
    let content = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
        .open(&tmp).map_err(|e| e.to_string())?;
    if let Err(err) = file.write_all(&content) {
        let _ = fs::remove_file(&tmp);
        return Err(err.to_string());
    }
    drop(file);
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => { let _ = fs::remove_file(&tmp); Err(err.to_string()) }
    }
}
fn valid_url(s: &str) -> bool { s.starts_with("https://") || s.starts_with("http://") }
fn feed_key(feed: &str) -> &str { feed.trim().trim_end_matches('/') }
fn stamp(seconds: f64) -> String {
    let s = if seconds.is_finite() && seconds > 0.0 { seconds as u64 } else { 0 };
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60) }
    else { format!("{:02}:{:02}", s / 60, s % 60) }
}

// curl keeps TLS and HTTP support out of the Rust binary. Each request runs on
// a worker thread and never blocks key handling or UI rendering.
fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if !valid_url(url) { return Err("Expected an http(s) URL".into()); }
    let output = Command::new("curl")
        .args(["-fsSL", "--compressed", "--connect-timeout", "7", "--max-time", "20",
               "--max-filesize", "8388608", "--user-agent", "podterm-rs/0.4.2", "--", url])
        .output().map_err(|e| format!("curl is required: {e}"))?;
    if !output.status.success() {
        return Err(format!("Could not load URL (curl exit {}).", output.status));
    }
    if output.stdout.len() > 8_388_608 { return Err("Feed is too large (8 MB limit)".into()); }
    Ok(output.stdout)
}
fn search_podcasts(term: &str) -> Result<Vec<Podcast>, String> {
    let url = format!("https://itunes.apple.com/search?term={}&media=podcast&entity=podcast&limit=35&country=us", urlencoding::encode(term));
    let body: Value = serde_json::from_slice(&fetch(&url)?).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    if let Some(results) = body["results"].as_array() {
        for p in results {
            let feed = p["feedUrl"].as_str().unwrap_or("");
            if !valid_url(feed) { continue; }
            out.push(Podcast { title: p["collectionName"].as_str().unwrap_or("Untitled podcast").into(),
                author: p["artistName"].as_str().unwrap_or("").into(), feed: feed.into() });
        }
    }
    Ok(out)
}
fn local_tag(q: &[u8]) -> &[u8] { q.rsplit(|c| *c == b':').next().unwrap_or(q) }
fn attribute(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    for attr in tag.attributes().flatten() {
        if local_tag(attr.key.as_ref()) == name {
            if let Ok(s) = attr.unescape_value() { return Some(s.into_owned()); }
        }
    }
    None
}
fn set_audio(tag: &BytesStart<'_>, node_name: &[u8], episode: &mut Episode) {
    if node_name == b"enclosure" {
        if let Some(url) = attribute(tag, b"url") { if valid_url(&url) { episode.url = url; } }
    } else if node_name == b"link" && attribute(tag, b"rel").as_deref() == Some("enclosure") {
        if let Some(url) = attribute(tag, b"href") { if valid_url(&url) { episode.url = url; } }
    }
}
fn parse_feed(data: &[u8]) -> Result<(String, Vec<Episode>), String> {
    let mut reader = Reader::from_reader(data);
    // Preserve spaces in mixed text/entity content (e.g. "A &amp; B").
    // Trimming each XML Text event would turn "A &amp; B" into "A&B".
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut feed_title = String::new();
    let mut episodes = Vec::new();
    let mut entry: Option<Episode> = None;
    let mut field: Vec<u8> = Vec::new();
    // Depth tracks nested tags, to avoid treating an embedded <title> as an episode title.
    let mut depth = 0usize;
    let mut feed_title_active = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let tag_name = e.name();
                let name = local_tag(tag_name.as_ref());
                if name == b"item" || name == b"entry" {
                    entry = Some(Episode::default());
                    depth = 0;
                } else if let Some(ep) = entry.as_mut() {
                    set_audio(&e, name, ep);
                    depth += 1;
                } else if name == b"title" && feed_title.is_empty() {
                    feed_title_active = true;
                }
                field = name.to_vec();
            }
            Ok(XmlEvent::Empty(e)) => {
                if let Some(ep) = entry.as_mut() {
                    let tag_name = e.name();
                    set_audio(&e, local_tag(tag_name.as_ref()), ep);
                }
                field.clear();
            }
            Ok(XmlEvent::Text(e)) => {
                let value = e.decode().map_err(|e| e.to_string())?.into_owned();
                if let Some(ep) = entry.as_mut() {
                    if depth == 1 { match field.as_slice() {
                        b"title" => ep.title.push_str(&value),
                        b"pubDate" | b"updated" | b"published" => ep.date.push_str(&value),
                        b"description" | b"summary" | b"encoded" => ep.description.push_str(&value),
                        _ => {}
                    }}
                } else if feed_title_active && field == b"title" { feed_title.push_str(&value); }
            }
            Ok(XmlEvent::GeneralRef(e)) => {
                let name = e.decode().map_err(|e| e.to_string())?;
                let reference = format!("&{name};");
                if let Ok(value) = quick_xml::escape::unescape(&reference) {
                    if let Some(ep) = entry.as_mut() {
                        if depth == 1 { match field.as_slice() {
                            b"title" => ep.title.push_str(&value),
                            b"pubDate" | b"updated" | b"published" => ep.date.push_str(&value),
                            b"description" | b"summary" | b"encoded" => ep.description.push_str(&value),
                            _ => {}
                        }}
                    } else if feed_title_active && field == b"title" { feed_title.push_str(&value); }
                }
            }
            Ok(XmlEvent::CData(e)) => {
                let value = e.decode().map_err(|e| e.to_string())?.into_owned();
                if let Some(ep) = entry.as_mut() {
                    if depth == 1 { match field.as_slice() {
                            b"title" => ep.title.push_str(&value),
                            b"description" | b"summary" | b"encoded" => ep.description.push_str(&value),
                            _ => {}
                        }}
                } else if feed_title_active && field == b"title" { feed_title.push_str(&value); }
            }
            Ok(XmlEvent::End(e)) => {
                let tag_name = e.name();
                let name = local_tag(tag_name.as_ref());
                if name == b"item" || name == b"entry" {
                    if let Some(mut ep) = entry.take() {
                        if valid_url(&ep.url) && episodes.len() < 500 {
                            ep.title = ep.title.trim().to_string();
                            if ep.title.is_empty() { ep.title = "Untitled episode".into(); }
                            ep.date = ep.date.trim().chars().take(22).collect();
                            // Keep a small searchable excerpt, not the entire show notes.
                            ep.description = ep.description.trim().chars().take(2000).collect();
                            episodes.push(ep);
                        }
                    }
                    depth = 0;
                } else if entry.is_some() { depth = depth.saturating_sub(1); }
                else if name == b"title" { feed_title_active = false; }
                field.clear();
            }
            Ok(XmlEvent::Eof) => break,
            Err(e) => return Err(format!("Invalid RSS/Atom XML: {e}")),
            _ => {}
        }
        buf.clear();
    }
    Ok((feed_title.trim().to_string(), episodes))
}
// Search only the playable episodes returned by the current RSS feed.
// Apple's *podcast catalog* search is a separate feature; it does not search this show.
fn matching_episodes(episodes: &[Episode], query: &str) -> Vec<Episode> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() { return episodes.to_vec(); }
    episodes.iter().filter(|episode| {
        let searchable = format!("{} {} {}", episode.title, episode.description, episode.date).to_lowercase();
        words.iter().all(|word| searchable.contains(word))
    }).cloned().collect()
}

fn load_episodes(feed: &str) -> Result<Vec<Episode>, String> { Ok(parse_feed(&fetch(feed)?)?.1) }
fn load_rss(feed: &str) -> Result<Podcast, String> {
    let (title, _) = parse_feed(&fetch(feed)?)?;
    Ok(Podcast { title: if title.is_empty() { "My podcast".into() } else { title }, author: String::new(), feed: feed.into() })
}

// Keep a little audio prefetched while bounding mpv's *packet* buffers.
// Decoders/libraries still allocate their own memory; this does not cap RSS.
fn low_memory_mpv_args() -> &'static [&'static str] {
    &["--cache=auto", "--cache-secs=30", "--demuxer-max-bytes=8MiB",
      "--demuxer-max-back-bytes=2MiB", "--demuxer-hysteresis-secs=10"]
}
// Favor uninterrupted audio playback. The lower-memory mpv settings are
// opt-in because they can cause stuttering on some podcast streams.
fn low_memory_setting(value: Option<&str>) -> bool {
    matches!(value, Some("1"))
}
fn low_memory_enabled() -> bool {
    low_memory_setting(env::var("PODTERM_MPV_LOW_MEMORY").ok().as_deref())
}

struct Player {
    child: Option<Child>,
    socket: Option<PathBuf>,
    episode: Option<Episode>,
    podcast_title: String,
    position: f64,
    duration: f64,
    paused: bool,
    pending_resume: Option<f64>,
    position_confirmed: bool,
    started_at: Instant,
    history: HashMap<String, Value>,
    last_save: Instant,
}
impl Player {
    fn new() -> Self {
        Self { child: None, socket: None, episode: None, podcast_title: String::new(),
            position: 0.0, duration: 0.0, paused: false, pending_resume: None,
            position_confirmed: false, started_at: Instant::now(),
            history: read_json(&history_path()), last_save: Instant::now() }
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.as_mut().and_then(|ch| ch.try_wait().ok()), Some(None))
    }
    fn command(&mut self, args: Value) -> Option<Value> {
        if !self.alive() { return None; }
        let path = self.socket.as_ref()?;
        let mut conn = UnixStream::connect(path).ok()?;
        conn.set_read_timeout(Some(Duration::from_millis(160))).ok()?;
        conn.set_write_timeout(Some(Duration::from_millis(160))).ok()?;
        writeln!(conn, "{}", json!({"command":args,"request_id":42})).ok()?;
        let mut reader = BufReader::new(conn);
        let mut line = String::new();
        // MPV can emit asynchronous events before the command response.
        for _ in 0..24 {
            line.clear();
            if reader.read_line(&mut line).ok()? == 0 { return None; }
            let response: Value = match serde_json::from_str(&line) { Ok(v) => v, Err(_) => continue };
            if response["request_id"] == 42 && response["error"] == "success" {
                return Some(response["data"].clone());
            }
            if response["request_id"] == 42 { return None; }
        }
        None
    }
    fn start(&mut self, podcast: &Podcast, episode: &Episode,
             downloaded_file: Option<&Path>) -> Result<(), String> {
        self.stop();
        if !valid_url(&episode.url) { return Err("This episode has no audio URL".into()); }
        let saved = self.history.get(&episode.url).and_then(|v| v["position"].as_f64()).unwrap_or(0.0).max(0.0);
        self.position = saved;
        self.duration = 0.0;
        self.paused = false;
        self.pending_resume = (saved > 5.0).then_some(saved);
        self.position_confirmed = false;
        self.started_at = Instant::now();
        self.last_save = Instant::now();
        let path = home_dir().join(format!("mpv-rs-{}.sock", std::process::id()));
        fs::create_dir_all(home_dir()).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(&path);
        let mut command = Command::new("mpv");
        command.args(["--no-video", "--no-terminal", "--really-quiet"]);
        if low_memory_enabled() { command.args(low_memory_mpv_args()); }
        command.arg(format!("--input-ipc-server={}", path.display()));
        if saved > 5.0 { command.arg(format!("--start={saved:.2}")); }
        let media = downloaded_file.map(|p| p.as_os_str().to_os_string())
            .unwrap_or_else(|| std::ffi::OsString::from(episode.url.as_str()));
        let child = command.arg("--").arg(media)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("Could not start mpv: {e}"))?;
        // Keep old positions and metadata, even when loading historical Python data.
        let previous = self.history.remove(&episode.url).unwrap_or_else(|| json!({}));
        let mut entry = previous.as_object().cloned().unwrap_or_default();
        entry.insert("title".into(), json!(episode.title));
        entry.insert("date".into(), json!(episode.date));
        entry.insert("podcast".into(), json!(podcast.title));
        entry.insert("feed".into(), json!(podcast.feed));
        entry.insert("played_at".into(), json!(now_seconds()));
        self.history.insert(episode.url.clone(), Value::Object(entry));
        let _ = write_json(&history_path(), &self.history);
        self.child = Some(child);
        self.socket = Some(path);
        self.episode = Some(episode.clone());
        self.podcast_title = podcast.title.clone();
        Ok(())
    }
    fn tick(&mut self) -> bool {
        if self.child.is_none() { return false; }
        if !self.alive() { self.save(); self.child = None; return true; }
        let old_pos = self.position;
        let old_paused = self.paused;
        if let Some(Value::Number(n)) = self.command(json!(["get_property", "time-pos"])) {
            if let Some(pos) = n.as_f64() {
                if let Some(target) = self.pending_resume {
                    if pos >= target - 3.0 {
                        self.pending_resume = None;
                        self.position_confirmed = true;
                        self.position = pos;
                    } else if self.started_at.elapsed() < Duration::from_secs(25) {
                        self.command(json!(["seek", target, "absolute"]));
                    } else {
                        self.pending_resume = None;
                        self.position_confirmed = true;
                        self.position = pos;
                    }
                } else {
                    self.position_confirmed = true;
                    self.position = pos;
                }
            }
        }
        if let Some(d) = self.command(json!(["get_property", "duration"])) {
            if let Some(v) = d.as_f64() { self.duration = v; }
        }
        if let Some(Value::Bool(value)) = self.command(json!(["get_property", "pause"])) { self.paused = value; }
        if self.last_save.elapsed() >= Duration::from_secs(8) { self.save(); self.last_save = Instant::now(); }
        (self.position - old_pos).abs() > 0.4 || self.paused != old_paused
    }
    fn save(&mut self) {
        let Some(ep) = self.episode.as_ref() else { return; };
        if !self.position_confirmed && self.pending_resume.is_some() { return; }
        let pos = if self.duration > 0.0 && self.position >= self.duration - 12.0 { 0.0 } else { self.position };
        // Preserve played_at/feed/date when updating the playback position.
        let entry = self.history.entry(ep.url.clone()).or_insert_with(|| json!({}));
        if let Some(map) = entry.as_object_mut() {
            map.insert("position".into(), json!((pos * 10.0).round() / 10.0));
            map.insert("title".into(), json!(ep.title));
            map.insert("podcast".into(), json!(self.podcast_title));
        }
        let _ = write_json(&history_path(), &self.history);
    }
    fn stop(&mut self) {
        if self.child.is_some() {
            if self.alive() {
                if self.pending_resume.is_none() {
                    if let Some(v) = self.command(json!(["get_property", "time-pos"])) {
                        if let Some(pos) = v.as_f64() {
                            self.position = pos.max(0.0);
                            self.position_confirmed = true;
                        }
                    }
                }
                if let Some(v) = self.command(json!(["get_property", "duration"])) {
                    if let Some(n) = v.as_f64() { self.duration = n; }
                }
            }
            self.save();
            let _ = self.command(json!(["quit"]));
            if let Some(mut child) = self.child.take() {
                for _ in 0..10 {
                    if child.try_wait().ok().flatten().is_some() { break; }
                    thread::sleep(Duration::from_millis(35));
                }
                if child.try_wait().ok().flatten().is_none() { let _ = child.kill(); }
                let _ = child.wait();
            }
        }
        if let Some(path) = self.socket.take() { let _ = fs::remove_file(path); }
        self.episode = None;
        self.pending_resume = None;
    }
    fn pause_toggle(&mut self) { let _ = self.command(json!(["cycle", "pause"])); }
    fn seek(&mut self, offset: i64) { let _ = self.command(json!(["seek", offset, "relative"])); }
}

struct App {
    screen: Screen,
    items: Vec<Item>,
    library: Vec<Podcast>,
    history: Vec<View>,
    index: usize, scroll: usize,
    podcast: Option<Podcast>,
    query: String,
    episode_source: Vec<Episode>,
    seq: u64,
    busy: bool,
    status: String,
    cache: HashMap<String, (Instant, Vec<Episode>)>,
    downloads: Vec<DownloadedEpisode>,
    downloading: HashSet<String>,
    last_auto_refresh: Instant,
    tx: Sender<TaskReply>,
    rx: Receiver<TaskReply>,
    player: Player,
    dirty: bool,
}
impl App {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let mut a = Self { screen: Screen::Home, items: Vec::new(), library: read_json(&library_path()),
            history: Vec::new(), index: 0, scroll: 0, podcast: None, query: String::new(),
            episode_source: Vec::new(), seq: 0,
            busy: false, status: "Welcome! / Search · l Library · a RSS · ? Help".into(),
            cache: HashMap::new(), downloads: read_json(&downloads_path()),
            downloading: HashSet::new(), last_auto_refresh: Instant::now(),
            tx, rx, player: Player::new(), dirty: true };
        a.home();
        a
    }
    fn snapshot(&self) -> View {
        View { screen: self.screen, items: self.items.clone(), index: self.index, scroll: self.scroll,
            podcast: self.podcast.clone(), query: self.query.clone(),
            episode_source: self.episode_source.clone(), busy: self.busy }
    }
    fn remember(&mut self) { let view = self.snapshot(); self.history.push(view); if self.history.len() > 24 { self.history.remove(0); } }
    fn reset_view(&mut self, screen: Screen) {
        self.seq += 1;
        self.screen = screen;
        self.items.clear(); self.index = 0; self.scroll = 0; self.busy = false; self.dirty = true;
        // A filtered view gets a snapshot of the source; only entering a different
        // screen clears it, so Esc can restore the original episode list.
        if screen != Screen::EpisodeSearch { self.episode_source.clear(); }
    }
    fn recent_entries(&self) -> Vec<RecentEpisode> {
        let mut entries: Vec<_> = self.player.history.iter().filter_map(|(url, info)| {
            if !valid_url(url) { return None; }
            let title = info.get("title")?.as_str()?.to_string();
            let podcast = info.get("podcast").and_then(Value::as_str).unwrap_or("Unknown podcast");
            let feed = info.get("feed").and_then(Value::as_str).unwrap_or("");
            let date = info.get("date").and_then(Value::as_str).unwrap_or("");
            let position = info.get("position").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
            Some(RecentEpisode {
                episode: Episode { title, date: date.into(), url: url.clone(), description: String::new() },
                podcast: Podcast { title: podcast.into(), author: String::new(), feed: feed.into() },
                position, played_at: info.get("played_at").and_then(Value::as_u64).unwrap_or(0),
            })
        }).collect();
        entries.sort_by(|a,b| b.played_at.cmp(&a.played_at).then_with(|| a.episode.title.cmp(&b.episode.title)));
        entries.truncate(50);
        entries
    }
    fn home(&mut self) {
        self.reset_view(Screen::Home);
        self.history.clear();
        self.items = Vec::new();
        if self.recent_entries().iter().any(|r| r.position > 5.0) {
            self.items.push(Item::Action("Continue Listening", "continue"));
        }
        self.items.extend([
            Item::Action("Recently Played", "recent"),
            Item::Action("My Library", "library"),
            Item::Action("Downloaded Episodes", "downloads"),
            Item::Action("Search Apple Podcasts", "search"),
            Item::Action("Add RSS Feed", "rss"),
            Item::Action("Keyboard Shortcuts", "help")
        ]);
    }
    fn recent(&mut self) {
        if self.screen != Screen::Recent { self.remember(); }
        self.reset_view(Screen::Recent);
        self.items = self.recent_entries().into_iter().map(Item::Recent).collect();
        self.status = format!("{} recently played episodes · Enter to resume", self.items.len());
    }
    fn downloads(&mut self) {
        if self.screen != Screen::Downloads { self.remember(); }
        self.reset_view(Screen::Downloads);
        self.downloads = read_json(&downloads_path());
        self.items = self.downloads.iter()
            .filter(|e| local_download(e).is_some()).cloned().map(Item::Downloaded).collect();
        self.status = format!("{} offline episodes · Enter play · d delete download", self.items.len());
    }
    fn continue_listening(&mut self) {
        if let Some(recent) = self.recent_entries().into_iter().find(|r| r.position > 5.0) {
            self.play_episode(&recent.podcast, &recent.episode);
        } else { self.status = "No unfinished episode yet".into(); }
    }
    fn play_episode(&mut self, podcast: &Podcast, episode: &Episode) {
        let local = self.downloads.iter().find(|e| e.url == episode.url)
            .and_then(local_download);
        match self.player.start(podcast, episode, local.as_deref()) {
            Ok(()) => self.status = if self.player.position > 5.0 {
                format!("Resuming at {}{}", stamp(self.player.position), if local.is_some() { " · offline" } else { "" })
            } else if local.is_some() { "Playing downloaded episode · Space to pause".into() }
            else { "Playing · Space to pause".into() },
            Err(err) => self.status = err,
        }
    }
    fn download_selected(&mut self) {
        let choice = match self.items.get(self.index) {
            Some(Item::Episode(ep)) if matches!(self.screen, Screen::Episodes | Screen::EpisodeSearch) =>
                self.podcast.clone().map(|p| (ep.clone(), p)),
            Some(Item::Recent(r)) if self.screen == Screen::Recent =>
                Some((r.episode.clone(), r.podcast.clone())),
            _ => None,
        };
        let Some((episode, podcast)) = choice else {
            self.status = "Select an episode from a podcast or Recently Played to download".into(); return;
        };
        if self.downloads.iter().any(|d| d.url == episode.url && local_download(d).is_some()) {
            self.status = "Episode already downloaded".into(); return;
        }
        if self.downloading.len() >= 2 && !self.downloading.contains(&episode.url) {
            self.status = "Two downloads already running; finish one before starting another".into(); return;
        }
        if !self.downloading.insert(episode.url.clone()) {
            self.status = "Episode download already running".into(); return;
        }
        let tx = self.tx.clone();
        let url = episode.url.clone();
        self.status = format!("Downloading: {} (background)", episode.title);
        thread::spawn(move || {
            let result = download_to_disk(episode, podcast);
            let _ = tx.send(TaskReply { seq: u64::MAX,
                outcome: Ok(TaskResult::DownloadCompleted(url, result)) });
        });
    }
    fn delete_download(&mut self) {
        if self.screen != Screen::Downloads { return; }
        let Some(Item::Downloaded(record)) = self.items.get(self.index).cloned() else { return; };
        // Deleting the audio does not touch listening history or subscriptions.
        if let Some(path) = local_download(&record) {
            if let Err(err) = fs::remove_file(path) {
                self.status = format!("Cannot delete audio: {err}"); return;
            }
        }
        self.downloads.retain(|e| e.url != record.url);
        if let Err(err) = write_json(&downloads_path(), &self.downloads) {
            self.status = format!("Audio deleted but index update failed: {err}"); return;
        }
        let selected = self.index;
        self.downloads();
        self.index = selected.min(self.items.len().saturating_sub(1));
        self.status = "Downloaded audio deleted · listening progress preserved".into();
    }
    fn library(&mut self, push: bool) {
        if push && self.screen != Screen::Library { self.remember(); }
        self.reset_view(Screen::Library);
        self.library = read_json(&library_path());
        self.items.push(Item::Action("+ Add Podcast", "search"));
        self.items.extend(self.library.iter().cloned().map(Item::Podcast));
        self.status = format!("Library: {} podcasts · Enter on + to add · d to remove", self.library.len());
    }
    fn help(&mut self) {
        if self.screen != Screen::Help { self.remember(); }
        self.reset_view(Screen::Help);
        self.items = vec![
            "↑ / ↓  or  j / k       Move selection",
            "Enter                   Open selected item / Play episode",
            "Space                   Pause / Resume playback",
            "← / →                   Rewind / Forward 5 seconds",
            "s                       Stop playback",
            "r                       Refresh episodes (on Episodes screen)",
            "/                       Search episodes inside an open podcast",
            "x                       Download selected episode for offline playback",
            "d                       Delete download (on Downloads screen)",
            "Home                    Continue / Recent / Downloads",
            "/                       Search podcasts from other screens",
            "l                       Open My Library",
            "a                       Add an RSS feed",
            "f                       Save podcast to My Library",
            "d                       Remove selected podcast (Library only)",
            "Esc / b                 Go back",
            "q                       Quit",
            "?                       Keyboard shortcuts",
        ].into_iter().map(|v| Item::Label(v.into())).collect();
        self.status = "Esc or b to return".into();
    }
    fn spawn(&mut self, work: impl FnOnce() -> Result<TaskResult, String> + Send + 'static) {
        self.seq += 1;
        let seq = self.seq;
        self.busy = true;
        self.status = "Loading...".into();
        let tx = self.tx.clone();
        thread::spawn(move || { let _ = tx.send(TaskReply { seq, outcome: work() }); });
    }
    fn search(&mut self, query: String) {
        self.remember();
        self.reset_view(Screen::Search);
        self.query = query.clone();
        self.spawn(move || search_podcasts(&query).map(TaskResult::Search));
    }
    fn search_episodes(&mut self, query: String) {
        if !matches!(self.screen, Screen::Episodes | Screen::EpisodeSearch) { return; }
        let source = if self.screen == Screen::EpisodeSearch {
            self.episode_source.clone()
        } else {
            self.items.iter().filter_map(|item| match item {
                Item::Episode(episode) => Some(episode.clone()), _ => None
            }).collect()
        };
        // When opening a podcast without a cache, don't misreport an empty
        // response as a completed search. Wait for the RSS fetch to finish.
        if source.is_empty() && self.busy {
            self.status = "Episodes are loading; search again when ready".into();
            self.dirty = true;
            return;
        }
        self.remember();
        self.reset_view(Screen::EpisodeSearch);
        self.episode_source = source;
        self.query = query;
        self.items = matching_episodes(&self.episode_source, &self.query)
            .into_iter().map(Item::Episode).collect();
        self.status = format!("{} matches in {} available episodes · Esc/b to clear search",
            self.items.len(), self.episode_source.len());
    }
    fn open_podcast(&mut self, podcast: Podcast) {
        if !valid_url(&podcast.feed) { self.status = "This podcast has no RSS URL".into(); return; }
        self.remember();
        self.reset_view(Screen::Episodes);
        self.podcast = Some(podcast.clone());
        self.last_auto_refresh = Instant::now();
        let feed = podcast.feed;
        // RAM cache for same-session navigation; disk cache survives restarts.
        if let Some((when, episodes)) = self.cache.get(&feed) {
            if when.elapsed() < CACHE_TTL {
                self.items = episodes.iter().cloned().map(Item::Episode).collect();
                self.status = format!("{} episodes · memory cache · r refresh", self.items.len());
                return;
            }
        }
        if let Some(cached) = read_cached_episodes(&feed) {
            let fresh = cached.is_fresh();
            let when = Instant::now().checked_sub(cached.age()).unwrap_or_else(Instant::now);
            self.items = cached.episodes.iter().cloned().map(Item::Episode).collect();
            if fresh {
                self.cache.insert(feed.clone(), (when, cached.episodes));
                self.status = format!("{} episodes · disk cache · r refresh", self.items.len());
                return;
            }
            // Stale-while-revalidate: old list stays selectable while refreshing.
            self.status = format!("{} cached episodes · updating in background...", self.items.len());
        }
        let has_cache = !self.items.is_empty();
        self.spawn(move || fetch_and_cache_episodes(feed));
        if has_cache {
            self.status = format!("{} cached episodes · updating in background...", self.items.len());
        }
    }
    fn refresh_episodes(&mut self) {
        if let Some(p) = &self.podcast {
            self.last_auto_refresh = Instant::now();
            let feed = p.feed.clone();
            self.spawn(move || fetch_and_cache_episodes(feed));
            self.status = "Refreshing episodes · cached list remains available".into();
        }
    }
    fn back(&mut self) {
        self.seq += 1;
        self.busy = false;
        if let Some(view) = self.history.pop() {
            self.screen = view.screen; self.items = view.items; self.index = view.index;
            self.scroll = view.scroll; self.podcast = view.podcast; self.query = view.query;
            self.episode_source = view.episode_source;
            self.status = "Back".into();
            if view.busy {
                match self.screen {
                    Screen::Episodes => self.refresh_episodes(),
                    Screen::Search => { let term = self.query.clone(); self.items.clear();
                        self.spawn(move || search_podcasts(&term).map(TaskResult::Search)); },
                    _ => {}
                }
            }
        } else { self.home(); }
        // A saved Home snapshot may predate the last played episode.
        if self.screen == Screen::Home { self.home(); }
        self.dirty = true;
    }
    fn process_results(&mut self) {
        while let Ok(reply) = self.rx.try_recv() {
            if reply.seq == u64::MAX {
                if let Ok(TaskResult::DownloadCompleted(url, outcome)) = reply.outcome {
                    self.downloading.remove(&url);
                    match outcome {
                        Ok(record) => {
                            self.downloads.retain(|old| old.url != record.url);
                            self.downloads.push(record);
                            match write_json(&downloads_path(), &self.downloads) {
                                Ok(()) => {
                                    self.status = "Episode downloaded. Press Home > Downloaded Episodes".into();
                                    if self.screen == Screen::Downloads { self.downloads(); }
                                }
                                Err(err) => self.status = format!("Download completed, but index could not be saved: {err}"),
                            }
                        },
                        Err(message) => self.status = format!("Download failed: {message}"),
                    }
                    self.dirty = true;
                }
                continue;
            }
            if reply.seq != self.seq { continue; }
            self.busy = false;
            match reply.outcome {
                Ok(TaskResult::Search(results)) if self.screen == Screen::Search => {
                    self.items = results.into_iter().map(Item::Podcast).collect();
                    self.index = 0; self.scroll = 0;
                    self.status = format!("Loaded {} podcasts · f to save selected podcast", self.items.len());
                }
                Ok(TaskResult::Episodes(feed, episodes)) if self.screen == Screen::Episodes => {
                    // Don't replace playable offline entries with a temporarily empty feed.
                    if episodes.is_empty() && !self.items.is_empty() {
                        self.status = "Feed returned no playable episodes · showing cached list".into();
                        self.dirty = true;
                        continue;
                    }
                    self.cache.retain(|_, (when, _)| when.elapsed() < CACHE_TTL);
                    if self.cache.len() >= 20 && !self.cache.contains_key(&feed) {
                        if let Some(oldest) = self.cache.iter().min_by_key(|(_, (when, _))| *when).map(|(key, _)| key.clone()) {
                            self.cache.remove(&oldest);
                        }
                    }
                    self.cache.insert(feed, (Instant::now(), episodes.clone()));
                    let selected_url = self.items.get(self.index).and_then(|item| match item {
                        Item::Episode(ep) => Some(ep.url.clone()), _ => None,
                    });
                    self.items = episodes.into_iter().map(Item::Episode).collect();
                    self.index = selected_url.and_then(|url| self.items.iter().position(|item| {
                        matches!(item, Item::Episode(ep) if ep.url == url)
                    })).unwrap_or(0);
                    self.scroll = self.scroll.min(self.index);
                    self.status = format!("Loaded {} episodes · r refresh", self.items.len());
                }
                Ok(TaskResult::Rss(podcast)) => self.save_podcast(&podcast),
                Ok(_) => {},
                Err(message) => {
                    self.status = if self.screen == Screen::Episodes && !self.items.is_empty() {
                        format!("Update failed; showing cached episodes · {message}")
                    } else { format!("Error: {message}") };
                },
            }
            self.dirty = true;
        }
    }
    fn is_saved(&self, pod: &Podcast) -> bool {
        self.library.iter().any(|p| feed_key(&p.feed) == feed_key(&pod.feed))
    }
    fn save_podcast(&mut self, podcast: &Podcast) {
        if !valid_url(&podcast.feed) { self.status = "This podcast has no valid RSS URL".into(); return; }
        let mut library: Vec<Podcast> = read_json(&library_path());
        if library.iter().any(|p| feed_key(&p.feed) == feed_key(&podcast.feed)) {
            self.library = library; self.status = "Already in your library".into(); return;
        }
        library.push(podcast.clone());
        match write_json(&library_path(), &library) {
            Ok(()) => {
                self.library = library;
                if self.screen == Screen::Library { self.library(false); }
                self.status = format!("Podcast added: {}", podcast.title);
            }
            Err(e) => self.status = format!("Could not save library: {e}"),
        }
        self.dirty = true;
    }
    fn save_selected(&mut self) {
        if self.busy && self.items.is_empty() { self.status = "Loading; please wait".into(); return; }
        let pod = if self.screen == Screen::Search {
            match self.items.get(self.index) { Some(Item::Podcast(p)) => Some(p.clone()), _ => None }
        } else if matches!(self.screen, Screen::Episodes | Screen::EpisodeSearch) { self.podcast.clone() } else { None };
        if let Some(p) = pod { self.save_podcast(&p); }
        else { self.status = "Search (/), select podcast, press f to save".into(); }
    }
    fn remove_selected(&mut self) {
        if self.screen != Screen::Library { return; }
        let Some(Item::Podcast(p)) = self.items.get(self.index) else {
            self.status = "Select a podcast (not + Add Podcast)".into(); return;
        };
        let key = feed_key(&p.feed).to_string();
        let mut library: Vec<Podcast> = read_json(&library_path());
        library.retain(|p| feed_key(&p.feed) != key);
        if let Err(err) = write_json(&library_path(), &library) {
            self.status = format!("Could not update library: {err}"); return;
        }
        let selected = self.index;
        self.library(false);
        self.index = selected.min(self.items.len().saturating_sub(1));
        self.status = "Podcast removed from library".into();
    }
    fn select(&mut self) {
        if self.busy && self.items.is_empty() { return; }
        let Some(item) = self.items.get(self.index).cloned() else { return; };
        match item {
            Item::Action(_, cmd) => self.status = format!("INPUT:{cmd}"), // dispatched from main loop
            Item::Podcast(p) => self.open_podcast(p),
            Item::Episode(ep) => if let Some(pod) = self.podcast.clone() {
                self.play_episode(&pod, &ep);
            },
            Item::Recent(r) => self.play_episode(&r.podcast, &r.episode),
            Item::Downloaded(d) => {
                let pod = Podcast { title: d.podcast.clone(), author: String::new(), feed: d.feed.clone() };
                let ep = Episode { title: d.title, date: d.date, url: d.url, description: String::new() };
                self.play_episode(&pod, &ep);
            },
            Item::Label(_) => {},
        }
        self.dirty = true;
    }
}

// No full screen redraw on every input poll: only on activity and mpv updates.
fn display_width_clipped(text: &str, limit: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if width + w > limit { break; }
        width += w;
        result.push(ch);
    }
    result
}
fn put(out: &mut Stdout, row: u16, col: u16, max: usize, text: &str, color: Color, selected: bool) -> io::Result<()> {
    queue!(out, MoveTo(col, row), SetForegroundColor(color))?;
    if selected { queue!(out, SetBackgroundColor(Color::DarkGreen), SetForegroundColor(Color::Black))?; }
    queue!(out, Print(display_width_clipped(text, max)), ResetColor)?;
    Ok(())
}
fn draw(out: &mut Stdout, app: &mut App) -> io::Result<()> {
    let (w, h) = terminal::size()?;
    queue!(out, Clear(ClearType::All))?;
    if w < 44 || h < 14 {
        put(out, 0, 0, w as usize, "Terminal must be at least 44x14.", Color::White, false)?;
        out.flush()?; return Ok(());
    }
    put(out, 1, 2, 16, "◉ PODTERM", Color::Green, false)?;
    put(out, 1, 19, w.saturating_sub(22) as usize, &format!("v{APP_VERSION} · Rust / Apple Podcasts / RSS"), Color::Cyan, false)?;
    put(out, 2, 2, (w-4) as usize, &"─".repeat((w-4) as usize), Color::DarkGrey, false)?;
    let heading = match app.screen {
        Screen::Home => "HOME".into(),
        Screen::Library => "MY LIBRARY".into(),
        Screen::Search => "SEARCH RESULTS".into(),
        Screen::Episodes => app.podcast.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| "EPISODES".into()),
        Screen::EpisodeSearch => format!("EPISODE RESULTS: {}", app.query),
        Screen::Recent => "RECENTLY PLAYED".into(),
        Screen::Downloads => "DOWNLOADED EPISODES".into(),
        Screen::Help => "KEYBOARD SHORTCUTS".into(),
    };
    put(out, 4, 3, (w-6) as usize, &heading, Color::White, false)?;
    let visible = h.saturating_sub(13) as usize;
    app.index = app.index.min(app.items.len().saturating_sub(1));
    if app.index < app.scroll { app.scroll = app.index; }
    if app.index >= app.scroll + visible { app.scroll = app.index - visible + 1; }
    for i in app.scroll..app.items.len().min(app.scroll + visible) {
        let text = match &app.items[i] {
            Item::Action(t, _) => (*t).to_string(),
            Item::Podcast(p) if app.screen == Screen::Search => format!("{}{}  — {}", if app.is_saved(p) { "✓ " } else { "  " }, p.title, p.author),
            Item::Podcast(p) => p.title.clone(),
            Item::Episode(ep) => format!("{}{}{} · {}", if app.downloads.iter().any(|d| d.url == ep.url && local_download(d).is_some()) { "↓ " } else { "" }, if app.player.history.contains_key(&ep.url) { "↺ " } else { "" }, ep.title, ep.date),
            Item::Recent(r) => format!("{} — {} · at {}", r.episode.title, r.podcast.title, stamp(r.position)),
            Item::Downloaded(d) => format!("{} — {} [offline]", d.title, d.podcast),
            Item::Label(t) => t.clone(),
        };
        let selected = i == app.index;
        put(out, 6 + (i-app.scroll) as u16, 3, (w-6) as usize,
            &format!("{} {text}", if selected { "❯" } else { " " }), Color::White, selected)?;
    }
    if app.items.is_empty() {
        let empty_text = if app.busy { "Loading..." }
            else if app.screen == Screen::EpisodeSearch { "No matches among the available RSS episodes. Esc/b to clear search." }
            else { "No items. Press / to search or Esc to go back." };
        put(out, 7, 4, (w-7) as usize, empty_text, Color::DarkGrey, false)?;
    }
    put(out, h-7, 2, (w-4) as usize, &"─".repeat((w-4) as usize), Color::DarkGrey, false)?;
    let player = &mut app.player;
    let playing = player.alive() && player.episode.is_some();
    let play_text = if playing { if player.paused { "❚❚ PAUSED" } else { "▶ PLAYING" } } else { "■ PLAYER" };
    put(out, h-6, 3, (w-6) as usize, play_text, Color::Green, false)?;
    put(out, h-5, 3, (w-6) as usize,
        player.episode.as_ref().map(|e| e.title.as_str()).unwrap_or("Nothing playing"), Color::White, false)?;
    let barw = (w as usize).saturating_sub(28).clamp(8, 55);
    let ratio = if player.duration > 0.0 { (player.position / player.duration).clamp(0.0, 1.0) } else { 0.0 };
    let filled = ((ratio * barw as f64) as usize).min(barw);
    put(out, h-4, 3, (w-6) as usize, &format!("{}{} {} / {}", "━".repeat(filled), "─".repeat(barw-filled), stamp(player.position), stamp(player.duration)), Color::Green, false)?;
    put(out, h-3, 3, (w-6) as usize, "↑/↓ j/k Select   Enter Play   Space Pause   ←/→ ±5s   s Stop   x Download", Color::White, false)?;
    put(out, h-2, 3, (w-6) as usize, "/ Search podcasts/episodes   l Library   a RSS   f Save   d Remove   r Refresh   Esc/b Back   q Quit   ? Help", Color::Cyan, false)?;
    put(out, h-1, 2, (w-3) as usize, &app.status, Color::DarkGrey, false)?;
    out.flush()
}
fn prompt(out: &mut Stdout, label: &str) -> io::Result<Option<String>> {
    let mut text = String::new();
    execute!(out, Show)?;
    let answer = loop {
        let (w, h) = terminal::size()?;
        queue!(out, MoveTo(0, h.saturating_sub(1)), Clear(ClearType::CurrentLine))?;
        let caption = format!("{label}: {text}");
        queue!(out, SetForegroundColor(Color::Cyan), Print(display_width_clipped(&caption, w.saturating_sub(2) as usize)), ResetColor)?;
        out.flush()?;
        match event::read()? {
            TermEvent::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => break Some(text.trim().to_string()),
                KeyCode::Esc => break None,
                KeyCode::Backspace => { text.pop(); },
                KeyCode::Char(c) if !key.modifiers.contains(event::KeyModifiers::CONTROL) && text.len() < 1800 => { text.push(c); },
                _ => {},
            },
            TermEvent::Paste(v) => { if text.len() + v.len() < 1800 { text.push_str(&v); } },
            _ => {},
        }
    };
    execute!(out, Hide)?;
    Ok(answer.filter(|v| !v.is_empty()))
}
struct TerminalGuard;
impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen, Hide) {
            let _ = terminal::disable_raw_mode(); return Err(e);
        }
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), ResetColor, Show, LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}
fn action(app: &mut App, out: &mut Stdout, command: &str) -> io::Result<()> {
    match command {
        "search" => {
            let in_podcast = matches!(app.screen, Screen::Episodes | Screen::EpisodeSearch);
            if let Some(term) = prompt(out, if in_podcast { "Search episodes" } else { "Search podcasts" })? {
                if in_podcast { app.search_episodes(term); }
                else { app.search(term); }
            }
        },
        "library" => app.library(true),
        "recent" => app.recent(),
        "downloads" => app.downloads(),
        "continue" => app.continue_listening(),
        "help" => app.help(),
        "rss" => if let Some(feed) = prompt(out, "RSS URL")? {
            app.spawn(move || load_rss(&feed).map(TaskResult::Rss));
        },
        _ => {},
    }
    app.dirty = true;
    Ok(())
}
fn run(app: &mut App) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();
    let mut last_tick = Instant::now();
    loop {
        app.process_results();
        // Refresh only the currently open feed; never poll every subscribed podcast.
        if app.screen == Screen::Episodes && !app.busy
            && app.last_auto_refresh.elapsed() >= AUTO_REFRESH_INTERVAL {
            app.refresh_episodes();
        }
        if last_tick.elapsed() >= MPV_INTERVAL {
            app.dirty |= app.player.tick();
            last_tick = Instant::now();
        }
        if app.dirty { draw(&mut out, app)?; app.dirty = false; }
        if !event::poll(Duration::from_millis(180))? { continue; }
        let TermEvent::Key(key) = event::read()? else { app.dirty = true; continue; };
        if key.kind == KeyEventKind::Release { continue; }
        app.dirty = true;
        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Down | KeyCode::Char('j') => { if !app.items.is_empty() { app.index = (app.index+1).min(app.items.len()-1); } },
            KeyCode::Up | KeyCode::Char('k') => app.index = app.index.saturating_sub(1),
            KeyCode::Esc | KeyCode::Char('b') | KeyCode::Backspace => app.back(),
            KeyCode::Enter => {
                // Home and + Add Podcast rows are actions rather than podcasts.
                let cmd = match app.items.get(app.index) { Some(Item::Action(_, cmd)) => Some(*cmd), _ => None };
                if let Some(cmd) = cmd { action(app, &mut out, cmd)?; } else { app.select(); }
            },
            KeyCode::Char('/') => action(app, &mut out, "search")?,
            KeyCode::Char('l') => action(app, &mut out, "library")?,
            KeyCode::Char('a') => action(app, &mut out, "rss")?,
            KeyCode::Char('?') => action(app, &mut out, "help")?,
            KeyCode::Char('f') => app.save_selected(),
            KeyCode::Char('d') => {
                if app.screen == Screen::Downloads {
                    if prompt(&mut out, "Delete selected downloaded audio? y/N")?
                        .is_some_and(|answer| answer.eq_ignore_ascii_case("y")) {
                        app.delete_download();
                    }
                } else { app.remove_selected(); }
            },
            KeyCode::Char('x') => app.download_selected(),
            KeyCode::Char(' ') => app.player.pause_toggle(),
            KeyCode::Left => app.player.seek(-5),
            KeyCode::Right => app.player.seek(5),
            KeyCode::Char('s') => { app.player.stop(); app.status = "Stopped; playback position saved".into(); },
            KeyCode::Char('r') if app.screen == Screen::Episodes && !app.busy => app.refresh_episodes(),
            _ => {},
        }
    }
    app.player.stop();
    Ok(())
}
fn main() {
    let mut app = App::new();
    if let Err(err) = run(&mut app) {
        app.player.stop();
        eprintln!("podterm-rs error: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_rss_and_atom() {
        let xml = br#"<rss><channel><title>My show</title><item><title>Episode &amp; One</title><pubDate>Fri, 09 Oct 2026 12:00:00 GMT</pubDate><enclosure url="https://x.example/a.mp3" type="audio/mpeg"/></item></channel></rss>"#;
        let (title, episodes) = parse_feed(xml).unwrap();
        assert_eq!(title, "My show");
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].title, "Episode & One");
        assert_eq!(episodes[0].url, "https://x.example/a.mp3");
        let atom = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom</title><entry><title>Number two</title><link rel="enclosure" href="https://x.example/b.mp3" /></entry></feed>"#;
        assert_eq!(parse_feed(atom).unwrap().1[0].title, "Number two");
    }
    #[test]
    fn preserves_spaces_around_entities() {
        let xml = br#"<rss><channel><title>  My show  </title><item>
          <title>  History &amp; Football  </title>
          <pubDate>  Fri, 09 Oct 2026  </pubDate>
          <enclosure url="https://example.test/episode.mp3" />
        </item><item><title><![CDATA[  Sports  ]]> &amp; News</title>
          <enclosure url="https://example.test/second.mp3" />
        </item></channel></rss>"#;
        let (title, episodes) = parse_feed(xml).unwrap();
        assert_eq!(title, "My show");
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].title, "History & Football");
        assert_eq!(episodes[0].date, "Fri, 09 Oct 2026");
        assert_eq!(episodes[1].title, "Sports   & News");
    }
    #[test]
    fn handles_missing_urls_and_cdata() {
        let xml = br#"<rss><channel><title>Test</title><item><title><![CDATA[Great episode]]></title><enclosure url="https://x.example/p.mp3" /></item><item><title>Missing</title></item></channel></rss>"#;
        let (_, episodes) = parse_feed(xml).unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].title, "Great episode");
    }
    #[test]
    fn cache_round_trip_and_staleness() {
        let feed = "https://example.invalid/podterm-cache-test.xml";
        let ep = Episode { title: "Test & More".into(),
            date: "09 Oct".into(), url: "https://example.invalid/e.mp3".into(), description: String::new() };
        let json = serde_json::to_string(&CachedEpisodes {
            version: 1, feed: feed.into(), fetched_at: now_seconds(), episodes: vec![ep]
        }).unwrap();
        let restored: CachedEpisodes = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.feed, feed);
        assert_eq!(restored.episodes[0].title, "Test & More");
        assert!(restored.is_fresh());
        let expired = CachedEpisodes { fetched_at: now_seconds().saturating_sub(660), ..restored };
        assert!(!expired.is_fresh());
        assert_eq!(cache_key(feed), cache_key(feed));
        assert_ne!(cache_key(feed), cache_key("https://example.invalid/other"));
        assert!(cache_path(feed).extension().is_some());
    }
    #[test]
    fn searches_episode_titles_descriptions_and_dates() {
        let eps = vec![
            Episode { title: "#2512 - Joey Diaz".into(),
                description: "Comedian and actor".into(), date: "June 10, 2026".into(),
                url: "https://example.test/1.mp3".into() },
            Episode { title: "#2513 - Someone Else".into(),
                description: "Special guest is Joey Diaz".into(), date: "June 11, 2026".into(),
                url: "https://example.test/2.mp3".into() },
            Episode { title: "#2514 - Other Guest".into(),
                description: String::new(), date: "June 12, 2026".into(),
                url: "https://example.test/3.mp3".into() },
        ];
        assert_eq!(matching_episodes(&eps, "joey diaz").len(), 2);
        assert_eq!(matching_episodes(&eps, "DIAZ JOEY").len(), 2);
        assert_eq!(matching_episodes(&eps, "2512").len(), 1);
        assert_eq!(matching_episodes(&eps, "unknown").len(), 0);
        assert_eq!(matching_episodes(&eps, "   ").len(), 3);
    }
    #[test]
    fn searches_rss_description_and_supports_old_cache_format() {
        let xml = br#"<rss><channel><title>Show</title><item>
          <title>Episode 100</title>
          <description><![CDATA[An interview with Joey Diaz.]]></description>
          <enclosure url="https://example.test/100.mp3"/>
          </item></channel></rss>"#;
        let (_, eps) = parse_feed(xml).unwrap();
        assert_eq!(matching_episodes(&eps, "joey diaz").len(), 1);
        let old_json = r#"{"title":"Old episode","date":"Yesterday","url":"https://example.test/old.mp3"}"#;
        let old: Episode = serde_json::from_str(old_json).unwrap();
        assert!(old.description.is_empty());
    }
    #[test]
    fn standard_audio_is_default_and_low_memory_is_opt_in() {
        assert!(!low_memory_setting(None));
        assert!(!low_memory_setting(Some("0")));
        assert!(!low_memory_setting(Some("false")));
        assert!(low_memory_setting(Some("1")));
    }
    #[test]
    fn low_memory_options_have_safe_bounds() {
        let options = low_memory_mpv_args().join(" ");
        assert!(options.contains("--demuxer-max-bytes=8MiB"));
        assert!(options.contains("--demuxer-max-back-bytes=2MiB"));
        assert!(options.contains("--cache-secs=30"));
    }
    #[test]
    fn download_metadata_and_filename() {
        let url = "https://example.invalid/file.M4A?token=secret";
        let filename = download_filename(url);
        assert!(filename.ends_with(".m4a"));
        assert!(!filename.contains("secret"));
        assert_eq!(filename, download_filename(url));
        let record = DownloadedEpisode {
            url: url.into(), title: "Episode".into(), podcast: "Show".into(),
            feed: "https://example.invalid/feed".into(), date: "today".into(),
            filename, downloaded_at: 123,
        };
        let restored: DownloadedEpisode = serde_json::from_str(
            &serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(restored.title, "Episode");
        assert_eq!(restored.downloaded_at, 123);
        assert_eq!(media_extension("https://example.invalid/audio?x=1"), "audio");
    }
    #[test]
    fn formatting_and_feed_keys() {
        assert_eq!(feed_key("https://example.com/show/"), "https://example.com/show");
        assert_eq!(stamp(3723.0), "1:02:03");
        assert_eq!(stamp(65.0), "01:05");
    }
}
