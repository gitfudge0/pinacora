//! Local rotation preferences and deterministic scheduling. No platform calls occur here.
use crate::catalogue::{self, Artwork};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    hash::{BuildHasher, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ARTWORKS: usize = 10_000;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    EntireGallery,
    Selected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub source: Source,
    pub minutes: u16,
    pub selected: Vec<Artwork>,
    pub configured: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            source: Source::EntireGallery,
            minutes: 30,
            selected: vec![],
            configured: false,
        }
    }
}
pub fn minutes(value: &str) -> Result<u16> {
    ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "Enter a whole number from 1 to 1,440."
    );
    let n: u16 = value
        .parse()
        .map_err(|_| anyhow::anyhow!("Enter a whole number from 1 to 1,440."))?;
    ensure!(
        (1..=1440).contains(&n),
        "Enter a whole number from 1 to 1,440."
    );
    Ok(n)
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported rotation preferences version"
        );
        minutes(&self.minutes.to_string())?;
        ensure!(
            self.selected.len() <= MAX_ARTWORKS,
            "Too many saved artworks"
        );
        let mut ids = HashSet::new();
        for art in &self.selected {
            ensure!(
                catalogue::valid_artwork(art)
                    && art.id.len() <= 512
                    && art.key.len() <= 4096
                    && art.alt.len() <= 4096
                    && art.href.len() <= 4096,
                "Unsafe or incomplete saved artwork"
            );
            crate::download::validate_url(&art.original_url())?;
            ensure!(ids.insert(&art.id), "Duplicate saved artwork");
        }
        if self.configured && self.source == Source::Selected {
            ensure!(
                self.selected.len() >= 2,
                "Select at least two distinct artworks."
            );
        }
        Ok(())
    }
}
pub fn default_path() -> Result<PathBuf> {
    Ok(crate::storage::data_dir()?.join("rotation.json"))
}
pub fn load(path: &Path) -> Result<Preferences> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = vec![];
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Rotation preferences exceed the size limit"
    );
    let prefs: Preferences =
        serde_json::from_slice(&bytes).context("Invalid rotation preferences")?;
    prefs.validate()?;
    Ok(prefs)
}
pub fn save(path: &Path, prefs: &Preferences) -> Result<()> {
    prefs.validate()?;
    let data = serde_json::to_vec(prefs)?;
    ensure!(
        data.len() <= MAX_BYTES,
        "Rotation preferences exceed the size limit"
    );
    let parent = path.parent().context("Invalid preferences path")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&data)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Fetch every page until the server confirms an empty page or a later-page 404.
/// Duplicate-only pages are not an end marker. Limits fail explicitly.
pub fn collect_with(
    mut fetch: impl FnMut(usize) -> Result<Vec<Artwork>>,
    cancelled: &AtomicBool,
) -> Result<Vec<Artwork>> {
    let mut items = vec![];
    let mut ids = HashSet::new();
    for page in 1..=1000 {
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "Catalogue preparation cancelled"
        );
        let batch = fetch(page)?;
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "Catalogue preparation cancelled"
        );
        if batch.is_empty() {
            ensure!(
                items.len() >= 2,
                "The full gallery needs at least two eligible artworks."
            );
            return Ok(items);
        }
        for art in batch {
            ensure!(catalogue::valid_artwork(&art), "Unsafe catalogue artwork");
            if ids.insert(art.id.clone()) {
                items.push(art);
            }
            ensure!(
                items.len() <= MAX_ARTWORKS,
                "Full gallery exceeds the safe artwork limit; rotation was not started"
            );
        }
    }
    bail!("Full gallery exceeds the safe page limit; rotation was not started")
}
pub fn collect(cancelled: Arc<AtomicBool>) -> Result<Vec<Artwork>> {
    collect_with(catalogue::fetch_page, &cancelled)
}
pub fn random_seed() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}
fn shuffle(items: &mut [Artwork], mut seed: u64) {
    for i in (1..items.len()).rev() {
        seed = seed.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        items.swap(i, (z % (i as u64 + 1)) as usize);
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    pub source: Source,
    items: Vec<Artwork>,
    remaining: VecDeque<Artwork>,
    index: usize,
    complete: bool,
    consumed: HashSet<String>,
    future: Option<Vec<Artwork>>,
}
impl Queue {
    /// Validate a persisted queue before it can supply platform operations.
    pub fn validate(&self) -> Result<()> {
        for items in [
            &self.items,
            &self.remaining.iter().cloned().collect::<Vec<_>>(),
        ] {
            Preferences {
                selected: items.clone(),
                ..Preferences::default()
            }
            .validate()?;
        }
        ensure!(
            self.items.len() >= 2 && self.index < self.items.len(),
            "Invalid queue position"
        );
        ensure!(
            self.consumed.len() <= MAX_ARTWORKS
                && self
                    .consumed
                    .iter()
                    .all(|id| !id.is_empty() && id.len() <= 512),
            "Invalid queue history"
        );
        ensure!(
            self.remaining
                .iter()
                .all(|art| self.items.iter().any(|item| item.id == art.id)),
            "Invalid queued artwork"
        );
        if let Some(items) = &self.future {
            Preferences {
                selected: items.clone(),
                ..Preferences::default()
            }
            .validate()?;
            ensure!(items.len() >= 2, "Invalid future catalogue");
        }
        Ok(())
    }
    pub fn new(
        source: Source,
        items: Vec<Artwork>,
        current: Option<&str>,
        seed: u64,
    ) -> Result<Self> {
        let mut ids = HashSet::new();
        let items: Vec<_> = items
            .into_iter()
            .filter(|a| ids.insert(a.id.clone()))
            .collect();
        ensure!(
            items.len() >= 2,
            "Rotation needs at least two distinct artworks."
        );
        let mut queue = Self {
            source,
            items,
            remaining: VecDeque::new(),
            index: 0,
            complete: true,
            consumed: HashSet::new(),
            future: None,
        };
        if source == Source::EntireGallery {
            queue.next_cycle(current, seed);
        } else if let Some(i) = queue
            .items
            .iter()
            .position(|a| Some(a.id.as_str()) == current)
        {
            queue.index = (i + 1) % queue.items.len();
        }
        Ok(queue)
    }
    pub fn gallery(
        items: Vec<Artwork>,
        seen: &HashSet<String>,
        current: Option<&str>,
        seed: u64,
        complete: bool,
        candidate: Option<&str>,
    ) -> Result<Self> {
        let mut queue = Self::new(Source::EntireGallery, items, current, seed)?;
        queue.complete = complete;
        queue.consumed = seen.clone();
        queue.remaining.retain(|art| !seen.contains(&art.id));
        if let Some(candidate) = candidate
            && let Some(index) = queue.remaining.iter().position(|art| art.id == candidate)
            && let Some(art) = queue.remaining.remove(index)
        {
            queue.remaining.push_front(art);
        }
        Ok(queue)
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    /// A cold partial pool is extended once complete metadata arrives. Keep the next
    /// target and existing unconsumed order, then append unseen additions. A refresh
    /// of an already complete pool is staged for the next cycle instead.
    pub fn adopt_complete(&mut self, items: Vec<Artwork>, seed: u64) -> Result<()> {
        let validated = Self::new(Source::EntireGallery, items, None, seed)?;
        if self.complete {
            self.future = Some(validated.items);
            return Ok(());
        }
        let protected = self.remaining.front().cloned();
        let mut replacement = validated.items;
        if let Some(protected) = &protected
            && !replacement.iter().any(|art| art.id == protected.id)
        {
            replacement.push(protected.clone());
        }
        anyhow::ensure!(
            replacement.len() <= MAX_ARTWORKS,
            "Catalogue exceeds the safe artwork limit"
        );
        let eligible: HashSet<_> = replacement.iter().map(|art| art.id.clone()).collect();
        self.remaining
            .retain(|art| eligible.contains(&art.id) && !self.consumed.contains(&art.id));
        let queued: HashSet<_> = self.remaining.iter().map(|art| art.id.clone()).collect();
        let mut additions: Vec<_> = replacement
            .iter()
            .filter(|art| !self.consumed.contains(&art.id) && !queued.contains(&art.id))
            .cloned()
            .collect();
        shuffle(&mut additions, seed);
        self.remaining.extend(additions);
        self.items = replacement;
        self.complete = true;
        Ok(())
    }
    pub fn begin_gallery_cycle(&mut self, current: Option<&str>, seed: u64) -> bool {
        if self.source != Source::EntireGallery || !self.complete || !self.remaining.is_empty() {
            return false;
        }
        if let Some(items) = self.future.take() {
            self.items = items;
        }
        self.consumed.clear();
        self.next_cycle(current, seed);
        true
    }
    pub fn matches_selected(&self, items: &[Artwork]) -> bool {
        self.source == Source::Selected
            && self
                .items
                .iter()
                .map(|a| &a.id)
                .eq(items.iter().map(|a| &a.id))
    }
    pub fn eligible_count(&self, unavailable: &HashSet<String>) -> usize {
        self.items
            .iter()
            .filter(|art| !unavailable.contains(&art.id))
            .count()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn next(&self) -> Option<&Artwork> {
        if self.source == Source::Selected {
            self.items.get(self.index)
        } else {
            self.remaining.front()
        }
    }
    pub fn retry_skipped(&mut self, art: Artwork) -> Result<()> {
        ensure!(
            self.source == Source::EntireGallery,
            "Skipped originals belong to full-gallery rotation"
        );
        ensure!(
            self.items.iter().any(|a| a.id == art.id),
            "Artwork is no longer in the catalogue"
        );
        self.remaining.retain(|a| a.id != art.id);
        self.consumed.remove(&art.id);
        self.remaining.push_front(art);
        Ok(())
    }
    pub fn advance(&mut self) {
        if self.source == Source::Selected {
            self.index = (self.index + 1) % self.items.len();
        } else if let Some(art) = self.remaining.pop_front() {
            self.consumed.insert(art.id);
        }
    }
    pub fn next_cycle(&mut self, current: Option<&str>, seed: u64) {
        let mut items = self.items.clone();
        shuffle(&mut items, seed);
        if items
            .first()
            .is_some_and(|a| Some(a.id.as_str()) == current)
        {
            items.swap(0, 1);
        }
        self.remaining = items.into();
    }
    /// Refresh only at a cycle boundary; successes/skips in the old cycle stay consumed.
    pub fn refresh_cycle(
        &mut self,
        items: Vec<Artwork>,
        current: Option<&str>,
        seed: u64,
    ) -> Result<()> {
        ensure!(
            self.remaining.is_empty(),
            "Cannot restart an unfinished gallery cycle"
        );
        *self = Self::new(self.source, items, current, seed)?;
        Ok(())
    }
}
/// A cancelled epoch still owns its blocking request until the callback returns.
/// This prevents rapid pause/resume from accumulating concurrent HTTP requests.
#[derive(Debug, Default)]
pub struct Flight {
    ticket: Option<u64>,
}
impl Flight {
    pub fn active(&self) -> bool {
        self.ticket.is_some()
    }
    pub fn begin(&mut self, epoch: u64) -> bool {
        if self.active() {
            return false;
        }
        self.ticket = Some(epoch);
        true
    }
    pub fn complete(&mut self, epoch: u64) -> bool {
        if self.ticket != Some(epoch) {
            return false;
        }
        self.ticket = None;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum State {
    Stopped,
    Paused,
    Preparing,
    Running,
    Waiting,
    Applying,
    Failed,
}
#[derive(Debug)]
pub struct Schedule {
    pub state: State,
    pub epoch: u64,
    pub deadline: Option<SystemTime>,
    pub wants_running: bool,
    pub interval: Duration,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            state: State::Stopped,
            epoch: 0,
            deadline: None,
            wants_running: false,
            interval: Duration::from_secs(1800),
        }
    }
}
impl Schedule {
    pub fn invalidate(&mut self, state: State) -> u64 {
        self.epoch += 1;
        self.state = state;
        self.deadline = None;
        self.wants_running = false;
        self.epoch
    }
    pub fn valid(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }
    pub fn prepare(&mut self, running: bool) -> u64 {
        let epoch = self.invalidate(State::Preparing);
        self.wants_running = running;
        epoch
    }
    pub fn resume(&mut self, now: SystemTime) {
        self.epoch += 1;
        self.wants_running = true;
        self.state = State::Running;
        self.deadline = Some(now + self.interval);
    }
    pub fn pause(&mut self) {
        self.invalidate(State::Paused);
    }
    pub fn due(&self, now: SystemTime) -> bool {
        self.wants_running && self.deadline.is_some_and(|d| now >= d)
    }
    pub fn begin_apply(&mut self, epoch: u64) -> bool {
        if !self.valid(epoch) || !self.wants_running || self.state == State::Applying {
            return false;
        }
        self.state = State::Applying;
        self.deadline = None;
        true
    }
    pub fn applied(&mut self, epoch: u64, now: SystemTime, success: bool) -> bool {
        if !self.valid(epoch) || !self.wants_running {
            return false;
        }
        if success {
            self.state = State::Running;
            self.deadline = Some(now + self.interval);
        } else {
            self.pause();
            self.state = State::Failed;
        }
        true
    }
    pub fn remaining(&self, now: SystemTime) -> Option<Duration> {
        self.deadline
            .map(|d| d.duration_since(now).unwrap_or_default())
    }
    pub fn save_interval(&mut self, minutes: u16, now: SystemTime) {
        self.interval = Duration::from_secs(minutes as u64 * 60);
        if self.wants_running && self.state != State::Applying {
            self.deadline = Some(now + self.interval);
            self.state = State::Running;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn art(id: &str) -> Artwork {
        Artwork {
            id: id.into(),
            key: format!("originals/Artist - {id}.jpg"),
            alt: id.into(),
            href: format!("/artwork/{id}"),
        }
    }
    fn items() -> Vec<Artwork> {
        ["a", "b", "c", "d"].map(art).to_vec()
    }
    #[test]
    fn interval_rejects_invalid_values() {
        for value in ["", "0", "1441", "-1", "1.5", " 30", "30 ", "abc", "65536"] {
            assert!(minutes(value).is_err(), "{value}");
        }
        assert_eq!(minutes("1").unwrap(), 1);
        assert_eq!(minutes("1440").unwrap(), 1440);
    }
    #[test]
    fn preferences_are_atomic_bounded_and_validated() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rotation.json");
        assert!(!load(&path).unwrap().configured);
        let mut prefs = Preferences {
            selected: items(),
            configured: true,
            ..Preferences::default()
        };
        save(&path, &prefs).unwrap();
        assert_eq!(load(&path).unwrap().selected.len(), 4);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        prefs.selected[0].key = "originals/../bad.jpg".into();
        assert!(save(&path, &prefs).is_err());
        assert_eq!(load(&path).unwrap().selected.len(), 4);
        for raw in [
            "{}",
            "broken",
            r#"{"version":2,"source":"entire_gallery","minutes":30,"selected":[],"configured":true}"#,
        ] {
            std::fs::write(&path, raw).unwrap();
            assert!(load(&path).is_err());
        }
        std::fs::write(&path, vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(load(&path).is_err());
        prefs = Preferences::default();
        prefs.selected = vec![art("a"), art("a")];
        assert!(prefs.validate().is_err());
        prefs.selected[1] = art("b");
        prefs.selected[1].href = "https://evil.example/art".into();
        assert!(prefs.validate().is_err());
    }
    #[test]
    fn catalogue_deduplicates_without_premature_end_and_cancels() {
        let cancelled = AtomicBool::new(false);
        let mut requests = vec![];
        let result = collect_with(
            |page| {
                requests.push(page);
                Ok(match page {
                    1 | 2 => vec![art("a")],
                    3 => vec![art("b")],
                    _ => vec![],
                })
            },
            &cancelled,
        )
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(requests, [1, 2, 3, 4]);
        assert!(
            collect_with(
                |_| {
                    cancelled.store(true, Ordering::Relaxed);
                    Ok(items())
                },
                &cancelled
            )
            .is_err()
        );
        assert!(collect_with(|_| panic!("cancelled before fetch"), &cancelled).is_err());
        assert!(collect_with(|_| Ok(vec![art("a")]), &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn shuffle_consumes_each_item_once_and_avoids_boundary_repeat() {
        for seed in 0..100 {
            let mut q = Queue::new(Source::EntireGallery, items(), Some("a"), seed).unwrap();
            assert_ne!(q.next().unwrap().id, "a");
            let mut seen = HashSet::new();
            let mut last = String::new();
            while let Some(a) = q.next() {
                last = a.id.clone();
                assert!(seen.insert(last.clone()));
                q.advance();
            }
            assert_eq!(seen.len(), 4);
            q.refresh_cycle(items(), Some(&last), seed + 1).unwrap();
            assert_ne!(q.next().unwrap().id, last);
        }
    }
    #[test]
    fn selected_order_wraps_and_refresh_does_not_restart_a_cycle() {
        let mut q = Queue::new(Source::Selected, items(), Some("b"), 0).unwrap();
        for id in ["c", "d", "a", "b", "c"] {
            assert_eq!(q.next().unwrap().id, id);
            q.advance();
        }
        let mut q = Queue::new(Source::EntireGallery, items(), None, 0).unwrap();
        assert!(q.refresh_cycle(items(), None, 1).is_err());
    }
    #[test]
    fn skipped_retry_keeps_full_gallery_and_active_selected_order_is_explicit() {
        let mut q = Queue::new(Source::EntireGallery, items(), None, 9).unwrap();
        let skipped = q.next().unwrap().clone();
        q.advance();
        let rest: Vec<_> = q.remaining.iter().map(|a| a.id.clone()).collect();
        q.retry_skipped(skipped.clone()).unwrap();
        assert_eq!(q.source, Source::EntireGallery);
        assert_eq!(q.next().unwrap().id, skipped.id);
        q.advance();
        assert_eq!(
            q.remaining.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
            rest
        );
        while q.next().is_some() {
            q.advance();
        }
        q.refresh_cycle(items(), None, 10).unwrap();
        assert_eq!(q.remaining.len(), 4);
        let q = Queue::new(Source::Selected, items(), None, 0).unwrap();
        assert!(q.matches_selected(&items()));
        let mut added = items();
        added.push(art("e"));
        assert!(!q.matches_selected(&added));
        added.swap(0, 1);
        assert!(!q.matches_selected(&added));
    }
    #[test]
    fn cancelled_request_keeps_physical_ownership_until_completion() {
        let mut schedule = Schedule::default();
        let first = schedule.prepare(true);
        let mut flight = Flight::default();
        assert!(flight.begin(first));
        for _ in 0..20 {
            schedule.pause();
            schedule.resume(SystemTime::UNIX_EPOCH);
            assert!(!flight.begin(schedule.epoch));
        }
        assert!(!schedule.valid(first));
        assert!(!flight.complete(schedule.epoch));
        assert!(flight.active());
        assert!(flight.complete(first));
        assert!(!flight.active());
        assert!(flight.begin(schedule.epoch));
    }

    #[test]
    fn overdue_only_applies_once_and_pause_invalidates_completions() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let mut s = Schedule::default();
        s.resume(now);
        let epoch = s.epoch;
        let late = now + Duration::from_secs(10000);
        assert!(s.due(late));
        assert!(s.begin_apply(epoch));
        assert!(!s.begin_apply(epoch));
        assert!(!s.due(late));
        assert!(s.applied(epoch, late, true));
        assert!(!s.due(late));
        assert_eq!(s.remaining(late), Some(s.interval));
        s.pause();
        assert!(!s.applied(epoch, late, true));
        assert!(!s.wants_running);
        let old = s.prepare(true);
        s.invalidate(State::Stopped);
        assert!(!s.valid(old));
    }
    #[test]
    fn interval_edit_and_resume_get_fresh_deadlines_failure_stays_paused() {
        let now = SystemTime::UNIX_EPOCH;
        let mut s = Schedule::default();
        s.save_interval(1, now);
        assert!(s.deadline.is_none());
        s.resume(now);
        let epoch = s.epoch;
        s.save_interval(5, now + Duration::from_secs(50));
        assert_eq!(
            s.remaining(now + Duration::from_secs(50)),
            Some(Duration::from_secs(300))
        );
        assert!(s.begin_apply(epoch));
        assert!(s.applied(epoch, now, false));
        assert_eq!(s.state, State::Failed);
        assert!(!s.wants_running);
        s.resume(now);
        assert_eq!(s.remaining(now), Some(Duration::from_secs(300)));
    }
}
