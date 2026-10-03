//! Headless, single-owner rotation. The socket transports commands, never platform calls.
use crate::{
    catalogue::{self, Artwork},
    rotation::{self, Preferences, Queue, Source, State},
    rotation_catalogue::{self, Cache},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::{
    fs::PermissionsExt,
    net::{UnixListener, UnixStream},
};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, SystemTime},
};
const MAX_BYTES: usize = 16 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Status,
    Start { prefs: Preferences },
    Save { prefs: Preferences },
    Pause,
    Resume,
    Stop,
    ChangeNow,
    Retry,
    RetrySkipped { id: String },
    ManualWallpaper { art: Artwork, path: PathBuf },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub prefs: Preferences,
    pub state: State,
    pub wants_running: bool,
    pub deadline: Option<SystemTime>,
    pub current: Option<Artwork>,
    pub current_path: Option<PathBuf>,
    pub next: Option<Artwork>,
    pub prepared_path: Option<PathBuf>,
    pub pending: bool,
    pub error: Option<String>,
    pub notice: String,
    pub skipped: Vec<(Artwork, String)>,
    pub eligible: Option<usize>,
    pub cache_diagnostic: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Durable {
    #[serde(default)]
    setup: Option<(Preferences, bool)>,
    #[serde(default)]
    start_first: bool,
    #[serde(default)]
    apply_first: bool,
    version: u32,
    snapshot: Snapshot,
    queue: Option<Queue>,
}
#[derive(Serialize, Deserialize)]
struct Reply {
    snapshot: Option<Snapshot>,
    error: Option<String>,
}
pub fn socket_path() -> Result<PathBuf> {
    Ok(crate::storage::data_dir()?.join("service/rotation.sock"))
}
/// Hold across GUI pause, manual download/application, and current-wallpaper notification.
#[cfg(unix)]
pub fn manual_guard() -> Result<File> {
    let path = socket_path()?
        .parent()
        .context("Service path")?
        .join("wallpaper.lock");
    let file = wallpaper_lock(&path)?;
    file.lock()?;
    Ok(file)
}
#[cfg(not(unix))]
pub fn manual_guard() -> Result<File> {
    anyhow::bail!("Background rotation requires macOS or Linux")
}
fn wallpaper_lock(path: &Path) -> Result<File> {
    let parent = path.parent().context("Wallpaper lock path")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    Ok(fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?)
}
fn runtime_path() -> Result<PathBuf> {
    Ok(crate::storage::data_dir()?.join("rotation-runtime.json"))
}
fn write_frame(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Service message exceeds size limit"
    );
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()?;
    Ok(())
}
fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> Result<T> {
    let mut size = [0; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    ensure!(size <= MAX_BYTES, "Service message exceeds size limit");
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(unix)]
pub fn request(command: Command) -> Result<Snapshot> {
    let mut stream = UnixStream::connect(socket_path()?)
        .context("Background rotation service is unavailable")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_frame(&mut stream, &command)?;
    let reply: Reply = read_frame(&mut stream)?;
    if let Some(error) = reply.error {
        anyhow::bail!("{error}");
    }
    reply.snapshot.context("Service returned no status")
}
#[cfg(not(unix))]
pub fn request(_: Command) -> Result<Snapshot> {
    anyhow::bail!("Background rotation requires macOS or Linux")
}
trait Effects: Send + Sync {
    fn download(&self, art: &Artwork) -> Result<PathBuf>;
    fn apply(&self, path: &Path) -> Result<()>;
    fn fetch_page(&self, page: usize) -> Result<catalogue::Page> {
        catalogue::fetch_page_info(page)
    }
}
struct RealEffects;
impl Effects for RealEffects {
    fn download(&self, art: &Artwork) -> Result<PathBuf> {
        Ok(crate::download::fetch(&art.original_url(), true)?.0)
    }
    fn apply(&self, path: &Path) -> Result<()> {
        if crate::platform::all_desktops_supported() {
            crate::platform::apply_all_desktops(path)
        } else {
            crate::platform::apply(path).map(|_| ())
        }
    }
}
fn permanent_original_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Status(404 | 410, _))
        )
    })
}
fn known_eligible(queue: &Queue, skipped: &[(Artwork, String)]) -> usize {
    queue.eligible_count(&skipped.iter().map(|(art, _)| art.id.clone()).collect())
}
fn prepare_original(
    queue: &mut Queue,
    bootstrap: &mut Option<rotation_catalogue::Bootstrap>,
    cancel: &AtomicBool,
    effects: &dyn Effects,
) -> Result<(PathBuf, Vec<(Artwork, String)>)> {
    let mut skipped = vec![];
    for _ in 0..rotation::MAX_ARTWORKS {
        ensure!(!cancel.load(Ordering::Relaxed), "Preparation cancelled");
        if (queue.next().is_none() || known_eligible(queue, &skipped) < 2) && !queue.complete() {
            let pages = bootstrap
                .as_ref()
                .map(|b| b.pages.clone())
                .unwrap_or_default();
            let items =
                rotation_catalogue::collect_with(|page| effects.fetch_page(page), cancel, &pages)?;
            queue.adopt_complete(items.clone(), rotation::random_seed())?;
            if let Some(b) = bootstrap {
                b.artworks = items;
                b.complete = true;
            }
        }
        ensure!(
            queue.source != Source::EntireGallery
                || !queue.complete()
                || known_eligible(queue, &skipped) >= 2,
            "Fewer than two known eligible gallery originals remain"
        );
        let art = queue
            .next()
            .cloned()
            .context("No available original remains in this gallery cycle")?;
        match effects.download(&art) {
            Ok(path) => {
                ensure!(!cancel.load(Ordering::Relaxed), "Preparation cancelled");
                return Ok((path, skipped));
            }
            Err(e) if queue.source == Source::EntireGallery && permanent_original_error(&e) => {
                skipped.push((art, format!("{e:#}")));
                queue.advance();
            }
            Err(e) => return Err(e),
        }
    }
    anyhow::bail!("Gallery original preparation exceeds the safe artwork limit")
}
type PreparedOriginal = (
    Preferences,
    Queue,
    PathBuf,
    Option<rotation_catalogue::Bootstrap>,
    Vec<(Artwork, String)>,
);
enum Event {
    Command(Command, mpsc::SyncSender<Reply>),
    Prepared(u64, Result<PreparedOriginal>),
    Downloaded(u64, String, Result<PathBuf>),
    Catalogue(u64, Result<Vec<Artwork>>),
}
struct Engine {
    snapshot: Snapshot,
    queue: Option<Queue>,
    cache: Cache,
    epoch: u64,
    cancel: Arc<AtomicBool>,
    work: bool,
    refresh: bool,
    setup: Option<(Preferences, bool)>,
    retry_at: Option<SystemTime>,
    attempts: usize,
    change_now: bool,
    start_first: bool,
    apply_first: bool,
    refresh_active: bool,
    refresh_attempts: usize,
    refresh_retry_at: Option<SystemTime>,
    path: PathBuf,
    prefs_path: PathBuf,
    cache_path: PathBuf,
    effects: Arc<dyn Effects>,
}
impl Engine {
    fn load(
        path: PathBuf,
        prefs_path: PathBuf,
        cache_path: PathBuf,
        effects: Arc<dyn Effects>,
    ) -> Result<Self> {
        let prefs = rotation::load(&prefs_path)?;
        let (cache, diagnostic) = match rotation_catalogue::load(&cache_path) {
            Ok(cache) => (cache, None),
            Err(e) => (Cache::empty(), Some(format!("Catalogue cache: {e:#}"))),
        };
        let mut snapshot = Snapshot {
            state: if prefs.configured {
                State::Paused
            } else {
                State::Stopped
            },
            prefs,
            wants_running: false,
            deadline: None,
            current: None,
            current_path: None,
            next: None,
            prepared_path: None,
            pending: false,
            error: None,
            notice: String::new(),
            skipped: vec![],
            eligible: None,
            cache_diagnostic: diagnostic,
        };
        let mut queue = None;
        let mut setup = None;
        let mut start_first = false;
        let mut apply_first = false;
        if path.exists() {
            let restore = (|| -> Result<Durable> {
                let mut bytes = vec![];
                File::open(&path)?
                    .take((MAX_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= MAX_BYTES,
                    "Rotation runtime exceeds size limit"
                );
                let value: Durable = serde_json::from_slice(&bytes)?;
                ensure!(value.version == 1, "Unsupported rotation runtime version");
                value.snapshot.prefs.validate()?;
                if let Some((prefs, _)) = &value.setup {
                    prefs.validate()?;
                    ensure!(prefs.configured, "Unconfigured setup candidate");
                }
                ensure!(
                    value.queue.is_some()
                        || (value.snapshot.next.is_none()
                            && value.snapshot.prepared_path.is_none()),
                    "Prepared candidate has no queue"
                );
                ensure!(
                    !value.snapshot.wants_running || value.queue.is_some() || value.setup.is_some(),
                    "Active rotation has no queue or setup"
                );
                for art in value
                    .snapshot
                    .current
                    .iter()
                    .chain(value.snapshot.next.iter())
                    .chain(value.snapshot.skipped.iter().map(|(art, _)| art))
                {
                    Preferences {
                        selected: vec![art.clone()],
                        ..Preferences::default()
                    }
                    .validate()?;
                }
                ensure!(
                    value.snapshot.skipped.len() <= rotation::MAX_ARTWORKS,
                    "Too many skipped originals"
                );
                for p in value
                    .snapshot
                    .current_path
                    .iter()
                    .chain(value.snapshot.prepared_path.iter())
                {
                    ensure!(
                        p.is_absolute()
                            && p.parent()
                                == Some(
                                    path.parent()
                                        .context("Runtime path")?
                                        .join("originals")
                                        .as_path()
                                ),
                        "Invalid persisted original path"
                    );
                }
                if let Some(q) = &value.queue {
                    q.validate()?;
                    ensure!(
                        q.source == value.snapshot.prefs.source,
                        "Queue source differs from preferences"
                    );
                    if q.source == Source::Selected {
                        ensure!(
                            q.matches_selected(&value.snapshot.prefs.selected),
                            "Selected queue differs from preferences"
                        );
                    }
                    if let Some(next) = &value.snapshot.next {
                        ensure!(
                            q.next().is_some_and(|art| art.id == next.id),
                            "Prepared original differs from queue"
                        );
                    }
                }
                ensure!(
                    value.snapshot.prepared_path.is_none() || value.snapshot.next.is_some(),
                    "Prepared original has no artwork"
                );
                ensure!(
                    !value.snapshot.wants_running
                        || value.snapshot.prefs.configured
                        || value.setup.is_some(),
                    "Unconfigured active rotation"
                );
                Ok(value)
            })();
            match restore {
                Ok(value) => {
                    let differs = serde_json::to_value(&snapshot.prefs)?
                        != serde_json::to_value(&value.snapshot.prefs)?;
                    if differs {
                        snapshot.current = value.snapshot.current;
                        snapshot.current_path = value.snapshot.current_path;
                        snapshot.error=Some("Saved preferences changed outside the runtime transaction; rotation remains paused. Resume to prepare the saved source.".into());
                    } else {
                        snapshot = value.snapshot;
                        queue = value.queue;
                        setup = value.setup;
                        start_first = value.start_first;
                        apply_first = value.apply_first;
                        snapshot.pending = setup.is_some();
                        if setup.is_some() {
                            if snapshot.state != State::Failed {
                                snapshot.state = State::Preparing;
                            }
                        } else {
                            snapshot.pending = false;
                            if snapshot.wants_running {
                                snapshot.state = State::Running;
                                if snapshot.deadline.is_none() && !apply_first {
                                    snapshot.deadline = Some(
                                        SystemTime::now()
                                            + Duration::from_secs(
                                                snapshot.prefs.minutes as u64 * 60,
                                            ),
                                    );
                                }
                            }
                            if snapshot
                                .prepared_path
                                .as_ref()
                                .is_some_and(|p| !p.is_file())
                            {
                                snapshot.prepared_path = None;
                            }
                        }
                    }
                }
                Err(e) => {
                    snapshot.error = Some(format!(
                        "Saved rotation runtime could not be restored: {e:#}"
                    ));
                }
            }
        }
        let change_now = apply_first && snapshot.wants_running && setup.is_none();
        Ok(Self {
            snapshot,
            queue,
            cache,
            epoch: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            work: false,
            refresh: false,
            setup,
            retry_at: None,
            attempts: 0,
            change_now,
            start_first,
            apply_first,
            refresh_active: false,
            refresh_attempts: 0,
            refresh_retry_at: None,
            path,
            prefs_path,
            cache_path,
            effects,
        })
    }
    fn persist(&self) -> Result<()> {
        let durable = Durable {
            version: 1,
            snapshot: self.snapshot.clone(),
            queue: self.queue.clone(),
            setup: self.setup.clone(),
            start_first: self.start_first,
            apply_first: self.apply_first,
        };
        let bytes = serde_json::to_vec(&durable)?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "Rotation runtime exceeds size limit"
        );
        let parent = self.path.parent().context("Runtime path")?;
        fs::create_dir_all(parent)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(&self.path).map_err(|e| e.error)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    }
    fn save_cache(&mut self) {
        if let Err(e) = rotation_catalogue::save(&self.cache_path, &self.cache) {
            self.snapshot.cache_diagnostic =
                Some(format!("Catalogue cache could not be saved: {e:#}"));
        }
    }
    fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.setup = None;
        self.start_first = false;
        self.apply_first = false;
        self.retry_at = None;
        self.attempts = 0;
        self.change_now = false;
        self.snapshot.pending = false;
        if !self.refresh_active {
            self.refresh = false;
        }
        self.refresh_attempts = 0;
        self.refresh_retry_at = None;
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(self.snapshot.prefs.minutes as u64 * 60)
    }
    fn command(&mut self, command: Command, now: SystemTime) -> Result<()> {
        let start = matches!(&command, Command::Start { .. });
        match command {
            Command::Status => return Ok(()),
            Command::Start { mut prefs } | Command::Save { mut prefs } => {
                let running = start
                    || (self.snapshot.wants_running && prefs.source == self.snapshot.prefs.source);
                if start {
                    prefs.configured = true;
                }
                prefs.validate()?;
                if !prefs.configured {
                    rotation::save(&self.prefs_path, &prefs)?;
                    self.invalidate();
                    self.snapshot.prefs = prefs;
                    self.snapshot.wants_running = false;
                    self.snapshot.deadline = None;
                    self.snapshot.state = State::Stopped;
                    self.queue = None;
                    self.snapshot.next = None;
                    self.snapshot.prepared_path = None;
                    return self.persist();
                }
                let same = prefs.source == self.snapshot.prefs.source
                    && (prefs.source == Source::EntireGallery
                        || self
                            .queue
                            .as_ref()
                            .is_some_and(|q| q.matches_selected(&prefs.selected)));
                if !start && same && self.queue.is_some() {
                    rotation::save(&self.prefs_path, &prefs)?;
                    self.snapshot.prefs = prefs;
                    if self.snapshot.wants_running {
                        self.snapshot.deadline = Some(now + self.interval());
                    }
                } else {
                    self.invalidate();
                    self.snapshot.wants_running = false;
                    self.snapshot.deadline = None;
                    self.snapshot.state = State::Preparing;
                    self.snapshot.pending = true;
                    self.snapshot.error = None;
                    self.start_first = start;
                    self.apply_first = start;
                    self.setup = Some((prefs, running));
                }
            }
            Command::Pause => {
                self.invalidate();
                self.snapshot.wants_running = false;
                self.snapshot.deadline = None;
                self.snapshot.state = State::Paused;
            }
            Command::Stop => {
                self.invalidate();
                self.snapshot.wants_running = false;
                self.snapshot.deadline = None;
                self.snapshot.state = State::Stopped;
            }
            Command::Resume => {
                ensure!(self.snapshot.prefs.configured, "Configure rotation first");
                self.invalidate();
                self.snapshot.wants_running = true;
                self.snapshot.error = None;
                self.snapshot.state = State::Running;
                self.snapshot.deadline = Some(now + self.interval());
                if self.queue.is_none() {
                    self.setup = Some((self.snapshot.prefs.clone(), true));
                    self.snapshot.state = State::Preparing;
                    self.snapshot.pending = true;
                    self.apply_first = false;
                }
            }
            Command::ChangeNow | Command::Retry => {
                ensure!(
                    self.snapshot.prefs.configured || self.setup.is_some(),
                    "Configure rotation first"
                );
                self.snapshot.pending = self.setup.is_some();
                self.retry_at = None;
                self.attempts = 0;
                self.snapshot.error = None;
                self.snapshot.wants_running = true;
                self.snapshot.state = State::Waiting;
                self.change_now = true;
                if self.queue.is_none() && self.setup.is_none() {
                    self.setup = Some((self.snapshot.prefs.clone(), true));
                }
            }
            Command::ManualWallpaper { art, path } => {
                Preferences {
                    selected: vec![art.clone()],
                    ..Preferences::default()
                }
                .validate()?;
                let originals = self
                    .path
                    .parent()
                    .context("Runtime path")?
                    .join("originals");
                ensure!(
                    path.is_absolute()
                        && path.is_file()
                        && path.parent() == Some(originals.as_path())
                        && path.canonicalize()?.parent()
                            == Some(originals.canonicalize()?.as_path()),
                    "Invalid retained original path"
                );
                self.invalidate();
                self.snapshot.wants_running = false;
                self.snapshot.deadline = None;
                self.snapshot.state = State::Paused;
                self.snapshot.current = Some(art.clone());
                self.snapshot.current_path = Some(path);
                self.snapshot.prepared_path = None;
                self.snapshot.next = None;
                if self.snapshot.prefs.source == Source::Selected && self.snapshot.prefs.configured
                {
                    self.queue = Some(Queue::new(
                        Source::Selected,
                        self.snapshot.prefs.selected.clone(),
                        Some(&art.id),
                        rotation::random_seed(),
                    )?);
                }
                if self
                    .queue
                    .as_ref()
                    .and_then(|q| q.next())
                    .is_some_and(|next| next.id == art.id)
                {
                    self.queue.as_mut().unwrap().advance();
                }
                self.cache.shown(&art.id);
                self.save_cache();
            }
            Command::RetrySkipped { id } => {
                ensure!(id.len() <= 512, "Invalid artwork ID");
                let index = self
                    .snapshot
                    .skipped
                    .iter()
                    .position(|(a, _)| a.id == id)
                    .context("Skipped original not found")?;
                let art = self.snapshot.skipped[index].0.clone();
                self.queue
                    .as_mut()
                    .context("No prepared catalogue")?
                    .retry_skipped(art)?;
                self.invalidate();
                self.snapshot.skipped.remove(index);
                self.snapshot.next = None;
                self.snapshot.prepared_path = None;
            }
        }
        self.persist()
    }
    fn launch(&mut self, tx: &mpsc::Sender<Event>, now: SystemTime) {
        if self.work || self.snapshot.state == State::Failed {
            return;
        }
        if let Some((prefs, _)) = self.setup.clone() {
            self.work = true;
            self.snapshot.pending = true;
            let epoch = self.epoch;
            let tx = tx.clone();
            let cancel = self.cancel.clone();
            let cache = self.cache.clone();
            let effects = self.effects.clone();
            let current = if self.start_first {
                None
            } else {
                self.snapshot.current.as_ref().map(|a| a.id.clone())
            };
            std::thread::spawn(move || {
                let result = (|| -> Result<_> {
                    let mut bootstrap = if prefs.source == Source::EntireGallery && !cache.complete
                    {
                        Some(rotation_catalogue::bootstrap(
                            |page| effects.fetch_page(page),
                            &cancel,
                            rotation::random_seed(),
                            &cache.seen_set(),
                            cache.last.as_deref(),
                        )?)
                    } else {
                        None
                    };
                    ensure!(!cancel.load(Ordering::Relaxed), "Preparation cancelled");
                    let mut queue = if prefs.source == Source::Selected {
                        Queue::new(
                            Source::Selected,
                            prefs.selected.clone(),
                            current.as_deref(),
                            rotation::random_seed(),
                        )?
                    } else {
                        let (items, complete, candidate) = if let Some(b) = &bootstrap {
                            (
                                b.artworks.clone(),
                                b.complete,
                                b.artworks.first().map(|a| a.id.as_str()),
                            )
                        } else {
                            (cache.artworks.clone(), true, None)
                        };
                        let mut q = Queue::gallery(
                            items,
                            &cache.seen_set(),
                            current.as_deref().or(cache.last.as_deref()),
                            rotation::random_seed(),
                            complete,
                            candidate,
                        )?;
                        q.begin_gallery_cycle(current.as_deref(), rotation::random_seed());
                        q
                    };
                    let (path, skipped) =
                        prepare_original(&mut queue, &mut bootstrap, &cancel, effects.as_ref())?;
                    Ok((prefs, queue, path, bootstrap, skipped))
                })();
                let _ = tx.send(Event::Prepared(epoch, result));
            });
            return;
        }
        if self.queue.is_none() {
            return;
        }
        if self.snapshot.wants_running
            && self.snapshot.prefs.source == Source::EntireGallery
            && !self.refresh
            && !self.refresh_active
            && self.refresh_attempts < 3
            && self.refresh_retry_at.is_none_or(|at| now >= at)
        {
            self.refresh = true;
            self.refresh_active = true;
            let tx = tx.clone();
            let epoch = self.epoch;
            let cancel = self.cancel.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Event::Catalogue(
                    epoch,
                    rotation_catalogue::collect_with(
                        catalogue::fetch_page_info,
                        &cancel,
                        &Default::default(),
                    ),
                ));
            });
        }
        if !(self.snapshot.wants_running || self.snapshot.state == State::Preparing)
            || self.retry_at.is_some_and(|at| now < at)
        {
            return;
        }
        if self.snapshot.prepared_path.is_some() {
            return;
        }
        let q = self.queue.as_mut().unwrap();
        if q.begin_gallery_cycle(
            self.snapshot.current.as_ref().map(|a| a.id.as_str()),
            rotation::random_seed(),
        ) {
            self.cache.reset_cycle();
            self.snapshot.skipped.clear();
            if !self.refresh_active {
                self.refresh = false;
                self.refresh_attempts = 0;
            }
        }
        let Some(art) = q.next().cloned() else {
            self.snapshot.state = State::Waiting;
            return;
        };
        self.snapshot.next = Some(art.clone());
        self.snapshot.pending = true;
        self.work = true;
        let epoch = self.epoch;
        let tx = tx.clone();
        let effects = self.effects.clone();
        std::thread::spawn(move || {
            let result = effects.download(&art);
            let _ = tx.send(Event::Downloaded(epoch, art.id, result));
        });
    }
    fn completed(&mut self, event: Event, now: SystemTime) -> Result<()> {
        match event {
            Event::Prepared(epoch, result) => {
                self.work = false;
                if epoch != self.epoch {
                    return Ok(());
                }
                self.snapshot.pending = false;
                match result {
                    Ok((prefs, queue, path, bootstrap, skipped)) => {
                        let running = self.setup.as_ref().is_some_and(|(_, running)| *running);
                        rotation::save(&self.prefs_path, &prefs)?;
                        self.snapshot.prefs = prefs;
                        self.snapshot.eligible = Some(known_eligible(&queue, &skipped));
                        self.snapshot.next = queue.next().cloned();
                        self.snapshot.prepared_path = Some(path);
                        self.queue = Some(queue);
                        self.setup = None;
                        self.snapshot.wants_running = running;
                        self.snapshot.state = if running {
                            if self.apply_first {
                                State::Waiting
                            } else {
                                State::Running
                            }
                        } else {
                            State::Paused
                        };
                        self.change_now = running && self.apply_first;
                        self.snapshot.skipped = skipped;
                        self.snapshot.deadline = if running && !self.apply_first {
                            Some(now + self.interval())
                        } else {
                            None
                        };
                        if let Some(b) = bootstrap
                            && b.complete
                        {
                            self.cache.inventory(b.artworks)?;
                            self.save_cache();
                        }
                    }
                    Err(e) => {
                        self.snapshot.state = State::Failed;
                        self.snapshot.pending = true;
                        self.snapshot.error = Some(format!(
                            "Preparation failed: {e:#}. Saved configuration and wallpaper were kept."
                        ));
                        self.snapshot.wants_running = false;
                    }
                }
            }
            Event::Downloaded(epoch, id, result) => {
                self.work = false;
                if epoch != self.epoch {
                    return Ok(());
                }
                if self.snapshot.next.as_ref().is_none_or(|a| a.id != id) {
                    return Ok(());
                }
                self.snapshot.pending = false;
                match result {
                    Ok(path) => {
                        self.snapshot.prepared_path = Some(path);
                        self.retry_at = None;
                        self.attempts = 0;
                        self.snapshot.error = None;
                    }
                    Err(e) => {
                        self.attempts += 1;
                        self.snapshot.error = Some(format!("Next original unavailable: {e:#}"));
                        if self.snapshot.prefs.source == Source::EntireGallery
                            && permanent_original_error(&e)
                        {
                            if let Some(art) = self.snapshot.next.take() {
                                self.snapshot.skipped.push((art, format!("{e:#}")));
                            }
                            self.queue.as_mut().unwrap().advance();
                            self.attempts = 0;
                            let q = self.queue.as_ref().unwrap();
                            let eligible = known_eligible(q, &self.snapshot.skipped);
                            self.snapshot.eligible = Some(eligible);
                            if q.complete() && eligible < 2 {
                                self.snapshot.state = State::Failed;
                                self.snapshot.wants_running = false;
                                self.snapshot.deadline = None;
                                self.snapshot.error=Some("Fewer than two known eligible gallery originals remain. Retry a skipped original.".into());
                                return self.persist();
                            }
                        }
                        if self.attempts >= 3 {
                            self.snapshot.state = State::Failed;
                            self.snapshot.wants_running = false;
                            self.snapshot.deadline = None;
                            self.snapshot.error = Some(format!(
                                "Next original unavailable after three attempts: {e:#}. Retry when connected."
                            ));
                        } else {
                            self.retry_at =
                                Some(now + Duration::from_secs(30 * self.attempts.max(1) as u64));
                        }
                    }
                }
            }
            Event::Catalogue(epoch, result) => {
                self.refresh_active = false;
                // One refresh per engine generation; cancellation does not start concurrent refreshes.
                if epoch != self.epoch {
                    self.refresh = false;
                    return Ok(());
                }
                match result {
                    Ok(items) => {
                        if let Some(q) = &mut self.queue {
                            q.adopt_complete(items.clone(), rotation::random_seed())?;
                            let eligible = known_eligible(q, &self.snapshot.skipped);
                            self.snapshot.eligible = Some(eligible);
                            if q.complete() && eligible < 2 {
                                self.snapshot.state = State::Failed;
                                self.snapshot.wants_running = false;
                                self.snapshot.deadline = None;
                                self.snapshot.error=Some("Fewer than two known eligible gallery originals remain. Retry a skipped original.".into());
                            }
                        }
                        self.cache.inventory(items)?;
                        self.save_cache();
                    }
                    Err(e) => {
                        self.snapshot.cache_diagnostic = Some(format!("Catalogue refresh: {e:#}"));
                        self.refresh = false;
                        self.refresh_attempts += 1;
                        self.refresh_retry_at =
                            Some(now + Duration::from_secs(60 * self.refresh_attempts as u64));
                    }
                }
            }
            Event::Command(..) => unreachable!(),
        }
        self.persist()
    }
    fn apply_due(&mut self, now: SystemTime) -> Result<()> {
        if self.setup.is_some()
            || !self.snapshot.wants_running
            || !(self.change_now || self.snapshot.deadline.is_some_and(|d| now >= d))
        {
            return Ok(());
        }
        ensure!(
            self.queue.is_some(),
            "No active queue for wallpaper application"
        );
        let q = self.queue.as_ref().unwrap();
        if q.source == Source::EntireGallery && known_eligible(q, &self.snapshot.skipped) < 2 {
            ensure!(
                !q.complete(),
                "Fewer than two known eligible gallery originals remain"
            );
            self.snapshot.state = State::Waiting;
            return Ok(());
        }
        let Some(path) = self.snapshot.prepared_path.clone() else {
            self.snapshot.state = State::Waiting;
            return Ok(());
        };
        let lock = wallpaper_lock(
            &self
                .path
                .parent()
                .context("Runtime path")?
                .join("service/wallpaper.lock"),
        )?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        self.snapshot.state = State::Applying;
        self.snapshot.deadline = None;
        match self.effects.apply(&path) {
            Ok(()) => {
                self.snapshot.current = self.snapshot.next.take();
                self.snapshot.current_path = Some(path);
                self.snapshot.prepared_path = None;
                if let Some(art) = &self.snapshot.current {
                    self.cache.shown(&art.id);
                    self.save_cache();
                }
                self.queue.as_mut().context("No active queue")?.advance();
                self.snapshot.state = State::Running;
                self.snapshot.deadline = Some(now + self.interval());
                self.snapshot.error = None;
                self.change_now = false;
                self.apply_first = false;
            }
            Err(e) => {
                self.snapshot.state = State::Failed;
                self.snapshot.wants_running = false;
                self.snapshot.error = Some(format!(
                    "Wallpaper change failed: {e:#}. Retry keeps the same original."
                ));
                self.change_now = false;
            }
        }
        self.persist()
    }
}
#[cfg(unix)]
fn bind_owned(path: &Path) -> Result<(File, UnixListener)> {
    let parent = path.parent().context("Socket path")?;
    fs::create_dir_all(parent)?;
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join("owner.lock"))?;
    lock.try_lock()
        .context("Another rotation service owns this session")?;
    if path.exists() {
        ensure!(
            UnixStream::connect(path).is_err(),
            "A rotation service is already listening"
        );
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok((lock, listener))
}
#[cfg(unix)]
pub fn run() -> Result<()> {
    let path = socket_path()?;
    let (_lock, listener) = bind_owned(&path)?;
    let mut engine = Engine::load(
        runtime_path()?,
        rotation::default_path()?,
        rotation_catalogue::default_path()?,
        Arc::new(RealEffects),
    )?;
    let (tx, rx) = mpsc::channel();
    let clients = Arc::new(AtomicUsize::new(0));
    let accept_tx = tx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            if clients.fetch_add(1, Ordering::Relaxed) >= 32 {
                clients.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let tx = accept_tx.clone();
            let clients = clients.clone();
            std::thread::spawn(move || {
                let _ = (|| -> Result<()> {
                    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                    let command = read_frame(&mut stream)?;
                    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
                    tx.send(Event::Command(command, reply_tx))?;
                    let reply = reply_rx.recv_timeout(Duration::from_secs(4))?;
                    write_frame(&mut stream, &reply)
                })();
                clients.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
    loop {
        if let Ok(event) = rx.recv_timeout(Duration::from_millis(100)) {
            handle_event(&mut engine, event);
        }
        // Pause/stop already accepted by the socket always win before a platform mutation.
        while let Ok(event) = rx.try_recv() {
            handle_event(&mut engine, event);
        }
        let now = SystemTime::now();
        if let Err(e) = engine.apply_due(now) {
            engine.snapshot.error = Some(format!("Rotation persistence failed: {e:#}"));
            engine.snapshot.wants_running = false;
            engine.snapshot.state = State::Failed;
        }
        engine.launch(&tx, now);
    }
}
fn handle_event(engine: &mut Engine, event: Event) {
    let now = SystemTime::now();
    if let Event::Command(command, tx) = event {
        let result = engine.command(command, now);
        if result.is_err() {
            // Mutation may have failed after changing memory; do not apply without durable intent.
            engine.snapshot.wants_running = false;
            engine.snapshot.deadline = None;
            engine.snapshot.state = State::Failed;
            let _ = engine.persist();
        }
        let reply = Reply {
            snapshot: Some(engine.snapshot.clone()),
            error: result.err().map(|e| format!("{e:#}")),
        };
        let _ = tx.send(reply);
    } else if let Err(e) = engine.completed(event, now) {
        engine.snapshot.error = Some(format!("Rotation service failed: {e:#}"));
        engine.snapshot.wants_running = false;
        engine.snapshot.state = State::Failed;
    }
}
#[cfg(not(unix))]
pub fn run() -> Result<()> {
    anyhow::bail!("Background rotation requires macOS or Linux")
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Mock {
        applies: AtomicUsize,
        fail: AtomicBool,
    }
    impl Effects for Mock {
        fn download(&self, _: &Artwork) -> Result<PathBuf> {
            anyhow::bail!("offline")
        }
        fn apply(&self, _: &Path) -> Result<()> {
            self.applies.fetch_add(1, Ordering::Relaxed);
            ensure!(!self.fail.load(Ordering::Relaxed), "mock platform failure");
            Ok(())
        }
    }
    fn art(id: &str) -> Artwork {
        Artwork {
            id: id.into(),
            key: format!("originals/Artist - {id}.jpg"),
            alt: id.into(),
            href: format!("/artwork/{id}"),
        }
    }
    fn mock() -> Arc<Mock> {
        Arc::new(Mock {
            applies: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        })
    }
    fn load(dir: &Path, effects: Arc<Mock>) -> Engine {
        Engine::load(
            dir.join("rotation-runtime.json"),
            dir.join("rotation.json"),
            dir.join("rotation-catalogue.json"),
            effects,
        )
        .unwrap()
    }
    fn ready(engine: &mut Engine, dir: &Path, now: SystemTime) {
        let prefs = Preferences {
            source: Source::Selected,
            selected: vec![art("a"), art("b"), art("c")],
            configured: true,
            ..Preferences::default()
        };
        fs::create_dir_all(dir.join("originals")).unwrap();
        let path = dir.join("originals/mock.jpg");
        fs::write(&path, b"fixture").unwrap();
        engine.snapshot.prefs = prefs.clone();
        engine.queue = Some(Queue::new(Source::Selected, prefs.selected.clone(), None, 0).unwrap());
        engine.snapshot.next = Some(art("a"));
        engine.snapshot.prepared_path = Some(path);
        engine.snapshot.wants_running = true;
        engine.snapshot.deadline = Some(now + Duration::from_secs(100));
        engine.snapshot.state = State::Running;
        rotation::save(&engine.prefs_path, &engine.snapshot.prefs).unwrap();
        engine.persist().unwrap();
    }
    #[test]
    fn close_reconnect_and_restart_keep_deadline_queue_then_overdue_changes_once() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        engine
            .command(Command::Status, now + Duration::from_secs(50))
            .unwrap();
        let deadline = engine.snapshot.deadline;
        drop(engine);
        let mut restored = load(dir.path(), effects.clone());
        assert_eq!(restored.snapshot.deadline, deadline);
        assert!(restored.snapshot.wants_running);
        assert_eq!(restored.queue.as_ref().unwrap().next().unwrap().id, "a");
        let overdue = now + Duration::from_secs(10000);
        restored.apply_due(overdue).unwrap();
        restored.apply_due(overdue).unwrap();
        assert_eq!(effects.applies.load(Ordering::Relaxed), 1);
        assert_eq!(restored.queue.as_ref().unwrap().next().unwrap().id, "b");
        assert_eq!(
            restored.snapshot.deadline,
            Some(overdue + restored.interval())
        );
        drop(restored);
        let restored = load(dir.path(), effects);
        assert_eq!(restored.snapshot.current.unwrap().id, "a");
        assert_eq!(restored.queue.unwrap().next().unwrap().id, "b");
    }
    #[test]
    fn pause_ack_invalidates_download_and_prevents_apply() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        engine.work = true;
        let epoch = engine.epoch;
        engine.command(Command::Pause, now).unwrap();
        engine
            .completed(
                Event::Downloaded(epoch, "a".into(), Ok(dir.path().join("untrusted"))),
                now,
            )
            .unwrap();
        engine.apply_due(now + Duration::from_secs(10000)).unwrap();
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
        assert!(!engine.snapshot.wants_running);
        assert!(!engine.work);
        assert_eq!(
            engine.snapshot.prepared_path.unwrap(),
            dir.path().join("originals/mock.jpg")
        );
    }
    #[test]
    fn apply_failure_persists_paused_same_candidate_retry() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        effects.fail.store(true, Ordering::Relaxed);
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        engine.command(Command::ChangeNow, now).unwrap();
        engine.apply_due(now).unwrap();
        assert_eq!(engine.snapshot.state, State::Failed);
        drop(engine);
        let mut restored = load(dir.path(), effects.clone());
        assert!(!restored.snapshot.wants_running);
        assert_eq!(restored.snapshot.next.as_ref().unwrap().id, "a");
        assert_eq!(restored.queue.as_ref().unwrap().next().unwrap().id, "a");
        effects.fail.store(false, Ordering::Relaxed);
        restored.command(Command::Retry, now).unwrap();
        restored.apply_due(now).unwrap();
        assert_eq!(restored.snapshot.current.unwrap().id, "a");
    }
    #[test]
    fn transactional_failure_keeps_saved_configuration_and_single_collection_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects);
        ready(&mut engine, dir.path(), now);
        rotation::save(&engine.prefs_path, &engine.snapshot.prefs).unwrap();
        let prefs = Preferences {
            source: Source::EntireGallery,
            configured: true,
            ..Preferences::default()
        };
        engine.command(Command::Save { prefs }, now).unwrap();
        let epoch = engine.epoch;
        engine
            .completed(Event::Prepared(epoch, Err(anyhow::anyhow!("offline"))), now)
            .unwrap();
        assert_eq!(
            rotation::load(&engine.prefs_path).unwrap().source,
            Source::Selected
        );
        assert_eq!(engine.snapshot.prefs.source, Source::Selected);
        assert!(engine.snapshot.current.is_none());
        assert!(engine.snapshot.prepared_path.is_some());
        let prefs = Preferences {
            selected: vec![art("a")],
            ..Preferences::default()
        };
        engine.command(Command::Save { prefs }, now).unwrap();
        assert!(!engine.snapshot.prefs.configured);
        assert_eq!(engine.snapshot.state, State::Stopped);
        assert!(!engine.snapshot.pending);
    }
    #[test]
    fn corrupt_runtime_path_queue_and_version_cannot_restore_active_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&engine.path).unwrap()).unwrap();
        for field in ["path", "queue", "version"] {
            let mut value = original.clone();
            match field {
                "path" => value["snapshot"]["prepared_path"] = serde_json::json!("/etc/passwd"),
                "queue" => value["queue"]["index"] = serde_json::json!(999),
                _ => value["version"] = serde_json::json!(99),
            }
            fs::write(&engine.path, serde_json::to_vec(&value).unwrap()).unwrap();
            let restored = load(dir.path(), effects.clone());
            assert!(!restored.snapshot.wants_running);
            assert!(restored.snapshot.error.is_some());
        }
    }
    #[cfg(unix)]
    #[test]
    fn ownership_lock_rejects_second_owner_without_unlinking_live_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("service/rotation.sock");
        let (lock, listener) = bind_owned(&path).unwrap();
        assert!(bind_owned(&path).is_err());
        assert!(UnixStream::connect(&path).is_ok());
        drop(listener);
        drop(lock);
        let (_lock, _listener) = bind_owned(&path).unwrap();
        assert!(UnixStream::connect(&path).is_ok());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn held_manual_lock_defers_apply_and_pause_suppresses_after_release() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        let lock = wallpaper_lock(&dir.path().join("service/wallpaper.lock")).unwrap();
        lock.lock().unwrap();
        engine.command(Command::ChangeNow, now).unwrap();
        engine.apply_due(now).unwrap();
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
        engine.command(Command::Pause, now).unwrap();
        drop(lock);
        engine.apply_due(now).unwrap();
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn failed_first_setup_retry_retains_candidate_and_save_does_not_start_network_when_paused() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects);
        let prefs = Preferences {
            source: Source::Selected,
            selected: vec![art("a"), art("b")],
            configured: true,
            ..Preferences::default()
        };
        engine.command(Command::Start { prefs }, now).unwrap();
        let epoch = engine.epoch;
        engine
            .completed(Event::Prepared(epoch, Err(anyhow::anyhow!("offline"))), now)
            .unwrap();
        assert!(!engine.snapshot.prefs.configured);
        assert!(engine.snapshot.pending);
        engine.command(Command::Retry, now).unwrap();
        assert_eq!(engine.setup.as_ref().unwrap().0.source, Source::Selected);
        engine.command(Command::Pause, now).unwrap();
        let (tx, _rx) = mpsc::channel();
        engine.launch(&tx, now);
        assert!(!engine.work);
        assert!(!engine.refresh_active);
    }
    #[test]
    fn network_failures_are_bounded_without_consuming_gallery_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects);
        ready(&mut engine, dir.path(), now);
        engine.snapshot.prefs.source = Source::EntireGallery;
        engine.queue =
            Some(Queue::new(Source::EntireGallery, vec![art("a"), art("b")], None, 0).unwrap());
        let next = engine.queue.as_ref().unwrap().next().unwrap().clone();
        engine.snapshot.next = Some(next.clone());
        engine.snapshot.prepared_path = None;
        for _ in 0..3 {
            engine.work = true;
            engine
                .completed(
                    Event::Downloaded(
                        engine.epoch,
                        next.id.clone(),
                        Err(anyhow::anyhow!("offline")),
                    ),
                    now,
                )
                .unwrap();
        }
        assert!(!engine.snapshot.wants_running);
        assert_eq!(engine.queue.as_ref().unwrap().next().unwrap().id, next.id);
        assert!(engine.snapshot.skipped.is_empty());
    }
    #[test]
    fn pending_and_failed_setup_restore_candidate_and_initial_start_intent() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let effects = mock();
        let mut engine = load(dir.path(), effects.clone());
        let prefs = Preferences {
            source: Source::Selected,
            selected: vec![art("a"), art("b")],
            configured: true,
            ..Preferences::default()
        };
        engine.command(Command::Start { prefs }, now).unwrap();
        drop(engine);
        let mut restored = load(dir.path(), effects.clone());
        assert_eq!(restored.snapshot.state, State::Preparing);
        assert!(restored.snapshot.pending);
        assert!(restored.start_first && restored.apply_first);
        assert!(restored.setup.as_ref().unwrap().1);
        let epoch = restored.epoch;
        restored
            .completed(Event::Prepared(epoch, Err(anyhow::anyhow!("offline"))), now)
            .unwrap();
        drop(restored);
        let mut restored = load(dir.path(), effects);
        assert_eq!(restored.snapshot.state, State::Failed);
        assert!(restored.snapshot.pending);
        restored.command(Command::Retry, now).unwrap();
        assert_eq!(restored.setup.unwrap().0.selected[0].id, "a");
    }
    #[test]
    fn active_interval_save_keeps_exact_queue_original_and_fresh_deadline_without_apply() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let effects = mock();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        let queue = serde_json::to_value(&engine.queue).unwrap();
        let path = engine.snapshot.prepared_path.clone();
        let mut prefs = engine.snapshot.prefs.clone();
        prefs.minutes = 5;
        engine.command(Command::Save { prefs }, now).unwrap();
        engine.apply_due(now).unwrap();
        assert_eq!(serde_json::to_value(&engine.queue).unwrap(), queue);
        assert_eq!(engine.snapshot.prepared_path, path);
        assert!(engine.snapshot.wants_running);
        assert!(engine.setup.is_none());
        assert_eq!(
            engine.snapshot.deadline,
            Some(now + Duration::from_secs(300))
        );
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn mismatched_preference_files_keep_new_preferences_and_pause() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let effects = mock();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        let mut prefs = engine.snapshot.prefs.clone();
        prefs.minutes = 7;
        rotation::save(&engine.prefs_path, &prefs).unwrap();
        drop(engine);
        let restored = load(dir.path(), effects);
        assert_eq!(restored.snapshot.prefs.minutes, 7);
        assert!(!restored.snapshot.wants_running);
        assert!(restored.queue.is_none());
        assert!(restored.snapshot.error.is_some());
    }
    #[test]
    fn active_candidate_without_queue_is_rejected_before_platform_effect() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let effects = mock();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&engine.path).unwrap()).unwrap();
        value["queue"] = serde_json::Value::Null;
        fs::write(&engine.path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut restored = load(dir.path(), effects.clone());
        restored
            .apply_due(now + Duration::from_secs(10000))
            .unwrap();
        assert!(!restored.snapshot.wants_running);
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn collection_save_prepares_continuity_and_resumes_countdown_without_immediate_apply() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let effects = mock();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        engine.snapshot.current = Some(art("a"));
        let mut prefs = engine.snapshot.prefs.clone();
        prefs.selected.push(art("d"));
        engine
            .command(
                Command::Save {
                    prefs: prefs.clone(),
                },
                now,
            )
            .unwrap();
        assert!(!engine.start_first && !engine.apply_first);
        let queue = Queue::new(Source::Selected, prefs.selected.clone(), Some("a"), 0).unwrap();
        let path = dir.path().join("originals/mock.jpg");
        engine
            .completed(
                Event::Prepared(engine.epoch, Ok((prefs, queue, path, None, vec![]))),
                now,
            )
            .unwrap();
        engine.apply_due(now).unwrap();
        assert_eq!(engine.snapshot.state, State::Running);
        assert_eq!(engine.snapshot.next.as_ref().unwrap().id, "b");
        assert_eq!(engine.snapshot.deadline, Some(now + engine.interval()));
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
    }
    struct GalleryFixture;
    impl Effects for GalleryFixture {
        fn download(&self, art: &Artwork) -> Result<PathBuf> {
            if art.id == "a" || art.id == "b" {
                return Err(anyhow::Error::new(ureq::Error::Status(
                    404,
                    ureq::Response::new(404, "Not Found", "").unwrap(),
                )));
            }
            Ok("/fixture/original.jpg".into())
        }
        fn apply(&self, _: &Path) -> Result<()> {
            panic!("preparation must not apply")
        }
        fn fetch_page(&self, page: usize) -> Result<catalogue::Page> {
            Ok(catalogue::Page {
                artworks: match page {
                    1 => vec![art("a"), art("b")],
                    2 => vec![art("c"), art("d")],
                    _ => vec![],
                },
                last_page: Some(2),
            })
        }
    }
    #[test]
    fn two_item_complete_gallery_with_one_missing_original_cannot_start_or_continue() {
        let mut queue =
            Queue::new(Source::EntireGallery, vec![art("a"), art("c")], None, 0).unwrap();
        queue.retry_skipped(art("a")).unwrap();
        assert!(
            prepare_original(
                &mut queue,
                &mut None,
                &AtomicBool::new(false),
                &GalleryFixture
            )
            .is_err()
        );
        let dir = tempfile::tempdir().unwrap();
        let effects = mock();
        let now = SystemTime::now();
        let mut engine = load(dir.path(), effects.clone());
        ready(&mut engine, dir.path(), now);
        engine.snapshot.prefs.source = Source::EntireGallery;
        engine.queue =
            Some(Queue::new(Source::EntireGallery, vec![art("a"), art("c")], None, 0).unwrap());
        engine
            .queue
            .as_mut()
            .unwrap()
            .retry_skipped(art("a"))
            .unwrap();
        engine.snapshot.next = Some(art("a"));
        engine.snapshot.prepared_path = None;
        let error = GalleryFixture.download(&art("a")).unwrap_err();
        engine
            .completed(Event::Downloaded(engine.epoch, "a".into(), Err(error)), now)
            .unwrap();
        assert!(!engine.snapshot.wants_running);
        assert_eq!(engine.snapshot.eligible, Some(1));
        engine.apply_due(now + Duration::from_secs(10000)).unwrap();
        assert_eq!(effects.applies.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn first_gallery_404_skips_expand_partial_pool_using_cached_metadata() {
        let mut queue = Queue::gallery(
            vec![art("a"), art("b")],
            &Default::default(),
            None,
            0,
            false,
            Some("a"),
        )
        .unwrap();
        let mut bootstrap = Some(rotation_catalogue::Bootstrap {
            artworks: vec![art("a"), art("b")],
            complete: false,
            pages: [(1, GalleryFixture.fetch_page(1).unwrap())]
                .into_iter()
                .collect(),
        });
        let (_path, skipped) = prepare_original(
            &mut queue,
            &mut bootstrap,
            &AtomicBool::new(false),
            &GalleryFixture,
        )
        .unwrap();
        assert_eq!(skipped.len(), 2);
        assert!(queue.complete());
        assert!(bootstrap.unwrap().complete);
        assert!(matches!(queue.next().unwrap().id.as_str(), "c" | "d"));
    }
    #[test]
    fn framing_is_bounded_and_rejects_unknown_commands() {
        let mut bytes = (MAX_BYTES as u32 + 1).to_be_bytes().to_vec();
        assert!(read_frame::<Command>(&mut bytes.as_slice()).is_err());
        bytes.clear();
        write_frame(&mut bytes, &Command::Status).unwrap();
        assert!(matches!(
            read_frame::<Command>(&mut bytes.as_slice()).unwrap(),
            Command::Status
        ));
    }
}
