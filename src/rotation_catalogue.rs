//! Fast first-wallpaper discovery, complete catalogue snapshots, and local cycle history.
use crate::{
    catalogue::{self, Artwork, Page},
    rotation::{MAX_ARTWORKS, Preferences, Source},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    pub version: u32,
    pub complete: bool,
    pub artworks: Vec<Artwork>,
    pub seen: Vec<String>,
    pub last: Option<String>,
}
impl Cache {
    pub fn empty() -> Self {
        Self {
            version: 1,
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported catalogue cache version");
        ensure!(
            self.complete || self.artworks.is_empty(),
            "Partial inventory cannot be cached as a complete catalogue"
        );
        if self.complete {
            ensure!(self.artworks.len() >= 2, "Incomplete catalogue cache");
        }
        Preferences {
            source: Source::EntireGallery,
            selected: self.artworks.clone(),
            ..Preferences::default()
        }
        .validate()?;
        ensure!(
            self.seen.len() <= MAX_ARTWORKS,
            "Seen history exceeds the safe limit"
        );
        let mut ids = HashSet::new();
        for id in &self.seen {
            ensure!(
                !id.is_empty() && id.len() <= 512 && ids.insert(id),
                "Invalid seen history"
            );
        }
        if let Some(last) = &self.last {
            ensure!(
                !last.is_empty() && last.len() <= 512,
                "Invalid last artwork"
            );
        }
        Ok(())
    }
    pub fn seen_set(&self) -> HashSet<String> {
        self.seen.iter().cloned().collect()
    }
    pub fn shown(&mut self, id: &str) {
        if !self.seen.iter().any(|seen| seen == id) {
            self.seen.push(id.to_owned());
        }
        self.last = Some(id.to_owned());
    }
    pub fn reset_cycle(&mut self) {
        self.seen.clear();
    }
    pub fn inventory(&mut self, artworks: Vec<Artwork>) -> Result<()> {
        let mut replacement = self.clone();
        replacement.complete = true;
        replacement.artworks = artworks;
        replacement.validate()?;
        *self = replacement;
        Ok(())
    }
}
pub fn default_path() -> Result<PathBuf> {
    Ok(crate::storage::data_dir()?.join("rotation-catalogue.json"))
}
pub fn load(path: &Path) -> Result<Cache> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Cache::empty()),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = vec![];
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Catalogue cache exceeds the safe size limit"
    );
    let cache: Cache = serde_json::from_slice(&bytes).context("Invalid catalogue cache")?;
    cache.validate()?;
    Ok(cache)
}
pub fn save(path: &Path, cache: &Cache) -> Result<()> {
    cache.validate()?;
    let bytes = serde_json::to_vec(cache)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Catalogue cache exceeds the safe size limit"
    );
    let parent = path.parent().context("Invalid catalogue cache path")?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(&bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn check(cancel: &AtomicBool) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "Catalogue discovery cancelled"
    );
    Ok(())
}
fn valid_artwork(art: &Artwork) -> bool {
    catalogue::valid_artwork(art)
        && art.id.len() <= 512
        && art.key.len() <= 4096
        && art.alt.len() <= 4096
        && art.href.len() <= 4096
        && crate::download::validate_url(&art.original_url()).is_ok()
}
fn fetch_cached(
    page: usize,
    fetch: &mut impl FnMut(usize) -> Result<Page>,
    pages: &mut BTreeMap<usize, Page>,
    cancel: &AtomicBool,
) -> Result<Page> {
    check(cancel)?;
    if let Some(value) = pages.get(&page) {
        return Ok(value.clone());
    }
    let value = fetch(page)?;
    check(cancel)?;
    ensure!(
        value.artworks.len() <= MAX_ARTWORKS
            && value.artworks.iter().all(valid_artwork)
            && value
                .last_page
                .is_none_or(|page| (1..=1000).contains(&page)),
        "Unsafe catalogue page"
    );
    pages.insert(page, value.clone());
    Ok(value)
}
/// Map a global artwork position using the actual page size and short final page.
pub fn position(
    slot: usize,
    page_size: usize,
    last_page: usize,
    last_size: usize,
) -> Result<(usize, usize)> {
    ensure!(
        page_size > 0 && (1..=1000).contains(&last_page) && last_size > 0 && last_size <= page_size,
        "Invalid pagination layout"
    );
    let total = (last_page - 1)
        .checked_mul(page_size)
        .and_then(|n| n.checked_add(last_size))
        .context("Pagination size overflow")?;
    ensure!(
        total <= MAX_ARTWORKS && slot < total,
        "Catalogue position exceeds the safe range"
    );
    Ok((slot / page_size + 1, slot % page_size))
}
#[derive(Debug)]
pub struct Bootstrap {
    pub artworks: Vec<Artwork>,
    pub complete: bool,
    pub pages: BTreeMap<usize, Page>,
}
fn candidate_first(
    items: &mut [Artwork],
    candidate: &str,
    seen: &HashSet<String>,
    last: Option<&str>,
) {
    let index = items
        .iter()
        .position(|a| a.id == candidate && !seen.contains(&a.id) && Some(a.id.as_str()) != last)
        .or_else(|| {
            items
                .iter()
                .position(|a| !seen.contains(&a.id) && Some(a.id.as_str()) != last)
        });
    if let Some(index) = index {
        items.swap(0, index);
    }
}
fn dedup(items: impl IntoIterator<Item = Artwork>) -> Vec<Artwork> {
    let mut ids = HashSet::new();
    items
        .into_iter()
        .filter(|a| ids.insert(a.id.clone()))
        .collect()
}
/// About three metadata requests on a stable paginated site, then prepare the original.
/// If pagination is unknown or changes, use a bounded complete enumeration instead.
pub fn bootstrap(
    mut fetch: impl FnMut(usize) -> Result<Page>,
    cancel: &AtomicBool,
    seed: u64,
    seen: &HashSet<String>,
    last: Option<&str>,
) -> Result<Bootstrap> {
    let mut pages = BTreeMap::new();
    let first = fetch_cached(1, &mut fetch, &mut pages, cancel)?;
    let last_page = first.last_page.unwrap_or(1);
    let discovered = (|| -> Result<Option<Bootstrap>> {
        if last_page == 1 {
            let second = fetch_cached(2, &mut fetch, &mut pages, cancel)?;
            if !second.artworks.is_empty() {
                return Ok(None);
            }
            ensure!(
                first.artworks.len() >= 2,
                "At least two artworks are required"
            );
            let mut artworks = first.artworks.clone();
            let candidate = artworks[(seed % artworks.len() as u64) as usize].id.clone();
            candidate_first(&mut artworks, &candidate, seen, last);
            return Ok(Some(Bootstrap {
                artworks,
                complete: true,
                pages: pages.clone(),
            }));
        }
        let last_info = fetch_cached(last_page, &mut fetch, &mut pages, cancel)?;
        if last_info.artworks.is_empty() || last_info.last_page.is_some_and(|n| n > last_page) {
            return Ok(None);
        }
        let size = first.artworks.len();
        let last_size = last_info.artworks.len();
        if size == 0 || last_size > size {
            return Ok(None);
        }
        let total = (last_page - 1)
            .checked_mul(size)
            .and_then(|n| n.checked_add(last_size))
            .context("Pagination size overflow")?;
        ensure!(
            (2..=MAX_ARTWORKS).contains(&total),
            "Catalogue exceeds the safe artwork limit"
        );
        let (selected_page, selected_slot) =
            position((seed % total as u64) as usize, size, last_page, last_size)?;
        let selected = fetch_cached(selected_page, &mut fetch, &mut pages, cancel)?;
        let expected = if selected_page == last_page {
            last_size
        } else {
            size
        };
        if selected.artworks.len() != expected || selected.last_page.is_some_and(|n| n > last_page)
        {
            return Ok(None);
        }
        let candidate = selected.artworks[selected_slot].id.clone();
        let mut artworks = dedup(pages.values().flat_map(|p| p.artworks.clone()));
        ensure!(
            artworks.len() >= 2,
            "At least two distinct artworks are required"
        );
        candidate_first(&mut artworks, &candidate, seen, last);
        // A pool containing only already-seen artwork cannot establish cycle exhaustion.
        if !artworks
            .iter()
            .any(|a| !seen.contains(&a.id) && Some(a.id.as_str()) != last)
        {
            return Ok(None);
        }
        Ok(Some(Bootstrap {
            artworks,
            complete: false,
            pages: pages.clone(),
        }))
    })()?;
    if let Some(result) = discovered {
        return Ok(result);
    }
    let mut artworks = collect_with(&mut fetch, cancel, &pages)?;
    let candidate = artworks[(seed % artworks.len() as u64) as usize].id.clone();
    candidate_first(&mut artworks, &candidate, seen, last);
    Ok(Bootstrap {
        artworks,
        complete: true,
        pages,
    })
}
/// Reuse pages obtained during discovery but require an explicit end marker before
/// returning a complete snapshot. No partial/error result is eligible for persistence.
pub fn collect_with(
    mut fetch: impl FnMut(usize) -> Result<Page>,
    cancel: &AtomicBool,
    initial: &BTreeMap<usize, Page>,
) -> Result<Vec<Artwork>> {
    let mut items = vec![];
    let mut ids = HashSet::new();
    for page in 1..=1000 {
        check(cancel)?;
        let value = match initial.get(&page) {
            Some(value) => value.clone(),
            None => fetch(page)?,
        };
        check(cancel)?;
        ensure!(
            value.artworks.len() <= MAX_ARTWORKS
                && value.artworks.iter().all(valid_artwork)
                && value
                    .last_page
                    .is_none_or(|page| (1..=1000).contains(&page)),
            "Unsafe catalogue page"
        );
        if value.artworks.is_empty() {
            ensure!(
                items.len() >= 2,
                "At least two distinct artworks are required"
            );
            return Ok(items);
        }
        for art in value.artworks {
            if ids.insert(art.id.clone()) {
                items.push(art);
            }
            ensure!(
                items.len() <= MAX_ARTWORKS,
                "Catalogue exceeds the safe artwork limit"
            );
        }
    }
    anyhow::bail!("Catalogue exceeds the safe page limit")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn art(id: usize) -> Artwork {
        Artwork {
            id: id.to_string(),
            key: format!("originals/Artist - {id}.jpg"),
            alt: id.to_string(),
            href: format!("/artwork/{id}"),
        }
    }
    fn fixture(page: usize) -> Result<Page> {
        Ok(Page {
            artworks: match page {
                1 => (0..4).map(art).collect(),
                2 => (4..8).map(art).collect(),
                3 => (8..10).map(art).collect(),
                _ => vec![],
            },
            last_page: Some(3),
        })
    }
    #[test]
    fn weighted_slots_include_short_final_page_without_biasing_page_selection() {
        let mut counts = [0; 3];
        for slot in 0..10 {
            let (page, index) = position(slot, 4, 3, 2).unwrap();
            counts[page - 1] += 1;
            let result = bootstrap(
                fixture,
                &AtomicBool::new(false),
                slot as u64,
                &HashSet::new(),
                None,
            )
            .unwrap();
            assert_eq!(result.artworks[0].id, slot.to_string());
            assert!(!result.complete);
            assert_eq!(result.pages.len(), if page == 2 { 3 } else { 2 });
            assert_eq!(fixture(page).unwrap().artworks[index].id, slot.to_string());
        }
        assert_eq!(counts, [4, 4, 2]);
        assert!(position(10, 4, 3, 2).is_err());
        assert!(position(0, 0, 3, 2).is_err());
        assert!(position(0, 10000, 1000, 3).is_err());
    }
    #[test]
    fn stable_discovery_fetches_first_last_and_selected_once_and_rejects_bad_hints() {
        let mut requests = vec![];
        let result = bootstrap(
            |page| {
                requests.push(page);
                fixture(page)
            },
            &AtomicBool::new(false),
            5,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert_eq!(requests, [1, 3, 2]);
        assert_eq!(result.artworks[0].id, "5");
        assert!(
            bootstrap(
                |page| {
                    let mut p = fixture(page)?;
                    p.last_page = Some(0);
                    Ok(p)
                },
                &AtomicBool::new(false),
                0,
                &HashSet::new(),
                None
            )
            .is_err()
        );
        assert!(
            collect_with(
                |page| {
                    let mut p = fixture(page)?;
                    p.artworks[0].alt = "x".repeat(4097);
                    Ok(p)
                },
                &AtomicBool::new(false),
                &BTreeMap::new()
            )
            .is_err()
        );
    }
    #[test]
    fn complete_collection_reuses_discovery_pages_and_requires_end() {
        let initial =
            bootstrap(fixture, &AtomicBool::new(false), 5, &HashSet::new(), None).unwrap();
        let mut requests = vec![];
        let result = collect_with(
            |page| {
                requests.push(page);
                fixture(page)
            },
            &AtomicBool::new(false),
            &initial.pages,
        )
        .unwrap();
        assert_eq!(result.len(), 10);
        assert_eq!(requests, [4]);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rotation-catalogue.json");
        let mut cache = Cache::empty();
        cache.inventory(result).unwrap();
        save(&path, &cache).unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(
            collect_with(
                |page| if page == 2 {
                    anyhow::bail!("offline")
                } else {
                    fixture(page)
                },
                &AtomicBool::new(false),
                &BTreeMap::new()
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[test]
    fn changing_layout_falls_back_and_missing_navigation_confirms_single_page() {
        let result = bootstrap(
            |page| {
                let mut p = fixture(page)?;
                if page == 2 {
                    p.artworks.pop();
                }
                Ok(p)
            },
            &AtomicBool::new(false),
            5,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert!(result.complete);
        assert_eq!(result.artworks.len(), 9);
        let result = bootstrap(
            |page| {
                Ok(Page {
                    artworks: if page == 1 {
                        (0..3).map(art).collect()
                    } else {
                        vec![]
                    },
                    last_page: None,
                })
            },
            &AtomicBool::new(false),
            1,
            &HashSet::new(),
            None,
        )
        .unwrap();
        assert!(result.complete);
        assert_eq!(result.artworks[0].id, "1");
    }
    #[test]
    fn cancellation_prevents_more_requests_and_cache_commit() {
        let cancel = AtomicBool::new(false);
        let mut requests = 0;
        assert!(
            bootstrap(
                |page| {
                    requests += 1;
                    cancel.store(true, Ordering::Relaxed);
                    fixture(page)
                },
                &cancel,
                0,
                &HashSet::new(),
                None
            )
            .is_err()
        );
        assert_eq!(requests, 1);
        assert!(bootstrap(|_| panic!("cancelled"), &cancel, 0, &HashSet::new(), None).is_err());
    }
    #[test]
    fn history_only_persists_before_full_inventory_and_restarts_without_seen() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rotation-catalogue.json");
        let mut cache = Cache::empty();
        cache.shown("2");
        save(&path, &cache).unwrap();
        let mut restored = load(&path).unwrap();
        assert!(!restored.complete);
        assert_eq!(restored.last.as_deref(), Some("2"));
        assert!(restored.seen_set().contains("2"));
        restored.inventory((0..4).map(art).collect()).unwrap();
        save(&path, &restored).unwrap();
        let q = crate::rotation::Queue::gallery(
            restored.artworks.clone(),
            &restored.seen_set(),
            restored.last.as_deref(),
            0,
            true,
            None,
        )
        .unwrap();
        assert_ne!(q.next().unwrap().id, "2");
        restored.reset_cycle();
        assert_eq!(restored.last.as_deref(), Some("2"));
        assert!(restored.seen.is_empty());
        for raw in [
            "broken",
            r#"{"version":2,"complete":false,"artworks":[],"seen":[],"last":null}"#,
        ] {
            std::fs::write(&path, raw).unwrap();
            assert!(load(&path).is_err());
        }
        restored.complete = false;
        assert!(save(&path, &restored).is_err());
        restored = Cache::empty();
        restored.artworks = vec![art(1)];
        assert!(restored.validate().is_err());
        restored = Cache::empty();
        restored.complete = true;
        restored.artworks = (0..2).map(art).collect();
        restored.artworks[0].key = "originals/../bad.jpg".into();
        assert!(save(&path, &restored).is_err());
        std::fs::write(&path, vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(load(&path).is_err());
    }
    #[test]
    fn unseen_partial_candidate_unavailable_expands_without_resetting_seen_history() {
        let seen = HashSet::from(["0".to_owned(), "1".to_owned()]);
        let mut queue = crate::rotation::Queue::gallery(
            (0..3).map(art).collect(),
            &seen,
            Some("1"),
            0,
            false,
            Some("2"),
        )
        .unwrap();
        assert_eq!(queue.next().unwrap().id, "2");
        queue.advance(); // Original 2 was unavailable, not a successful display.
        assert!(queue.next().is_none());
        assert!(!queue.begin_gallery_cycle(Some("1"), 0));
        queue.adopt_complete((0..5).map(art).collect(), 0).unwrap();
        let next = queue.next().unwrap().id.clone();
        assert!(next == "3" || next == "4");
        queue.advance();
        assert!(queue.next().is_some());
        assert_eq!(seen.len(), 2); // Expansion never resets persisted history.
    }
    #[test]
    fn adopting_full_snapshot_preserves_next_and_consumed_and_stages_refresh() {
        let seen = HashSet::from(["0".to_owned()]);
        let mut q = crate::rotation::Queue::gallery(
            (0..3).map(art).collect(),
            &seen,
            Some("0"),
            0,
            false,
            Some("1"),
        )
        .unwrap();
        assert_eq!(q.next().unwrap().id, "1");
        q.adopt_complete((0..5).map(art).collect(), 4).unwrap();
        assert_eq!(q.next().unwrap().id, "1");
        q.advance();
        q.adopt_complete((0..6).map(art).collect(), 5).unwrap();
        let mut played = HashSet::new();
        while let Some(a) = q.next() {
            assert_ne!(a.id, "0");
            assert_ne!(a.id, "1");
            assert!(played.insert(a.id.clone()));
            q.advance();
        }
        assert!(!played.contains("5"));
        assert!(q.begin_gallery_cycle(Some("4"), 9));
        assert_eq!(q.len(), 6);
        assert_ne!(q.next().unwrap().id, "4");
        let mut partial = crate::rotation::Queue::gallery(
            (0..2).map(art).collect(),
            &HashSet::new(),
            None,
            0,
            false,
            None,
        )
        .unwrap();
        partial.advance();
        partial.advance();
        assert!(!partial.begin_gallery_cycle(None, 0));
        assert!(partial.next().is_none());
    }
}
