use crate::accessibility::{
    AccessibilityAction, AccessibilityBridge, AccessibilityNode, AccessibilityRole,
};
use crate::onboarding::{self, Outcome};
use crate::search_input::{Changed, Navigate, TextInput};
use gpui::{
    Animation, AnimationExt, AnyElement, BoxShadow, Context, Entity, FocusHandle, Focusable,
    FontWeight, KeyDownEvent, ObjectFit, ScrollHandle, SharedString, Window, WindowControlArea,
    div, img, linear_color_stop, linear_gradient, point, prelude::*, px, rgb, rgba,
};
use pinacora::catalogue::{Artwork, Detail};
use pinacora::rotation::{self, Source as RotationSource, State as RotationState};
use pinacora::rotation_service::{self, Command, Snapshot};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    time::{Duration, Instant},
};
fn app_icon(edge: f32) -> gpui::Img {
    static ICON: OnceLock<Arc<gpui::Image>> = OnceLock::new();
    img(ICON
        .get_or_init(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                include_bytes!("../resources/pinacora-mark.png").to_vec(),
            ))
        })
        .clone())
    .w(px(edge))
    .h(px(edge))
    .object_fit(ObjectFit::Contain)
}

struct HoverMotion {
    hovered: bool,
    epoch: u64,
    from: f32,
    started: Instant,
}

fn eased(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

fn hover_scale(from: f32, hovered: bool, progress: f32) -> f32 {
    let target = if hovered { 1.025 } else { 1. };
    from + (target - from) * progress.clamp(0., 1.)
}

fn reveal_progress(t: f32, delay: f32) -> f32 {
    eased((t - delay) / (1. - delay))
}

struct Rotation {
    snapshot: Snapshot,
    draft: Option<rotation::Preferences>,
    minutes: String,
    command_busy: bool,
    command_epoch: u64,
    scroll: ScrollHandle,
    reveal: Option<String>,
    details_open: bool,
}
impl std::ops::Deref for Rotation {
    type Target = Snapshot;
    fn deref(&self) -> &Snapshot {
        &self.snapshot
    }
}
impl std::ops::DerefMut for Rotation {
    fn deref_mut(&mut self) -> &mut Snapshot {
        &mut self.snapshot
    }
}
impl Rotation {
    fn new() -> Self {
        let result = rotation::default_path().and_then(|path| rotation::load(&path));
        let error = result
            .as_ref()
            .err()
            .map(|e| format!("Could not restore rotation settings: {e:#}"));
        let prefs = result.unwrap_or_default();
        let state = if prefs.configured {
            RotationState::Paused
        } else {
            RotationState::Stopped
        };
        Self {
            snapshot: Snapshot {
                prefs,
                state,
                wants_running: false,
                deadline: None,
                current: None,
                current_path: None,
                next: None,
                prepared_path: None,
                pending: false,
                error,
                notice: "Rotation continues after closing Pinacora. Pause or stop it here.".into(),
                skipped: vec![],
                eligible: None,
                cache_diagnostic: None,
            },
            draft: None,
            minutes: "30".into(),
            command_busy: false,
            command_epoch: 0,
            scroll: ScrollHandle::new(),
            reveal: None,
            details_open: false,
        }
    }
    fn unavailable(&mut self, error: String) {
        let report = self.prefs.configured || self.wants_running;
        self.state = if self.prefs.configured {
            RotationState::Paused
        } else {
            RotationState::Stopped
        };
        self.wants_running = false;
        self.deadline = None;
        self.pending = false;
        self.prepared_path = None;
        self.next = None;
        if report {
            self.error = Some(format!("Rotation service unavailable: {error}"));
        }
        self.notice = "Use Resume or Start rotation to reconnect the background service.".into();
    }
}
#[derive(Default)]
struct Updates {
    visible: bool,
    checking: bool,
    installing: bool,
    checked: bool,
    release: Option<pinacora::updater::Release>,
    error: Option<String>,
}
impl Updates {
    fn begin_check(&mut self) -> bool {
        if self.checking || self.installing {
            return false;
        }
        self.checking = true;
        self.error = None;
        true
    }
    fn checked(&mut self, result: Result<Option<pinacora::updater::Release>, String>) {
        self.checking = false;
        match result {
            Ok(release) => {
                self.release = release;
                self.checked = true;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}
pub struct Gallery {
    updates: Updates,
    updates_focus: FocusHandle,
    rotation: Rotation,
    rotation_input: Entity<TextInput>,
    ax: Rc<RefCell<Option<AccessibilityBridge>>>,
    ax_nodes: Rc<RefCell<Vec<AccessibilityNode>>>,
    ax_receiver: Option<async_channel::Receiver<AccessibilityAction>>,
    ax_sender: async_channel::Sender<AccessibilityAction>,
    artworks: Vec<Artwork>,
    search: SiteSearch,
    search_known: Vec<Artwork>,
    previews: HashMap<String, Result<PathBuf, String>>,
    selected: Option<String>,
    detail: Option<Detail>,
    page: usize,
    loading: bool,
    applying: bool,
    downloading: bool,
    status: String,
    query: String,
    focus: FocusHandle,
    preview_queue: VecDeque<Artwork>,
    preview_pending: HashSet<String>,
    preview_active: usize,
    search_input: Entity<TextInput>,
    catalogue_error: Option<String>,
    detail_error: Option<String>,
    detail_loading: bool,
    catalogue_notice: String,
    new_ids: Vec<String>,
    last_load_refresh: bool,
    controls: HashMap<String, FocusHandle>,
    tiles: HashMap<String, FocusHandle>,
    columns: usize,
    keyboard_tile: Option<String>,
    selected_result_preview: bool,
    results_offset: gpui::Point<gpui::Pixels>,
    has_results_return: bool,
    focus_results_after_layout: bool,
    navigation_subscribed: bool,
    rows_start: usize,
    metadata_scroll: ScrollHandle,
    end: bool,
    selection_epoch: u64,
    grid_scroll: ScrollHandle,
    hero_previews: HashMap<String, PathBuf>,
    hero_active: bool,
    motion_enabled: bool,
    tile_motion: HashMap<String, HoverMotion>,
    hover_epoch: u64,

    status_epoch: u64,
    hero_queued: Option<(Artwork, u64)>,
    intro_visible: bool,
    intro_focus: FocusHandle,
    intro_save_error: Option<String>,
}
impl Gallery {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let intro_visible = !onboarding::default_path()
            .and_then(|path| onboarding::dismissed(&path))
            .unwrap_or(false);
        let (ax_sender, ax_receiver) = async_channel::bounded(64);
        let search_input = cx.new(TextInput::new);
        let rotation_input = cx.new(TextInput::interval);
        cx.subscribe(&rotation_input, |app, _, event: &Changed, cx| {
            app.rotation.minutes = event.0.clone();
            if app.rotation.draft.is_some() {
                app.rotation.reveal = Some("rotation-interval".into());
            }
            cx.notify();
        })
        .detach();
        cx.observe(&rotation_input, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&search_input, |app, _, event: &Changed, cx| {
            app.query = event.0.clone();
            app.schedule_search(true, cx);
            app.selected_result_preview = false;
            app.has_results_return = false;
            app.focus_results_after_layout = app.query.trim().is_empty();
            app.grid_scroll.set_offset(point(px(0.), px(0.)));
            cx.notify();
        })
        .detach();
        cx.observe(&search_input, |_, _, cx| cx.notify()).detach();
        let controls = [
            "updates",
            "updates-check",
            "updates-install",
            "updates-close",
            "rotation",
            "rotation-add",
            "rotation-entire",
            "rotation-selected",
            "rotation-details",
            "rotation-close",
            "rotation-start",
            "rotation-save",
            "rotation-pause",
            "rotation-resume",
            "rotation-stop",
            "rotation-now",
            "rotation-retry",
            "rotation-stop-save",
            "back-to-results",
            "motion-toggle",
            "show-walkthrough",
            "refresh",
            "clear-search",
            "empty-clear-search",
            "retry-catalogue",
            "load-more",
            "show-new",
            "apply",
            "view-source",
            "retry-details",
            "retry-preview",
            "gallery-source",
            "intro-motion",
            "intro-start",
        ]
        .into_iter()
        .map(|id| (id.to_owned(), cx.focus_handle().tab_stop(true)))
        .collect();
        let mut app = Self {
            updates: Updates::default(),
            updates_focus: cx.focus_handle(),
            rotation: Rotation::new(),
            rotation_input,
            ax: Rc::new(RefCell::new(None)),
            ax_nodes: Rc::new(RefCell::new(vec![])),
            ax_receiver: Some(ax_receiver),
            ax_sender,
            artworks: vec![],
            search: SiteSearch::default(),
            search_known: vec![],
            previews: HashMap::new(),
            selected: None,
            detail: None,
            page: 0,
            loading: false,
            applying: false,
            downloading: false,
            status: String::new(),
            query: String::new(),
            focus: cx.focus_handle(),
            preview_queue: VecDeque::new(),
            preview_pending: HashSet::new(),
            preview_active: 0,
            search_input,
            catalogue_error: None,
            detail_error: None,
            detail_loading: false,
            catalogue_notice: String::new(),
            new_ids: vec![],
            last_load_refresh: true,
            controls,
            tiles: HashMap::new(),
            columns: 2,
            keyboard_tile: None,
            selected_result_preview: false,
            results_offset: point(px(0.), px(0.)),
            has_results_return: false,
            focus_results_after_layout: false,
            navigation_subscribed: false,
            rows_start: 3,
            metadata_scroll: ScrollHandle::new(),
            end: false,
            selection_epoch: 0,
            grid_scroll: ScrollHandle::new(),
            hero_previews: HashMap::new(),
            hero_active: false,
            motion_enabled: true,
            tile_motion: HashMap::new(),
            hover_epoch: 0,

            status_epoch: 0,
            hero_queued: None,
            intro_visible,
            intro_focus: cx.focus_handle(),
            intro_save_error: None,
        };
        app.updates_poll(cx);
        app.rotation_poll(cx);
        app.load(true, cx);
        app
    }
    fn show_intro(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying || self.rotation.draft.is_some() {
            return;
        }
        self.intro_visible = true;
        self.controls["intro-start"].focus(window);
        cx.notify();
    }
    fn close_intro(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        self.intro_save_error = onboarding::default_path()
            .and_then(|path| onboarding::save(&path, outcome))
            .err()
            .map(|e| format!("Walkthrough dismissed, but preferences could not be saved: {e}"));
        self.intro_visible = false;
        cx.notify();
    }
    fn intro_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "tab" => self.focus_step(event.keystroke.modifiers.shift, window, cx),
            "escape" => self.close_intro(Outcome::Skipped, cx),
            "enter" => self.close_intro(Outcome::Completed, cx),
            _ => {}
        }
    }
    fn toggle_motion(&mut self, cx: &mut Context<Self>) {
        self.motion_enabled = !self.motion_enabled;
        cx.notify();
    }
    fn schedule_search(&mut self, debounce: bool, cx: &mut Context<Self>) {
        let epoch = self.search.begin(&self.query);
        if !self.search.loading {
            cx.notify();
            return;
        }
        let timer = cx.background_executor().timer(if debounce {
            Duration::from_millis(250)
        } else {
            Duration::ZERO
        });
        cx.spawn(async move |this, cx| {
            timer.await;
            let query = this
                .update(cx, |app, _| {
                    app.search.current(epoch).then(|| app.search.query.clone())
                })
                .ok()
                .flatten();
            let Some(query) = query else {
                return;
            };
            let task = cx.background_executor().spawn(async move {
                pinacora::catalogue::fetch_search(&query)
                    .map_err(|e| format!("Could not search Reframed: {e:#}"))
            });
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if !app.search.complete(epoch, result) {
                    return;
                }
                append_unique(&mut app.search_known, app.search.results.clone());
                for art in &app.search.results {
                    app.tiles
                        .entry(art.id.clone())
                        .or_insert_with(|| cx.focus_handle());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn searching(&self) -> bool {
        !self.query.trim().is_empty()
    }
    fn all_artworks(&self) -> impl Iterator<Item = &Artwork> {
        self.artworks.iter().chain(self.search_known.iter())
    }
    fn visible_loading(&self) -> bool {
        if self.searching() {
            self.search.loading
        } else {
            self.loading
        }
    }
    fn visible_error(&self) -> Option<String> {
        if self.searching() {
            self.search.error.clone()
        } else {
            self.catalogue_error.clone()
        }
    }
    fn visible_notice(&self) -> String {
        if self.searching() {
            if self.search.query.chars().count() < 2 {
                "Enter at least 2 characters to search.".into()
            } else if self.search.loading || self.search.error.is_some() {
                String::new()
            } else {
                "Showing the site’s top matching artworks.".into()
            }
        } else {
            self.catalogue_notice.clone()
        }
    }
    fn load(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.loading || (refresh && self.applying) || (!refresh && self.end) {
            return;
        }
        self.loading = true;
        self.last_load_refresh = refresh;
        self.catalogue_error = None;
        let page = request_page(self.page, refresh);
        let search_epoch = self.search.epoch;
        let task = cx.background_executor().spawn(async move {
            pinacora::catalogue::fetch_page(page).map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.loading = false;
                match result {
                    Ok(items) => {
                        let confirmed_end = items.is_empty();
                        let old_selected = app.selected.clone();
                        if refresh {
                            app.artworks.clear();
                            if app.search.browse_can_update_view(search_epoch) {
                                app.grid_scroll.set_offset(point(px(0.), px(0.)));
                            }
                        }
                        let added = append_unique(&mut app.artworks, items);
                        app.page = page;
                        app.end = confirmed_end;
                        app.new_ids = if refresh {
                            vec![]
                        } else {
                            added.iter().map(|a| a.id.clone()).collect()
                        };
                        app.catalogue_notice = if refresh {
                            String::new()
                        } else if confirmed_end {
                            "You’ve reached the end of the catalogue".into()
                        } else if added.is_empty() {
                            "This page contained artwork already loaded. You can keep loading."
                                .into()
                        } else {
                            format!("{} more artworks loaded", added.len())
                        };
                        for art in added {
                            app.tiles
                                .entry(art.id.clone())
                                .or_insert_with(|| cx.focus_handle());
                        }
                        let retained = selection_retained(&app.artworks, old_selected.as_deref());
                        if !retained && app.search.browse_can_update_view(search_epoch) {
                            app.selected = None;
                            app.detail = None;
                            app.detail_error = None;
                            app.status.clear();
                            app.selection_epoch += 1;
                            if let Some(first) = app.artworks.first().cloned() {
                                app.select(first, cx);
                            }
                        }
                    }
                    Err(e) => app.catalogue_error = Some(format!("Could not load artwork: {e}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn preview(&mut self, art: Artwork, cx: &mut Context<Self>) {
        if self.previews.contains_key(&art.id) || !self.preview_pending.insert(art.id.clone()) {
            return;
        }
        self.preview_queue.push_back(art);
        self.pump_previews(cx);
    }
    fn pump_previews(&mut self, cx: &mut Context<Self>) {
        while self.preview_active < 4 {
            let Some(art) = self.preview_queue.pop_front() else {
                break;
            };
            self.preview_active += 1;
            let url = art.preview_url();
            let id = art.id;
            let task = cx.background_executor().spawn(async move {
                pinacora::download::fetch(&url, false)
                    .map(|(p, _, _)| p)
                    .map_err(|e| format!("{e:#}"))
            });
            cx.spawn(async move |this, cx| {
                let result = task.await;
                let _ = this.update(cx, |app, cx| {
                    app.preview_active -= 1;
                    app.preview_pending.remove(&id);
                    app.previews.insert(id, result);
                    app.pump_previews(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }
    fn pump_hero(&mut self, cx: &mut Context<Self>) {
        if self.hero_active {
            return;
        }
        let Some((art, epoch)) = self.hero_queued.take() else {
            return;
        };
        if self.selection_epoch != epoch || self.selected.as_ref() != Some(&art.id) {
            return;
        }
        self.hero_active = true;
        let url = art.hero_preview_url();
        let id = art.id;
        let task = cx
            .background_executor()
            .spawn(async move { pinacora::download::fetch(&url, false).map(|(path, _, _)| path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.hero_active = false;
                if app.selection_epoch == epoch
                    && app.selected.as_ref() == Some(&id)
                    && let Ok(path) = result
                {
                    app.hero_previews.insert(id, path);
                }
                // A failed large preview retains the grid preview; only the latest selection is queued.
                app.pump_hero(cx);
                cx.notify();
            });
        })
        .detach();
    }
    fn select(&mut self, art: Artwork, cx: &mut Context<Self>) {
        if self.applying {
            return;
        }
        self.selection_epoch += 1;

        let epoch = self.selection_epoch;
        self.selected = Some(art.id.clone());
        self.hero_queued = None;
        if !self.hero_previews.contains_key(&art.id) {
            self.hero_queued = Some((art.clone(), epoch));
            self.pump_hero(cx);
        }
        self.detail = None;
        self.detail_loading = true;
        self.detail_error = None;
        self.status.clear();
        self.preview(art.clone(), cx);
        let id = art.id.clone();
        let task = cx.background_executor().spawn(async move {
            pinacora::catalogue::fetch_detail(&art).map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.selected.as_ref() != Some(&id) || app.selection_epoch != epoch {
                    return;
                }
                app.detail_loading = false;
                match result {
                    Ok(detail) => {
                        app.detail = Some(detail);
                    }
                    Err(e) => app.detail_error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.applying
            || self.intro_visible
            || (!self.searching() && self.loading && self.last_load_refresh)
        {
            return;
        }
        let Some(detail) = self.detail.clone() else {
            return;
        };
        let Some(art) = self
            .all_artworks()
            .find(|a| Some(&a.id) == self.selected.as_ref())
            .cloned()
        else {
            return;
        };
        if self.rotation.command_busy {
            return;
        }
        self.rotation.command_epoch += 1;
        self.applying = true;
        self.downloading = true;
        self.status = "Downloading and validating the full resolution original…".into();
        let all_desktops = pinacora::platform::all_desktops_supported();
        let task = cx.background_executor().spawn(async move {
            let guard = rotation_service::manual_guard()
                .map_err(|e| format!("Could not reserve wallpaper operation: {e:#}"))?;
            let snapshot = pinacora::rotation_service_manager::ensure_running()
                .and_then(|_| rotation_service::request(Command::Pause))
                .map_err(|e| format!("Could not pause rotation; wallpaper was kept: {e:#}"))?;
            let downloaded = pinacora::download::fetch(&detail.original, true)
                .map_err(|e| format!("Could not download original: {e:#}"));
            Ok::<_, String>((guard, snapshot, downloaded))
        });
        cx.spawn(async move |this, cx| {
            let paused = task.await;
            let (downloaded, manual_guard) = match paused {
                Ok((guard, snapshot, downloaded)) => {
                    let _ = this.update(cx, |app, cx| { app.rotation.snapshot = snapshot; cx.notify(); });
                    (downloaded, Some(guard))
                }
                Err(error) => (Err(error), None),
            };
            let (path, w, h) = match downloaded {
                Ok(original) => original,
                Err(error) => {
                    let _ = this.update(cx, |app, cx| {
                        app.status = error;
                        app.applying = false;
                        app.downloading = false;
                        app.status_epoch += 1;
                        cx.notify();
                    });
                    return;
                }
            };
            if this.update(cx, |app, cx| {
                app.downloading = false;
                app.status = if all_desktops {
                    "Applying wallpaper to all Desktops…".into()
                } else {
                    "Applying wallpaper to connected displays…".into()
                };
                cx.notify();
            }).is_err() {
                return;
            }
            let manual_path = path.clone();
            let (mut status, applied) = if all_desktops {
                let task = cx.background_executor().spawn(async move {
                    pinacora::platform::apply_all_desktops(&path)
                        .map_err(|error| format!("{error:#}"))
                });
                match task.await {
                    Ok(()) => (format!("Wallpaper set on all Desktops · {w} × {h}"), true),
                    Err(error) => (format!("Could not apply wallpaper: {error}"), false),
                }
            } else if cfg!(target_os = "linux") {
                let task = cx.background_executor().spawn(async move {
                    pinacora::platform::apply(&path).map_err(|error| format!("{error:#}"))
                });
                match task.await {
                    Ok(n) => (format!("Wallpaper applied · {n} display setting(s) · {w} × {h}"), true),
                    Err(error) => (format!("Could not apply wallpaper: {error}"), false),
                }
            } else {
                // AppKit requires the foreground/main thread on macOS 12–13.
                match pinacora::platform::apply(&path) {
                    Ok(n) => (format!("Wallpaper set on {n} connected display(s) in the current Desktop · {w} × {h}"), true),
                    Err(error) => (format!("Could not apply wallpaper: {error:#}"), false),
                }
            };
            if applied {
                let task = cx.background_executor().spawn(async move {
                    rotation_service::request(Command::ManualWallpaper { art, path: manual_path }).map_err(|e| format!("{e:#}"))
                });
                match task.await {
                    Ok(snapshot) => { let _ = this.update(cx, |app, _| app.rotation.snapshot = snapshot); }
                    Err(error) => status.push_str(&format!(" · Wallpaper set, but service status could not sync: {error}")),
                }
            }
            drop(manual_guard);
            let _ = this.update(cx, |app, cx| {
                app.status = format!("{status} · Rotation paused");
                app.applying = false;
                app.status_epoch += 1;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn key(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.navigation_key(e, window, cx);
    }
    fn navigation_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.updates.visible {
            self.updates_key(e, window, cx);
            return matches!(e.keystroke.key.as_str(), "tab" | "escape");
        }
        if self.rotation.draft.is_some() {
            self.rotation_key(e, window, cx);
            return matches!(e.keystroke.key.as_str(), "tab" | "escape");
        }
        if self.intro_visible {
            return false;
        }
        if (if cfg!(target_os = "macos") {
            e.keystroke.modifiers.platform
        } else {
            e.keystroke.modifiers.control
        }) && e.keystroke.key == "f"
        {
            cx.stop_propagation();
            self.focus_results_after_layout = true;
            self.search_input.focus_handle(cx).focus(window);
            cx.notify();
            return true;
        }
        if e.keystroke.key == "tab" {
            cx.stop_propagation();
            self.focus_step(e.keystroke.modifiers.shift, window, cx);
            cx.notify();
            return true;
        }
        if e.keystroke.key == "escape"
            && self.has_results_return
            && !self.search_input.focus_handle(cx).is_focused(window)
        {
            cx.stop_propagation();
            self.back_to_results(window, cx);
            return true;
        }
        false
    }
    fn focus_step(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.visible {
            self.updates_focus_step(reverse, window, cx);
            return;
        }
        if self.rotation.draft.is_some() {
            self.rotation_focus_step(reverse, window, cx);
            return;
        }
        if self.intro_visible {
            let next = if self.controls["intro-start"].is_focused(window) {
                "intro-motion"
            } else {
                "intro-start"
            };
            self.controls[next].focus(window);
            cx.notify();
            return;
        }
        if reverse {
            window.focus_prev();
        } else {
            window.focus_next();
        }
        if let Some(index) = self
            .filtered()
            .iter()
            .position(|art| self.tiles[&art.id].is_focused(window))
        {
            self.reveal_row(index / self.columns + self.rows_start, false);
        }
        if self.controls["load-more"].is_focused(window) {
            let filtered = self.filtered();
            self.reveal_row(
                self.rows_start
                    + filtered.len().div_ceil(self.columns)
                    + usize::from(filtered.is_empty()),
                false,
            );
        }
        cx.notify();
    }
    fn open_result(&mut self, art: Artwork, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.visible
            || self.applying
            || self.rotation.draft.is_some()
            || self.intro_visible
        {
            return;
        }
        self.results_offset = self.grid_scroll.offset();
        self.has_results_return = true;
        self.selected_result_preview = true;
        self.grid_scroll.set_offset(point(px(0.), px(0.)));
        self.select(art, cx);
        let epoch = self.selection_epoch;
        let this = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = this.update(cx, |app, cx| {
                if app.rotation.draft.is_none()
                    && app.selection_epoch == epoch
                    && app.selected_result_preview
                {
                    app.controls["back-to-results"].focus(window);
                    cx.notify();
                }
            });
        });
    }
    fn back_to_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_result_preview = false;
        self.grid_scroll.set_offset(self.results_offset);
        if let Some(id) = self
            .selected
            .clone()
            .filter(|id| self.filtered().iter().any(|a| &a.id == id))
        {
            self.keyboard_tile = Some(id.clone());
            if let Some(handle) = self.tiles.get(&id) {
                handle.focus(window);
            }
        } else {
            self.search_input.focus_handle(cx).focus(window);
        }
        cx.notify();
    }
    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search_input.update(cx, |input, cx| input.clear(cx));
    }
    #[allow(clippy::too_many_arguments)] // Mirrors the semantic snapshot fields.
    fn semantic(
        &self,
        id: String,
        label: String,
        role: AccessibilityRole,
        value: String,
        enabled: bool,
        selected: bool,
        focused: bool,
    ) -> impl IntoElement {
        let nodes = self.ax_nodes.clone();
        gpui::canvas(
            move |bounds, _, _| {
                nodes.borrow_mut().push(AccessibilityNode {
                    id,
                    label,
                    role,
                    value,
                    enabled,
                    selected,
                    focused,
                    bounds,
                    selected_range: None,
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
    fn text(&self, id: &str, value: impl Into<String>) -> AnyElement {
        let value = value.into();
        div()
            .relative()
            .w_full()
            .min_h(px(20.))
            .text_sm()
            .child(value.clone())
            .child(self.semantic(
                id.into(),
                value,
                AccessibilityRole::StaticText,
                String::new(),
                true,
                false,
                false,
            ))
            .into_any_element()
    }
    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        enabled: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let label: SharedString = label.into();
        let ax_label = label.to_string();
        let primary = matches!(
            id,
            "apply" | "intro-start" | "rotation-start" | "rotation-save" | "updates-install"
        ) || (id == "rotation-resume" && !self.rotation_dirty());
        let source_button = matches!(id, "rotation-entire" | "rotation-selected");
        let source_selected = self.rotation.draft.as_ref().is_some_and(|draft| {
            (id == "rotation-entire" && draft.source == RotationSource::EntireGallery)
                || (id == "rotation-selected" && draft.source == RotationSource::Selected)
        });
        div()
            .relative()
            .id(id)
            .track_focus(
                &self.controls[id]
                    .clone()
                    .tab_stop(enabled && self.control_available(id))
                    .tab_index(control_tab_index(id)),
            )
            .tab_index(0)
            .tab_stop(enabled && self.control_available(id))
            .px_3()
            .py_2()
            .rounded(px(10.))
            .bg(rgb(SURFACE))
            .border_1()
            .border_color(rgb(0x3a3a40))
            .text_sm()
            .when(
                matches!(
                    id,
                    "motion-toggle"
                        | "show-walkthrough"
                        | "rotation"
                        | "intro-motion"
                        | "rotation-details"
                        | "rotation-stop"
                        | "rotation-close"
                ),
                |d| {
                    d.bg(rgba(0xffffff00))
                        .border_color(rgba(0xffffff00))
                        .text_xs()
                        .text_color(rgb(MUTED))
                },
            )
            .when(
                matches!(
                    id,
                    "motion-toggle"
                        | "show-walkthrough"
                        | "rotation"
                        | "refresh"
                        | "clear-search"
                        | "updates"
                ),
                |d| {
                    d.h(px(36.))
                        .py_0()
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(14.))
                        .line_height(px(24.))
                },
            )
            .when(source_button, |d| {
                d.flex_1()
                    .h(px(38.))
                    .py_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(8.))
                    .border_color(if source_selected {
                        rgb(0x45454d)
                    } else {
                        rgba(0xffffff00)
                    })
                    .bg(if source_selected {
                        rgb(0x323238)
                    } else {
                        rgba(0xffffff00)
                    })
                    .text_color(rgb(if source_selected { TEXT } else { MUTED }))
                    .font_weight(if source_selected {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
            })
            .when(primary, |d| {
                d.px_4()
                    .py_2()
                    .bg(rgb(ACCENT))
                    .border_color(rgb(ACCENT))
                    .text_color(rgb(TEXT))
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(enabled, |d| {
                d.cursor_pointer()
                    .hover(move |s| s.bg(rgb(if primary { 0x2994ff } else { RAISED })))
                    .active(move |s| s.bg(rgb(if primary { 0x0068d1 } else { 0x36363c })))
            })
            .opacity(if enabled { 1. } else { 0.4 })
            .focus(|s| s.border_color(rgb(ACCENT)))
            .child(label)
            .child(self.semantic(
                id.into(),
                ax_label,
                AccessibilityRole::Button,
                String::new(),
                enabled,
                source_selected,
                false,
            ))
            .on_click(cx.listener(move |app, _, window, cx| {
                if enabled && app.control_available(id) {
                    app.controls[id].focus(window);
                    app.activate(id, window, cx);
                }
            }))
            .on_key_down(cx.listener(move |app, e: &KeyDownEvent, window, cx| {
                if app.controls[id].is_focused(window) && app.navigation_key(e, window, cx) {
                    return;
                }
                if enabled
                    && !e.is_held
                    && app.controls[id].is_focused(window)
                    && matches!(e.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                    app.activate(id, window, cx);
                }
            }))
            .into_any_element()
    }
    fn control_available(&self, id: &str) -> bool {
        if self.updates.visible {
            id.starts_with("updates-")
        } else if id.starts_with("updates-") {
            false
        } else if self.intro_visible {
            id.starts_with("intro-")
        } else if self.rotation.draft.is_some() {
            id.starts_with("rotation-") && id != "rotation-add"
        } else {
            !id.starts_with("intro-") && (!id.starts_with("rotation-") || id == "rotation-add")
        }
    }
    fn activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.control_available(id) {
            return;
        }
        if id.starts_with("rotation-")
            && !matches!(id, "rotation-close" | "rotation-details")
            && self.rotation.command_busy
        {
            return;
        }
        if let Some(rest) = id.strip_prefix("rotation-remove-") {
            if let Ok(i) = rest.parse::<usize>()
                && !self.applying
                && !self.rotation.command_busy
                && let Some(d) = &mut self.rotation.draft
                && i < d.selected.len()
            {
                d.selected.remove(i);
                self.rotation_sync_controls(cx);
                self.controls["rotation-selected"].focus(window);
                cx.notify();
            }
            return;
        }
        for (prefix, up) in [("rotation-up-", true), ("rotation-down-", false)] {
            if let Some(rest) = id.strip_prefix(prefix) {
                if let Ok(i) = rest.parse::<usize>()
                    && !self.applying
                    && !self.rotation.command_busy
                    && let Some(d) = &mut self.rotation.draft
                {
                    let next = if up { i.saturating_sub(1) } else { i + 1 };
                    if i < d.selected.len() && next < d.selected.len() {
                        d.selected.swap(i, next);
                        cx.notify();
                    }
                }
                return;
            }
        }
        if let Some(rest) = id.strip_prefix("rotation-retry-skipped-") {
            if let Ok(i) = rest.parse::<usize>()
                && !self.applying
                && !self.rotation.pending
                && let Some((art, _)) = self.rotation.skipped.get(i).cloned()
            {
                self.rotation_command(Command::RetrySkipped { id: art.id }, cx);
                cx.notify();
            }
            return;
        }
        match id {
            "updates" => self.updates_open(window, cx),
            "updates-check" => self.updates_check(cx),
            "updates-install" => self.updates_install(cx),
            "updates-close" => self.updates_close(window, cx),
            "rotation" => self.rotation_open(window, cx),
            "rotation-add" => self.rotation_add(cx),
            "rotation-entire" | "rotation-selected"
                if !self.applying
                    && !self.rotation.command_busy
                    && self.rotation.state != RotationState::Preparing =>
            {
                if let Some(d) = &mut self.rotation.draft {
                    d.source = if id == "rotation-entire" {
                        RotationSource::EntireGallery
                    } else {
                        RotationSource::Selected
                    };
                }
                self.rotation.error = None;
                cx.notify();
            }
            "rotation-details" => {
                self.rotation.details_open = !self.rotation.details_open;
                cx.notify();
            }
            "rotation-close" => self.rotation_close(window, cx),
            "rotation-stop-save" => self.rotation_stop_save(cx),
            "rotation-start" => self.rotation_commit(true, cx),
            "rotation-save" => self.rotation_commit(false, cx),
            "rotation-pause" => self.rotation_pause(cx),
            "rotation-resume" => self.rotation_resume(cx),
            "rotation-stop" if !self.applying => self.rotation_command(Command::Stop, cx),
            "rotation-now" => self.rotation_apply(cx),
            "rotation-retry" => self.rotation_retry(cx),
            "back-to-results" => self.back_to_results(window, cx),
            "intro-motion" => self.toggle_motion(cx),
            "intro-start" => self.close_intro(Outcome::Completed, cx),
            "motion-toggle" => self.toggle_motion(cx),
            "show-walkthrough" => self.show_intro(window, cx),
            "refresh" => {
                if self.searching() {
                    self.schedule_search(false, cx);
                } else {
                    self.load(true, cx);
                }
            }
            "clear-search" | "empty-clear-search" => {
                self.clear_search(cx);
                self.search_input.focus_handle(cx).focus(window);
            }
            "retry-catalogue" if self.searching() => self.schedule_search(false, cx),
            "load-more" | "retry-catalogue" => self.load(
                if self.catalogue_error.is_some() {
                    self.last_load_refresh
                } else {
                    self.artworks.is_empty()
                },
                cx,
            ),
            "show-new" => {
                let filtered = self.filtered();
                if let Some(index) = filtered.iter().position(|a| self.new_ids.contains(&a.id)) {
                    self.reveal_row(index / self.columns + self.rows_start, true);
                }
            }
            "apply" => self.apply(cx),
            "view-source" => {
                if let Some(art) = self
                    .all_artworks()
                    .find(|a| Some(&a.id) == self.selected.as_ref())
                    && let Ok(url) = art.page_url()
                {
                    cx.open_url(&url);
                }
            }
            "retry-details" | "retry-preview" => {
                let art = self
                    .all_artworks()
                    .find(|a| Some(&a.id) == self.selected.as_ref())
                    .cloned();
                if let Some(art) = art {
                    if id == "retry-preview" {
                        self.previews.remove(&art.id);
                        self.hero_previews.remove(&art.id);
                    }
                    self.select(art, cx);
                }
            }
            _ => {}
        }
    }
    fn navigation_bottom(&self) -> f32 {
        if self.visible_loading()
            || !self.visible_notice().is_empty()
            || self.visible_error().is_some()
        {
            160.
        } else {
            72.
        }
    }
    fn reveal_row(&self, index: usize, align_top: bool) {
        if let Some(mut bounds) = self.grid_scroll.bounds_for_item(index) {
            let mut offset = self.grid_scroll.offset();
            bounds.origin += offset;
            let top = px(self.navigation_bottom());
            let bottom = self.grid_scroll.bounds().bottom();
            if align_top || bounds.top() < top {
                offset.y += top - bounds.top();
            } else if bounds.bottom() > bottom {
                offset.y -= bounds.bottom() - bottom;
            }
            offset.y = offset
                .y
                .clamp(-self.grid_scroll.max_offset().height, px(0.));
            self.grid_scroll.set_offset(offset);
        }
    }
    fn prioritize_previews(&mut self, cx: &mut Context<Self>) {
        let viewport = self.grid_scroll.bounds();
        let offset = self.grid_scroll.offset();
        let filtered = self.filtered();
        let mut visible = vec![];
        for (row, items) in filtered.chunks(self.columns).enumerate() {
            if let Some(mut bounds) = self.grid_scroll.bounds_for_item(row + self.rows_start) {
                bounds.origin += offset;
                if bounds.intersects(&viewport) {
                    visible.extend(items.iter().cloned());
                }
            }
        }
        for art in visible {
            self.preview(art, cx);
        }
    }
    fn filtered(&self) -> Vec<Artwork> {
        if self.searching() {
            self.search.results.clone()
        } else {
            self.artworks.clone()
        }
    }
}

const CANVAS: u32 = 0x111113;
const SURFACE: u32 = 0x202023;
const TEXT: u32 = 0xf5f5f7;
const MUTED: u32 = 0xa1a1aa;
const ACCENT: u32 = 0x0a84ff;
const RAISED: u32 = 0x2b2b30;

impl Gallery {
    fn reveal(&self, element: gpui::Div, group: &'static str, delay: f32) -> AnyElement {
        if !self.motion_enabled {
            return element.into_any_element();
        }
        element
            .with_animation(
                SharedString::from(format!("{group}-{}", self.selection_epoch)),
                Animation::new(Duration::from_millis(360)),
                move |element, t| {
                    let progress = reveal_progress(t, delay);
                    element
                        .opacity(progress)
                        .relative()
                        .top(px(8. * (1. - progress)))
                },
            )
            .into_any_element()
    }

    fn header(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .h(px(72.))
            .flex_none()
            .pl(px(if cfg!(target_os = "macos") { 100. } else { 24. }))
            .pr_6()
            .flex()
            .items_center()
            .gap_2()
            .child(app_icon(28.))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Pinacora"),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(self.button(
                "motion-toggle",
                if self.motion_enabled {
                    "Motion on"
                } else {
                    "Motion off"
                },
                true,
                cx,
            ))
            .child(self.button("show-walkthrough", "Guide", !self.applying, cx))
            .child(self.button(
                "rotation",
                match self.rotation.state {
                    RotationState::Running | RotationState::Applying => "Rotation on",
                    RotationState::Waiting => "Rotation waiting",
                    RotationState::Preparing => "Rotation preparing",
                    RotationState::Paused | RotationState::Failed => "Rotation paused",
                    RotationState::Stopped => "Rotation",
                },
                true,
                cx,
            ))
            .child(self.button(
                "updates",
                if self.updates.release.is_some() {
                    "Update available"
                } else {
                    "Updates"
                },
                !self.applying,
                cx,
            ))
            .child(self.gallery_search(cx))
            .child(self.button(
                "refresh",
                if self.visible_loading() && (self.searching() || self.last_load_refresh) {
                    "Refreshing…"
                } else {
                    "Refresh"
                },
                !self.visible_loading() && !self.applying,
                cx,
            ))
            .into_any_element()
    }
    fn gallery_search(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(218.))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .relative()
                    .child(self.search_input.clone())
                    .child(self.semantic(
                        "search".into(),
                        "Search artwork".into(),
                        AccessibilityRole::TextField,
                        self.query.clone(),
                        true,
                        false,
                        false,
                    )),
            )
            .when(!self.query.is_empty(), |d| {
                d.child(self.button("clear-search", "Clear", true, cx))
            })
            .into_any_element()
    }
    fn hero(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let art = self
            .selected
            .as_ref()
            .and_then(|id| self.all_artworks().find(|a| &a.id == id));
        let image_width = f32::from(window.viewport_size().width);
        let height = (image_width * 0.375).clamp(405., 520.);
        let mut view = div()
            .relative()
            .w_full()
            .h(px(height))
            .flex_none()
            .overflow_hidden()
            .bg(rgb(CANVAS));
        if let Some(art) = art {
            let picture = self
                .hero_previews
                .get(&art.id)
                .or_else(|| self.previews.get(&art.id).and_then(|p| p.as_ref().ok()));
            if let Some(path) = picture {
                let image = img(path.clone())
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(image_width))
                    .h(px(height))
                    .object_fit(ObjectFit::Cover);
                let image = if self.motion_enabled {
                    image
                        .with_animation(
                            SharedString::from(format!(
                                "hero-{}-{}-{}",
                                self.selection_epoch,
                                art.id,
                                path.display()
                            )),
                            Animation::new(Duration::from_millis(460)).with_easing(eased),
                            move |image, t| {
                                let scale = 1. + 0.035 * (1. - t);
                                image
                                    .opacity(t)
                                    .w(px(image_width * scale))
                                    .h(px(height * scale))
                                    .left(px(image_width * (1. - scale) * 0.5))
                                    .top(px(height * (1. - scale) * 0.5))
                            },
                        )
                        .into_any_element()
                } else {
                    image.into_any_element()
                };
                view = view.child(image);
            }
            view = view
                .child(div().absolute().inset_0().bg(linear_gradient(
                    90.,
                    linear_color_stop(rgba(0x111113ee), 0.),
                    linear_color_stop(rgba(0x11111300), 0.82),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0x11111388), 0.),
                    linear_color_stop(rgba(0x11111300), 0.3),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0x11111300), 0.35),
                    linear_color_stop(rgba(0x111113ff), 0.92),
                )));
            let title = self
                .detail
                .as_ref()
                .map(|d| d.title.clone())
                .unwrap_or_else(|| art.title().to_owned());
            let artist = self
                .detail
                .as_ref()
                .map(|d| d.artist.clone())
                .unwrap_or_else(|| art.artist().to_owned());
            let width = (f32::from(window.viewport_size().width) * 0.52).min(650.);
            let failed =
                picture.is_none() && self.previews.get(&art.id).is_some_and(|p| p.is_err());
            view = view.child(
                div()
                    .absolute()
                    .left(px(32.))
                    .bottom(px(if self.status.is_empty() { 140. } else { 226. }))
                    .w(px(width))
                    .max_h(px(height
                        - 96.
                        - if self.status.is_empty() { 140. } else { 226. }))
                    .id("preview-metadata-scroll")
                    .overflow_y_scroll()
                    .track_scroll(&self.metadata_scroll)
                    .child(self.semantic(
                        "hero-metadata-viewport".into(),
                        String::new(),
                        AccessibilityRole::StaticText,
                        String::new(),
                        true,
                        false,
                        false,
                    ))
                    .child(
                        self.reveal(
                            div()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    div()
                                        .relative()
                                        .flex_none()
                                        .text_size(px(36.))
                                        .line_height(px(42.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .line_clamp(2)
                                        .child(title.clone())
                                        .child(self.semantic(
                                            "selected-title".into(),
                                            title,
                                            AccessibilityRole::StaticText,
                                            String::new(),
                                            true,
                                            false,
                                            false,
                                        )),
                                )
                                .child(
                                    div()
                                        .relative()
                                        .flex_none()
                                        .w_full()
                                        .min_h(px(26.))
                                        .text_lg()
                                        .line_clamp(2)
                                        .text_color(rgb(0xd1d1d6))
                                        .child(artist.clone())
                                        .child(self.semantic(
                                            "selected-artist".into(),
                                            artist,
                                            AccessibilityRole::StaticText,
                                            String::new(),
                                            true,
                                            false,
                                            false,
                                        )),
                                )
                                .when_some(
                                    self.detail.as_ref().and_then(|d| d.dimensions.clone()),
                                    |d, dimensions| {
                                        d.child(self.text("selected-dimensions", dimensions))
                                    },
                                )
                                .when(picture.is_none(), |d| {
                                    d.child(self.text(
                                        "preview-status",
                                        if failed {
                                            "Preview unavailable"
                                        } else {
                                            "Loading preview…"
                                        },
                                    ))
                                })
                                .when(failed, |d| {
                                    d.child(self.button(
                                        "retry-preview",
                                        "Retry preview",
                                        !self.applying,
                                        cx,
                                    ))
                                })
                                .when(self.detail_loading, |d| {
                                    d.child(
                                        self.text("detail-progress", "Loading artwork details…"),
                                    )
                                })
                                .when_some(self.detail_error.clone(), |d, error| {
                                    d.child(
                                        div()
                                            .id("detail-error-scroll")
                                            .max_h(px(62.))
                                            .overflow_y_scroll()
                                            .child(self.text("detail-error", error)),
                                    )
                                    .child(self.button(
                                        "retry-details",
                                        "Retry details",
                                        !self.applying,
                                        cx,
                                    ))
                                }),
                            "hero-metadata",
                            0.,
                        ),
                    ),
            );
            view = view.child(
                div()
                    .absolute()
                    .left(px(32.))
                    .bottom(px(24.))
                    .w(px(width))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        self.reveal(
                            div()
                                .flex()
                                .gap_3()
                                .child(self.button(
                                    "apply",
                                    if self.downloading {
                                        "Downloading…"
                                    } else if self.applying {
                                        "Applying…"
                                    } else {
                                        "Set wallpaper"
                                    },
                                    self.detail.is_some()
                                        && !self.applying
                                        && !(!self.searching()
                                            && self.loading
                                            && self.last_load_refresh),
                                    cx,
                                ))
                                .child(self.button("view-source", "View source", true, cx))
                                .child(self.button(
                                    "rotation-add",
                                    "Add to rotation",
                                    !self.applying,
                                    cx,
                                )),
                            "hero-actions",
                            0.16,
                        ),
                    )
                    .child(div().text_xs().text_color(rgb(0xa1a1aa)).child(self.text(
                        "display-scope",
                        if pinacora::platform::all_desktops_supported() {
                            "Untouched original · All Desktops"
                        } else {
                            "Untouched original · Connected displays in this Desktop"
                        },
                    )))
                    .when(!self.status.is_empty(), |d| {
                        d.child(
                            div()
                                .id("apply-status-scroll")
                                .h(px(56.))
                                .text_sm()
                                .text_color(rgb(0xd1d1d6))
                                .overflow_y_scroll()
                                .child(self.text("apply-status", self.status.clone())),
                        )
                    }),
            );
        } else {
            view = view.child(
                div()
                    .absolute()
                    .left(px(32.))
                    .w(px(600.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_3()
                    .bottom(px(60.))
                    .child(self.text(
                        "initial-status",
                        if self.loading {
                            "Loading the gallery…"
                        } else {
                            "Open the gallery to load artwork."
                        },
                    ))
                    .when_some(self.catalogue_error.clone(), |d, error| {
                        d.child(self.text("catalogue-error", error))
                            .child(self.button(
                                "load-more",
                                "Retry loading artwork",
                                !self.loading,
                                cx,
                            ))
                    }),
            );
        }
        view = view.when(self.has_results_return, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(self.navigation_bottom() + 20.))
                    .right(px(24.))
                    .child(self.button("back-to-results", "Back to results", true, cx)),
            )
        });
        view.into_any_element()
    }
    fn tile(&self, art: Artwork, width: f32, cx: &Context<Self>) -> AnyElement {
        let selected = self.selected.as_ref() == Some(&art.id);
        let picture = match self.previews.get(&art.id) {
            Some(Ok(path)) => {
                let image_width = width - 10.;
                let image_height = image_width * 9. / 16.;
                let image = img(path.clone())
                    .absolute()
                    .left_0()
                    .top_0()
                    .w(px(image_width))
                    .h(px(image_height))
                    .object_fit(ObjectFit::Cover);
                if self.motion_enabled
                    && let Some(motion) = self.tile_motion.get(&art.id)
                {
                    let from = motion.from;
                    let hovered = motion.hovered;
                    image
                        .with_animation(
                            SharedString::from(format!("tile-hover-{}-{}", art.id, motion.epoch)),
                            Animation::new(Duration::from_millis(200)).with_easing(eased),
                            move |image, t| {
                                let scale = hover_scale(from, hovered, t);
                                image
                                    .w(px(image_width * scale))
                                    .h(px(image_height * scale))
                                    .left(px(image_width * (1. - scale) * 0.5))
                                    .top(px(image_height * (1. - scale) * 0.5))
                            },
                        )
                        .into_any_element()
                } else {
                    image.into_any_element()
                }
            }
            Some(Err(_)) => div()
                .p_3()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Preview unavailable · select to retry")
                .into_any_element(),
            None => div()
                .p_3()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Loading preview…")
                .into_any_element(),
        };
        let click = art.clone();
        let keyart = art.clone();
        let hover_id = art.id.clone();
        div()
            .relative()
            .id(SharedString::from(format!("tile-{}", art.id)))
            .track_focus(
                &self.tiles[&art.id]
                    .clone()
                    .tab_stop(
                        !self.applying
                            && !self.updates.visible
                            && !self.intro_visible
                            && self.rotation.draft.is_none()
                            && self.keyboard_tile.as_ref() == Some(&art.id),
                    )
                    .tab_index(30),
            )
            .tab_index(0)
            .tab_stop(
                self.keyboard_tile.as_ref() == Some(&art.id)
                    && !self.updates.visible
                    && !self.intro_visible
                    && self.rotation.draft.is_none(),
            )
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_1()
            .rounded(px(12.))
            .cursor_pointer()
            .border_1()
            .border_color(if selected {
                rgb(ACCENT)
            } else {
                rgba(0xffffff00)
            })
            .focus(|s| s.border_color(rgb(ACCENT)))
            .bg(if selected {
                rgb(SURFACE)
            } else {
                rgba(0xffffff00)
            })
            .hover(move |s| {
                s.bg(rgb(RAISED))
                    .border_color(if selected {
                        rgb(ACCENT)
                    } else {
                        rgba(0xffffff38)
                    })
                    .shadow(vec![BoxShadow {
                        color: rgba(0x00000055).into(),
                        offset: point(px(0.), px(4.)),
                        blur_radius: px(12.),
                        spread_radius: px(0.),
                    }])
            })
            .active(|s| s.bg(rgb(0x36363c)))
            .on_hover(cx.listener(move |app, hovered: &bool, _, cx| {
                if app
                    .tile_motion
                    .get(&hover_id)
                    .is_some_and(|motion| motion.hovered == *hovered)
                {
                    return;
                }
                let from = app
                    .tile_motion
                    .get(&hover_id)
                    .map(|motion| {
                        hover_scale(
                            motion.from,
                            motion.hovered,
                            eased(motion.started.elapsed().as_secs_f32() / 0.2),
                        )
                    })
                    .unwrap_or(1.);
                app.hover_epoch += 1;
                app.tile_motion.insert(
                    hover_id.clone(),
                    HoverMotion {
                        hovered: *hovered,
                        epoch: app.hover_epoch,
                        from,
                        started: Instant::now(),
                    },
                );
                cx.notify();
            }))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px((width - 10.) * 9. / 16.))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(rgb(SURFACE))
                    .child(picture),
            )
            .child(
                div()
                    .text_sm()
                    .line_height(px(20.))
                    .font_weight(FontWeight::MEDIUM)
                    .line_clamp(2)
                    .child(art.title().to_owned()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(art.artist().to_owned()),
            )
            .child(self.semantic(
                format!("tile-{}", art.id),
                format!("{} by {}", art.title(), art.artist()),
                AccessibilityRole::Button,
                String::new(),
                !self.applying,
                selected,
                false,
            ))
            .on_click(cx.listener(move |app, _, window, cx| {
                if app.updates.visible || app.rotation.draft.is_some() || app.intro_visible {
                    return;
                }
                app.keyboard_tile = Some(click.id.clone());
                app.tiles[&click.id].focus(window);
                if app.previews.get(&click.id).is_some_and(|p| p.is_err()) {
                    app.previews.remove(&click.id);
                }
                app.open_result(click.clone(), window, cx);
            }))
            .on_key_down(cx.listener(move |app, e: &KeyDownEvent, window, cx| {
                if app.tiles[&keyart.id].is_focused(window) && app.navigation_key(e, window, cx) {
                    return;
                }
                if !app.tiles[&keyart.id].is_focused(window)
                    || !matches!(
                        e.keystroke.key.as_str(),
                        "enter" | "space" | "left" | "right" | "up" | "down"
                    )
                {
                    return;
                }
                cx.stop_propagation();
                match e.keystroke.key.as_str() {
                    "enter" | "space" => {
                        app.open_result(keyart.clone(), window, cx);
                    }
                    "left" | "right" | "up" | "down" => {
                        let filtered = app.filtered();
                        if let Some(i) = filtered.iter().position(|a| a.id == keyart.id) {
                            let delta = match e.keystroke.key.as_str() {
                                "left" => -1,
                                "right" => 1,
                                "up" => -(app.columns as isize),
                                _ => app.columns as isize,
                            };
                            let next =
                                (i as isize + delta).clamp(0, filtered.len() as isize - 1) as usize;
                            app.keyboard_tile = Some(filtered[next].id.clone());
                            app.tiles[&filtered[next].id].focus(window);
                            app.reveal_row(next / app.columns + app.rows_start, false);
                        }
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .into_any_element()
    }
    fn footer(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .items_start()
            .py_5()
            .flex()
            .flex_col()
            .gap_3()
            .when(
                !self.searching() && (!self.end || self.catalogue_error.is_some()),
                |d| {
                    d.child(self.button(
                        "load-more",
                        if self.loading {
                            "Loading artwork…"
                        } else if self.catalogue_error.is_some() {
                            "Retry loading artwork"
                        } else {
                            "Load more artwork"
                        },
                        !self.loading,
                        cx,
                    ))
                },
            )
            .when(
                !self.searching() && self.end && self.catalogue_error.is_none(),
                |d| {
                    d.child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child("You’ve reached the end of the catalogue"),
                    )
                },
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(self.text("gallery-attribution", "Artwork from reframed.gallery")),
            )
            .into_any_element()
    }
}

impl Gallery {
    fn intro(&self, _window: &Window, cx: &Context<Self>) -> AnyElement {
        let copy = |id: &str, label: &str| {
            div()
                .relative()
                .child(label.to_string())
                .child(self.semantic(
                    id.into(),
                    label.into(),
                    AccessibilityRole::StaticText,
                    String::new(),
                    true,
                    false,
                    false,
                ))
        };
        div()
            .id("walkthrough-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgb(CANVAS))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .track_focus(&self.intro_focus)
            .on_key_down(cx.listener(Self::intro_key))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .w_full()
                    .max_w(px(480.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_5()
                    .text_center()
                    .child(app_icon(88.))
                    .child(
                        div()
                            .w_full()
                            .text_size(px(36.))
                            .line_height(px(44.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(copy("intro-title", "Welcome to Pinacora")),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_size(px(18.))
                            .line_height(px(28.))
                            .text_color(rgb(TEXT))
                            .child(copy("intro-description", "Browse classic art, choose a painting, and set it as your wallpaper.")),
                    )
                    .child(
                        div()
                            .text_color(rgb(MUTED))
                            .child(copy("intro-attribution", "Artwork from Reframed Gallery.")),
                    )
                    .child(self.button("intro-start", "Start browsing", true, cx))
                    .child(self.button(
                        "intro-motion",
                        if self.motion_enabled { "Motion on" } else { "Motion off" },
                        true,
                        cx,
                    )),
            )
            .into_any_element()
    }
}
impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.updates.visible
            && !self.intro_visible
            && self.rotation.draft.is_none()
            && (window.focused(cx).is_none()
                || self.intro_focus.is_focused(window)
                || self.controls["intro-start"].is_focused(window)
                || self.controls["intro-motion"].is_focused(window))
        {
            self.focus.focus(window);
        }
        if !self.navigation_subscribed {
            self.navigation_subscribed = true;
            cx.subscribe_in(
                &self.rotation_input,
                window,
                |app, _, event: &Navigate, window, cx| app.rotation_focus_step(event.0, window, cx),
            )
            .detach();
            cx.subscribe_in(
                &self.rotation_input,
                window,
                |app, _, _: &crate::search_input::Dismiss, window, cx| {
                    app.rotation_close(window, cx)
                },
            )
            .detach();
            cx.subscribe_in(
                &self.search_input,
                window,
                |app, _, event: &Navigate, window, cx| app.focus_step(event.0, window, cx),
            )
            .detach();
        }
        self.search_input.update(cx, |input, cx| {
            input.set_tab_enabled(
                !self.updates.visible && !self.intro_visible && self.rotation.draft.is_none(),
                cx,
            )
        });
        self.rotation_input.update(cx, |input, cx| {
            input.set_tab_enabled(
                !self.updates.visible
                    && self.rotation.draft.is_some()
                    && !self.applying
                    && !self.rotation.command_busy
                    && self.rotation.state != RotationState::Preparing,
                cx,
            )
        });
        if let Some(id) = self.rotation.reveal.take() {
            let this = cx.entity().downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = this.update(cx, |app, cx| {
                    app.rotation_reveal(&id, cx);
                });
            });
        }
        self.ax_nodes.borrow_mut().clear();
        if let Some(receiver) = self.ax_receiver.take() {
            *self.ax.borrow_mut() = AccessibilityBridge::new(window, self.ax_sender.clone());
            let this = cx.entity().downgrade();
            window
                .spawn(cx, async move |cx| {
                    while let Ok(action) = receiver.recv().await {
                        let result = this.update_in(cx, |app, window, cx| {
                            let action_id = match &action {
                                AccessibilityAction::Press(id)
                                | AccessibilityAction::SetValue(id, _)
                                | AccessibilityAction::SetSelection(id, _, _)
                                | AccessibilityAction::Focus(id) => id,
                            };
                            if app.updates.visible && !action_id.starts_with("updates-") {
                                return;
                            }
                            if !app.updates.visible
                                && app.rotation.draft.is_some()
                                && (!action_id.starts_with("rotation-")
                                    || action_id == "rotation-add")
                            {
                                return;
                            }
                            if !app.updates.visible
                                && app.intro_visible
                                && !action_id.starts_with("intro-")
                            {
                                return;
                            }
                            match action {
                                AccessibilityAction::Press(id) => {
                                    if let Some(id) = id.strip_prefix("tile-") {
                                        let art = app.all_artworks().find(|a| a.id == id).cloned();
                                        if let Some(art) = art {
                                            app.open_result(art, window, cx);
                                        }
                                    } else {
                                        app.activate(&id, window, cx);
                                    }
                                }
                                AccessibilityAction::SetValue(id, value) => {
                                    if id == "rotation-interval" {
                                        app.rotation_input
                                            .update(cx, |input, cx| input.set_value(value, cx));
                                    } else if id == "search" {
                                        app.search_input
                                            .update(cx, |input, cx| input.set_value(value, cx));
                                    }
                                }
                                AccessibilityAction::SetSelection(id, start, length) => {
                                    if id == "rotation-interval" {
                                        app.rotation_input.update(cx, |input, cx| {
                                            input.set_selection(start, length, cx)
                                        });
                                    } else if id == "search" {
                                        app.search_input.update(cx, |input, cx| {
                                            input.set_selection(start, length, cx)
                                        });
                                    }
                                }
                                AccessibilityAction::Focus(id) => {
                                    if app.rotation.draft.is_some() {
                                        app.rotation.reveal = Some(id.clone());
                                    }
                                    if id == "rotation-interval" {
                                        app.rotation_input.focus_handle(cx).focus(window);
                                    } else if id == "search" {
                                        app.search_input.focus_handle(cx).focus(window);
                                    } else if let Some(id) = id.strip_prefix("tile-") {
                                        if let Some(handle) = app.tiles.get(id) {
                                            handle.focus(window);
                                            app.keyboard_tile = Some(id.into());
                                            if let Some(index) =
                                                app.filtered().iter().position(|a| a.id == id)
                                            {
                                                app.reveal_row(
                                                    index / app.columns + app.rows_start,
                                                    false,
                                                );
                                            }
                                        }
                                    } else if let Some(handle) = app.controls.get(&id) {
                                        handle.focus(window);
                                    }
                                }
                            }
                            cx.notify();
                        });
                        if result.is_err() {
                            break;
                        }
                    }
                })
                .detach();
        }
        let scroll = self.grid_scroll.clone();
        let metadata_scroll = self.metadata_scroll.clone();
        let ax = self.ax.clone();
        let nodes = self.ax_nodes.clone();
        let controls = self.controls.clone();
        let tiles = self.tiles.clone();
        let input = self.search_input.clone();
        let updates_visible = self.updates.visible;
        let intro_visible = self.intro_visible;
        let rotation_visible = self.rotation.draft.is_some();
        let rotation_input = self.rotation_input.clone();
        let navigation_bottom = self.navigation_bottom();
        let width = f32::from(window.viewport_size().width) - 48.;
        self.columns = if width >= 4. * 260. + 48. { 4 } else { 3 };
        let tile_width = (width - (self.columns - 1) as f32 * 16.) / self.columns as f32;
        let filtered = self.filtered();
        let show_hero = self.query.trim().is_empty() || self.selected_result_preview;
        self.rows_start = 3;
        let notice_visible = self.visible_loading()
            || !self.visible_notice().is_empty()
            || self.visible_error().is_some();
        if !filtered
            .iter()
            .any(|a| Some(&a.id) == self.keyboard_tile.as_ref())
        {
            self.keyboard_tile = filtered.first().map(|a| a.id.clone());
        }
        if self.grid_scroll.bounds_for_item(self.rows_start).is_none() {
            for art in filtered.iter().take(self.columns * 3).cloned() {
                self.preview(art, cx);
            }
        }
        let heading = if self.query.trim().is_empty() {
            "Recent artworks"
        } else {
            "Search results"
        };
        let count = if !self.searching() {
            format!("{} artworks loaded", self.artworks.len())
        } else if self.search.loading {
            "Searching Reframed…".into()
        } else if self.search.error.is_some() {
            "Search unavailable".into()
        } else if self.search.query.chars().count() < 2 {
            "Enter at least 2 characters to search.".into()
        } else {
            format!("{} artwork results from Reframed", filtered.len())
        };
        let empty: String = if self.searching() {
            if self.search.loading {
                "Searching Reframed…".into()
            } else if self.search.error.is_some() {
                "Search could not be completed. Retry below.".into()
            } else if self.search.query.chars().count() < 2 {
                "Enter at least 2 characters to search.".into()
            } else {
                "No matching artworks found on Reframed.".into()
            }
        } else if self.loading {
            "Loading artworks…".into()
        } else if self.catalogue_error.is_some() {
            "The gallery could not be loaded. Retry below.".into()
        } else {
            "No artwork is available in the catalogue.".into()
        };
        let rows: Vec<_> = filtered
            .chunks(self.columns)
            .map(|row| {
                div()
                    .px_6()
                    .pb_4()
                    .flex()
                    .gap_4()
                    .children(
                        row.iter()
                            .cloned()
                            .map(|art| self.tile(art, tile_width, cx)),
                    )
                    .into_any_element()
            })
            .collect();
        let align_results = std::mem::take(&mut self.focus_results_after_layout);
        let this = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = this.update(cx, |app, cx| {
                if align_results {
                    app.reveal_row(app.rows_start - 1, true);
                    cx.notify();
                }
                app.prioritize_previews(cx);
            });
        });
        if self.updates.visible && !self.updates_focus.contains_focused(window, cx) {
            self.updates_focus_step(false, window, cx);
        }
        if !self.updates.visible
            && self.intro_visible
            && !self.intro_focus.contains_focused(window, cx)
        {
            self.controls["intro-start"].focus(window);
        }
        div()
            .relative()
            .size_full()
            .bg(rgb(CANVAS))
            .text_color(rgb(TEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|app, _: &crate::CheckForUpdates, window, cx| {
                app.updates_open(window, cx);
                app.updates_check(cx);
            }))
            .flex()
            .flex_col()
            .child(
                div()
                    .id("artwork-grid")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.grid_scroll)
                    .child(self.semantic(
                        "grid-viewport".into(),
                        String::new(),
                        AccessibilityRole::StaticText,
                        String::new(),
                        true,
                        false,
                        false,
                    ))
                    .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                    .when(show_hero, |d| d.child(self.hero(window, cx)))
                    .when(!show_hero, |d| {
                        d.child(
                            div()
                                .h(px(if notice_visible { 160. } else { 72. }))
                                .flex_none(),
                        )
                    })
                    .child(
                        div()
                            .px_6()
                            .py_5()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(heading),
                            )
                            .child(self.text("results-count", count)),
                    )
                    .children(rows)
                    .when(filtered.is_empty(), |d| {
                        d.child(
                            div()
                                .px_6()
                                .py_5()
                                .child(self.text("empty-results", empty))
                                .when(!self.query.is_empty(), |d| {
                                    d.child(div().flex().gap_3().child(self.button(
                                        "empty-clear-search",
                                        "Clear search",
                                        true,
                                        cx,
                                    )))
                                }),
                        )
                    })
                    .child(div().px_6().child(self.footer(cx))),
            )
            .when(
                self.visible_loading()
                    || !self.visible_notice().is_empty()
                    || self.visible_error().is_some(),
                |d| {
                    d.child(
                        div()
                            .id("catalogue-feedback")
                            .block_mouse_except_scroll()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(72.))
                            .max_h(px(88.))
                            .overflow_y_scroll()
                            .bg(rgba(0x111113e8))
                            .px_6()
                            .py_2()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .when(self.visible_loading(), |d| {
                                        d.child(self.text(
                                            "catalogue-loading",
                                            if self.searching() {
                                                "Searching Reframed…"
                                            } else {
                                                "Loading artwork…"
                                            },
                                        ))
                                    })
                                    .when(!self.visible_notice().is_empty(), |d| {
                                        d.child(self.text(
                                            "catalogue-notice",
                                            self.visible_notice().clone(),
                                        ))
                                    })
                                    .when_some(self.visible_error().clone(), |d, error| {
                                        d.child(
                                            div()
                                                .line_clamp(2)
                                                .child(self.text("catalogue-error", error)),
                                        )
                                    }),
                            )
                            .when(
                                !self.searching()
                                    && filtered.iter().any(|a| self.new_ids.contains(&a.id)),
                                |d| d.child(self.button("show-new", "Show new artwork", true, cx)),
                            )
                            .when(self.visible_error().is_some(), |d| {
                                d.child(self.button(
                                    "retry-catalogue",
                                    if self.searching() {
                                        "Retry search"
                                    } else {
                                        "Retry loading artwork"
                                    },
                                    !self.visible_loading(),
                                    cx,
                                ))
                            }),
                    )
                },
            )
            .child(
                div()
                    .id("pinned-header")
                    .block_mouse_except_scroll()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(72.))
                    .child(self.header(cx)),
            )
            .when(self.intro_visible && !self.updates.visible, |d| {
                d.child(self.intro(window, cx))
            })
            .when(
                self.rotation.draft.is_some() && !self.updates.visible,
                |d| d.child(self.rotation_panel(window, cx)),
            )
            .when(self.updates.visible, |d| {
                d.child(self.updates_panel(window, cx))
            })
            .child(
                gpui::canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        if let Some(bridge) = ax.borrow_mut().as_mut() {
                            let mut snapshot = nodes.borrow().clone();
                            let grid_bounds = Some(scroll.bounds());
                            let metadata_bounds = Some(metadata_scroll.bounds());
                            for node in &mut snapshot {
                                let pinned = matches!(
                                    node.id.as_str(),
                                    "search"
                                        | "motion-toggle"
                                        | "show-walkthrough"
                                        | "refresh"
                                        | "clear-search"
                                        | "retry-catalogue"
                                        | "show-new"
                                        | "preferences-error"
                                ) || node.id.starts_with("catalogue-")
                                    || node.id.starts_with("updates-")
                                    || node.id == "updates"
                                    || node.id.starts_with("intro-")
                                    || (node.id.starts_with("rotation-")
                                        && node.id != "rotation-add")
                                    || node.id == "rotation";
                                if !pinned && let Some(bounds) = grid_bounds {
                                    let visible = gpui::Bounds::new(
                                        point(px(0.), px(navigation_bottom)),
                                        gpui::size(
                                            window.viewport_size().width,
                                            window.viewport_size().height - px(navigation_bottom),
                                        ),
                                    );
                                    node.bounds =
                                        node.bounds.intersect(&bounds).intersect(&visible);
                                }
                                if matches!(
                                    node.id.as_str(),
                                    "selected-title"
                                        | "selected-artist"
                                        | "selected-dimensions"
                                        | "preview-status"
                                        | "retry-preview"
                                        | "detail-progress"
                                        | "detail-error"
                                        | "retry-details"
                                ) && let Some(bounds) = metadata_bounds
                                {
                                    node.bounds = node.bounds.intersect(&bounds);
                                }
                            }
                            snapshot.retain(|n| {
                                n.id != "grid-viewport"
                                    && n.id != "hero-metadata-viewport"
                                    && n.bounds.size.width > px(0.)
                                    && n.bounds.size.height > px(0.)
                            });
                            for node in &mut snapshot {
                                if node.id == "rotation-interval" {
                                    node.focused =
                                        rotation_input.focus_handle(cx).is_focused(window);
                                    node.selected_range =
                                        Some(rotation_input.read(cx).utf16_selection());
                                } else if node.id == "search" {
                                    node.focused = input.focus_handle(cx).is_focused(window);
                                    node.selected_range = Some(input.read(cx).utf16_selection());
                                } else if let Some(handle) = controls.get(&node.id).or_else(|| {
                                    node.id.strip_prefix("tile-").and_then(|id| tiles.get(id))
                                }) {
                                    node.focused = handle.is_focused(window);
                                }
                            }
                            if updates_visible {
                                snapshot.retain(|node| node.id.starts_with("updates-"));
                            } else if rotation_visible {
                                snapshot.retain(|node| {
                                    node.id.starts_with("rotation-") && node.id != "rotation-add"
                                });
                            } else if intro_visible {
                                snapshot.retain(|node| node.id.starts_with("intro-"));
                            } else {
                                snapshot.retain(|node| {
                                    !node.id.starts_with("updates-")
                                        && !node.id.starts_with("intro-")
                                        && (!node.id.starts_with("rotation-")
                                            || node.id == "rotation-add")
                                });
                            }
                            bridge.update(snapshot);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

fn request_page(current: usize, refresh: bool) -> usize {
    if refresh { 1 } else { current + 1 }
}
fn selection_retained(artworks: &[Artwork], selected: Option<&str>) -> bool {
    selected.is_some_and(|id| artworks.iter().any(|a| a.id == id))
}
fn append_unique(artworks: &mut Vec<Artwork>, items: Vec<Artwork>) -> Vec<Artwork> {
    let mut added = vec![];
    for item in items {
        if !artworks.iter().any(|a| a.id == item.id) {
            added.push(item.clone());
            artworks.push(item);
        }
    }
    added
}
#[derive(Default)]
struct SiteSearch {
    epoch: u64,
    query: String,
    results: Vec<Artwork>,
    loading: bool,
    error: Option<String>,
}
impl SiteSearch {
    fn begin(&mut self, query: &str) -> u64 {
        self.epoch += 1;
        self.query = query.trim().into();
        self.results.clear();
        self.error = None;
        self.loading = self.query.chars().count() >= 2;
        self.epoch
    }
    fn browse_can_update_view(&self, epoch: u64) -> bool {
        self.epoch == epoch && self.query.is_empty()
    }
    fn current(&self, epoch: u64) -> bool {
        self.epoch == epoch && self.loading
    }
    fn complete(&mut self, epoch: u64, result: Result<Vec<Artwork>, String>) -> bool {
        if !self.current(epoch) {
            return false;
        }
        self.loading = false;
        match result {
            Ok(items) => self.results = items,
            Err(error) => self.error = Some(error),
        }
        true
    }
}

fn control_tab_index(id: &str) -> isize {
    match id {
        "motion-toggle" | "intro-start" => 0,
        "show-walkthrough" | "intro-motion" => 1,
        "clear-search" => 3,
        "refresh" => 4,
        "rotation" => 5,
        "updates" => 6,
        "updates-check" => 0,
        "updates-install" => 1,
        "updates-close" => 2,
        "show-new" => 10,
        "retry-catalogue" => 11,
        "back-to-results" => 20,
        "retry-preview" => 21,
        "retry-details" => 22,
        "apply" => 23,
        "view-source" => 24,
        "rotation-add" => 25,
        "empty-clear-search" => 31,
        "load-more" => 40,
        _ => 50,
    }
}

impl Gallery {
    fn updates_poll(&mut self, cx: &mut Context<Self>) {
        self.updates_check(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(pinacora::updater::CHECK_INTERVAL)
                    .await;
                if this.update(cx, |app, cx| app.updates_check(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    fn updates_check(&mut self, cx: &mut Context<Self>) {
        if !self.updates.begin_check() {
            return;
        }
        let task = cx
            .background_executor()
            .spawn(async { pinacora::updater::check().map_err(|error| error.to_string()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.updates.checked(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn updates_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying {
            return;
        }
        self.updates.visible = true;
        self.updates_focus_step(false, window, cx);
        cx.notify();
    }
    fn updates_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.installing {
            return;
        }
        self.updates.visible = false;
        if self.rotation.draft.is_some() {
            self.controls["rotation-close"].focus(window);
        } else if self.intro_visible {
            self.controls["intro-start"].focus(window);
        } else {
            self.controls["updates"].focus(window);
        }
        cx.notify();
    }
    fn updates_focus_step(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut ids = vec![];
        if !self.updates.checking && !self.updates.installing {
            ids.push("updates-check");
        }
        if self.updates.release.is_some() && !self.updates.checking && !self.updates.installing {
            ids.push("updates-install");
        }
        if !self.updates.installing {
            ids.push("updates-close");
        }
        if ids.is_empty() {
            self.updates_focus.focus(window);
            return;
        }
        let current = ids
            .iter()
            .position(|id| self.controls[*id].is_focused(window));
        let next = match current {
            Some(index) if reverse => (index + ids.len() - 1) % ids.len(),
            Some(index) => (index + 1) % ids.len(),
            None => 0,
        };
        self.controls[ids[next]].focus(window);
        cx.notify();
    }
    fn updates_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "tab" => {
                cx.stop_propagation();
                self.updates_focus_step(event.keystroke.modifiers.shift, window, cx);
            }
            "escape" => {
                cx.stop_propagation();
                self.updates_close(window, cx);
            }
            _ => {}
        }
    }
    fn updates_install(&mut self, cx: &mut Context<Self>) {
        if self.updates.installing || self.updates.checking {
            return;
        }
        let Some(release) = self.updates.release.clone() else {
            return;
        };
        self.updates.installing = true;
        self.updates.error = None;
        let task = cx.background_executor().spawn(async move {
            pinacora::updater::stage_update(&release)
                .and_then(|pending| pending.launch())
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.updates.installing = false;
                match result {
                    Ok(()) => cx.quit(),
                    Err(error) => {
                        app.updates.error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn updates_panel(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let busy = self.updates.checking || self.updates.installing;
        let status = if self.updates.installing {
            "Downloading and preparing the update… Pinacora will restart when ready.".into()
        } else if self.updates.checking {
            "Checking for updates…".into()
        } else if let Some(release) = &self.updates.release {
            format!("Pinacora {} is available.", release.version)
        } else if self.updates.error.is_some() {
            "Could not check for updates. Try again.".into()
        } else if self.updates.checked {
            "You're up to date. No newer stable release is available.".into()
        } else {
            "Check for the latest stable release.".into()
        };
        div()
            .id("updates-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x000000bb))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .track_focus(&self.updates_focus)
            .on_key_down(cx.listener(Self::updates_key))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("updates-panel")
                    .overflow_y_scroll()
                    .w(px(480.))
                    .max_w_full()
                    .max_h(px(f32::from(window.viewport_size().height) - 48.))
                    .bg(rgb(SURFACE))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(rgb(RAISED))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .relative()
                            .text_size(px(22.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Pinacora updates")
                            .child(self.semantic("updates-title".into(), "Pinacora updates".into(), AccessibilityRole::StaticText, String::new(), true, false, false)),
                    )
                    .child(div().text_color(rgb(MUTED)).child(self.text(
                        "updates-current",
                        format!("Current version: {}", env!("CARGO_PKG_VERSION")),
                    )))
                    .child(self.text("updates-status", status))
                    .when(self.rotation_dirty(), |d| {
                        d.child(div().text_color(rgb(MUTED)).child(self.text(
                            "updates-unsaved-rotation",
                            "Restarting discards unsaved rotation settings. Save or cancel those changes first.",
                        )))
                    })
                    .when_some(self.updates.error.as_ref(), |d, error| {
                        d.child(
                            div()
                                .text_color(rgb(0xf0b2ac))
                                .child(self.text("updates-error", error.clone())),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button("updates-check", "Check for updates", !busy, cx))
                            .when(self.updates.release.is_some(), |d| {
                                d.child(self.button(
                                    "updates-install",
                                    "Update and restart",
                                    !busy,
                                    cx,
                                ))
                            })
                            .child(div().flex_1())
                            .child(self.button(
                                "updates-close",
                                "Close",
                                !self.updates.installing,
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }
}

impl Gallery {
    fn rotation_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let epoch = match this.update(cx, |app, _| {
                    if app.rotation.command_busy || app.applying {
                        None
                    } else {
                        Some(app.rotation.command_epoch)
                    }
                }) {
                    Ok(value) => value,
                    Err(_) => break,
                };
                if let Some(epoch) = epoch {
                    let task = cx.background_executor().spawn(async {
                        rotation_service::request(Command::Status).map_err(|e| format!("{e:#}"))
                    });
                    let result = task.await;
                    if this
                        .update(cx, |app, cx| {
                            if app.rotation.command_epoch != epoch
                                || app.rotation.command_busy
                                || app.applying
                            {
                                return;
                            }
                            match result {
                                Ok(snapshot) => app.rotation.snapshot = snapshot,
                                Err(error) => app.rotation.unavailable(error),
                            }
                            app.rotation_sync_controls(cx);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        })
        .detach();
    }
    fn rotation_command(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.rotation.command_busy || self.applying {
            return;
        }
        self.rotation.command_busy = true;
        self.rotation.command_epoch += 1;
        let epoch = self.rotation.command_epoch;
        let task = cx.background_executor().spawn(async move {
            let result = pinacora::rotation_service_manager::ensure_running()
                .and_then(|_| rotation_service::request(command));
            match result {
                Ok(snapshot) => Ok(snapshot),
                Err(error) => match rotation_service::request(Command::Status) {
                    Ok(mut snapshot) => {
                        snapshot.error = Some(format!("{error:#}"));
                        Ok(snapshot)
                    }
                    Err(_) => Err(format!("{error:#}")),
                },
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                if app.rotation.command_epoch != epoch {
                    return;
                }
                app.rotation.command_busy = false;
                match result {
                    Ok(snapshot) => {
                        app.rotation.snapshot = snapshot;
                    }
                    Err(error) => {
                        app.rotation.unavailable(error.clone());
                        app.rotation.error = Some(error);
                    }
                }
                app.rotation_sync_controls(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn rotation_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.intro_visible {
            return;
        }
        self.rotation.details_open = false;
        self.rotation.draft = Some(self.rotation.prefs.clone());
        self.rotation.minutes = self.rotation.prefs.minutes.to_string();
        let value = self.rotation.minutes.clone();
        self.rotation_input
            .update(cx, |input, cx| input.set_value(value, cx));
        self.rotation.reveal = None;
        self.rotation_sync_controls(cx);
        self.rotation.scroll.set_offset(point(px(0.), px(0.)));
        // Changing focus during the opening Enter can deliver that same event to
        // the source control. Wait until the next frame and focus the saved source.
        let this = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            let _ = this.update(cx, |app, cx| {
                if let Some(draft) = &app.rotation.draft {
                    let id = if draft.source == RotationSource::EntireGallery {
                        "rotation-entire"
                    } else {
                        "rotation-selected"
                    };
                    app.controls[id].focus(window);
                    app.rotation.reveal = Some(id.into());
                    cx.notify();
                }
            });
        });
        cx.notify();
    }
    fn rotation_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rotation.draft = None;
        self.rotation.reveal = None;
        self.controls["rotation"].focus(window);
        cx.notify();
    }
    fn rotation_sync_controls(&mut self, cx: &mut Context<Self>) {
        let count = self.rotation.draft.as_ref().map_or(0, |d| d.selected.len());
        for i in 0..count {
            for action in ["remove", "up", "down"] {
                self.controls
                    .entry(format!("rotation-{action}-{i}"))
                    .or_insert_with(|| cx.focus_handle().tab_stop(true));
            }
        }
        for i in 0..self.rotation.skipped.len() {
            self.controls
                .entry(format!("rotation-retry-skipped-{i}"))
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
    }
    fn rotation_add(&mut self, cx: &mut Context<Self>) {
        if self.rotation.command_busy || self.applying {
            return;
        }
        let art = self
            .all_artworks()
            .find(|a| Some(&a.id) == self.selected.as_ref())
            .cloned();
        if let Some(art) = art {
            let mut prefs = self.rotation.prefs.clone();
            if prefs.selected.iter().any(|a| a.id == art.id) {
                self.status = "Already in your rotation collection.".into();
            } else if prefs.selected.len() < rotation::MAX_ARTWORKS {
                prefs.selected.push(art);
                self.rotation_command(Command::Save { prefs }, cx);
            }
            cx.notify();
        }
    }
    fn rotation_pause(&mut self, cx: &mut Context<Self>) {
        self.rotation_command(Command::Pause, cx);
    }
    fn rotation_stop_save(&mut self, cx: &mut Context<Self>) {
        if self.applying
            || self.rotation.command_busy
            || self.rotation.state == RotationState::Preparing
        {
            return;
        }
        let Some(mut prefs) = self.rotation.draft.clone() else {
            return;
        };
        let Ok(minutes) = rotation::minutes(&self.rotation.minutes) else {
            return;
        };
        prefs.minutes = minutes;
        prefs.configured = false;
        self.rotation_command(Command::Save { prefs }, cx);
    }
    fn rotation_commit(&mut self, start: bool, cx: &mut Context<Self>) {
        if self.applying
            || self.rotation.command_busy
            || self.rotation.state == RotationState::Preparing
        {
            return;
        }
        let Some(mut prefs) = self.rotation.draft.clone() else {
            return;
        };
        let validation = rotation::minutes(&self.rotation.minutes).and_then(|minutes| {
            prefs.minutes = minutes;
            prefs.configured = true;
            prefs.validate()
        });
        if let Err(error) = validation {
            self.rotation.error = Some(error.to_string());
            cx.notify();
            return;
        }
        if let Some(error) = pinacora::platform::support_error() {
            self.rotation.error = Some(error);
            cx.notify();
            return;
        }
        let command = if start {
            Command::Start { prefs }
        } else {
            Command::Save { prefs }
        };
        self.rotation_command(command, cx);
    }
    fn rotation_next(&self) -> Option<&Artwork> {
        self.rotation.next.as_ref()
    }
    fn rotation_resume(&mut self, cx: &mut Context<Self>) {
        self.rotation_command(Command::Resume, cx);
    }
    fn rotation_apply(&mut self, cx: &mut Context<Self>) {
        self.rotation_command(Command::ChangeNow, cx);
    }
    fn rotation_retry(&mut self, cx: &mut Context<Self>) {
        self.rotation_command(Command::Retry, cx);
    }
    fn rotation_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.rotation_close(window, cx);
        } else if event.keystroke.key == "tab" {
            cx.stop_propagation();
            self.rotation_focus_step(event.keystroke.modifiers.shift, window, cx);
        }
    }
    fn rotation_focus_ids(&self) -> Vec<String> {
        let busy = self.applying
            || self.rotation.command_busy
            || self.rotation.state == RotationState::Preparing;
        let mut ids = vec![];
        if self.rotation.prefs.configured || self.rotation.pending {
            if (self.rotation.wants_running || self.rotation.pending)
                && !self.applying
                && !self.rotation.command_busy
            {
                ids.push("rotation-pause".into());
            } else if self.rotation.prefs.configured && !self.rotation.pending && !busy {
                ids.push("rotation-resume".into());
            }
            if self.rotation.wants_running && self.rotation.prepared_path.is_some() && !busy {
                ids.push("rotation-now".into());
            }
            if self.rotation.error.is_some() && !busy {
                ids.push("rotation-retry".into());
            }
        }
        if !busy {
            ids.extend([
                "rotation-entire".into(),
                "rotation-selected".into(),
                "rotation-interval".into(),
            ]);
            if let Some(draft) = &self.rotation.draft
                && draft.source == RotationSource::Selected
            {
                for i in 0..draft.selected.len() {
                    if i > 0 {
                        ids.push(format!("rotation-up-{i}"));
                    }
                    if i + 1 < draft.selected.len() {
                        ids.push(format!("rotation-down-{i}"));
                    }
                    ids.push(format!("rotation-remove-{i}"));
                }
            }
            if !self.rotation.pending {
                for i in 0..self.rotation.skipped.len() {
                    ids.push(format!("rotation-retry-skipped-{i}"));
                }
            }
        }
        ids.push("rotation-details".into());
        if self.rotation.prefs.configured
            && self.rotation.state != RotationState::Stopped
            && !busy
            && !self.rotation.pending
        {
            ids.push("rotation-stop".into());
        }
        ids.push("rotation-close".into());
        if let Some(draft) = &self.rotation.draft
            && !busy
            && rotation::minutes(&self.rotation.minutes).is_ok()
        {
            let valid = (draft.source != RotationSource::Selected || draft.selected.len() >= 2)
                && pinacora::platform::support_error().is_none();
            if self.rotation_dirty()
                && draft.source == RotationSource::Selected
                && draft.selected.len() < 2
            {
                ids.push("rotation-stop-save".into());
            } else if self.rotation_dirty() && self.rotation.prefs.configured && valid {
                ids.push("rotation-save".into());
            }
            if (!self.rotation.prefs.configured || self.rotation.state == RotationState::Stopped)
                && valid
            {
                ids.push("rotation-start".into());
            }
        }
        ids
    }
    fn rotation_focus_step(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self.rotation_focus_ids();
        let current = ids.iter().position(|id| {
            if id == "rotation-interval" {
                self.rotation_input.focus_handle(cx).is_focused(window)
            } else {
                self.controls.get(id).is_some_and(|h| h.is_focused(window))
            }
        });
        let next = match current {
            Some(i) if reverse => (i + ids.len() - 1) % ids.len(),
            Some(i) => (i + 1) % ids.len(),
            None => 0,
        };
        self.rotation.reveal = Some(ids[next].clone());
        if ids[next] == "rotation-interval" {
            self.rotation_input.focus_handle(cx).focus(window);
        } else {
            self.controls[&ids[next]].focus(window);
        }
        cx.notify();
    }
    fn rotation_reveal(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.rotation.draft.is_none()
            || matches!(
                id,
                "rotation-close" | "rotation-save" | "rotation-start" | "rotation-stop-save"
            )
        {
            return;
        }
        let viewport = self.rotation.scroll.bounds();
        let nodes = self.ax_nodes.borrow();
        let Some(node) = nodes.iter().find(|node| node.id == id) else {
            return;
        };
        let top = node.bounds.top();
        let mut bottom = node.bounds.bottom();
        if id == "rotation-interval"
            && let Some(error) = nodes.iter().find(|node| node.id == "rotation-invalid")
        {
            bottom = bottom.max(error.bounds.bottom());
        }
        let mut offset = self.rotation.scroll.offset();
        if top < viewport.top() + px(12.) {
            offset.y += viewport.top() + px(12.) - top;
        } else if bottom > viewport.bottom() - px(12.) {
            offset.y -= bottom - viewport.bottom() + px(12.);
        }
        offset.y = offset
            .y
            .clamp(-self.rotation.scroll.max_offset().height, px(0.));
        if offset != self.rotation.scroll.offset() {
            self.rotation.scroll.set_offset(offset);
            cx.notify();
        }
    }
    fn rotation_row_button(
        &self,
        id: String,
        label: String,
        enabled: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let key = id.clone();
        let click = id.clone();
        let handle = self.controls[&id].clone();
        div()
            .relative()
            .id(SharedString::from(id.clone()))
            .track_focus(&handle.clone().tab_stop(enabled))
            .tab_stop(enabled)
            .px_3()
            .py_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(RAISED))
            .bg(rgb(SURFACE))
            .text_xs()
            .opacity(if enabled { 1. } else { 0.4 })
            .focus(|s| s.border_color(rgb(ACCENT)))
            .when(enabled, |d| d.cursor_pointer().hover(|s| s.bg(rgb(RAISED))))
            .child(label.clone())
            .child(self.semantic(
                id,
                label,
                AccessibilityRole::Button,
                String::new(),
                enabled,
                false,
                false,
            ))
            .on_click(cx.listener(move |app, _, window, cx| {
                if enabled {
                    app.activate(&click, window, cx);
                }
            }))
            .on_key_down(cx.listener(move |app, e: &KeyDownEvent, window, cx| {
                if enabled
                    && !e.is_held
                    && handle.is_focused(window)
                    && matches!(e.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                    app.activate(&key, window, cx);
                }
            }))
            .into_any_element()
    }
    fn rotation_dirty(&self) -> bool {
        self.rotation.draft.as_ref().is_some_and(|draft| {
            draft.source != self.rotation.prefs.source
                || rotation::minutes(&self.rotation.minutes).ok()
                    != Some(self.rotation.prefs.minutes)
                || !draft.selected.iter().map(|art| &art.id).eq(self
                    .rotation
                    .prefs
                    .selected
                    .iter()
                    .map(|art| &art.id))
        })
    }

    fn rotation_panel(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let Some(draft) = &self.rotation.draft else {
            return div().into_any_element();
        };
        let busy = self.applying
            || self.rotation.command_busy
            || self.rotation.state == RotationState::Preparing;
        let valid = rotation::minutes(&self.rotation.minutes).is_ok()
            && (draft.source != RotationSource::Selected || draft.selected.len() >= 2)
            && pinacora::platform::support_error().is_none();
        let mut body =
            div()
                .w_full()
                .flex_none()
                .flex()
                .flex_col()
                .gap_5()
                .p_6()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .w(px(44.))
                                .h(px(44.))
                                .flex_none()
                                .rounded(px(12.))
                                .bg(rgba(0x0a84ff18))
                                .border_1()
                                .border_color(rgba(0x0a84ff30))
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(28.))
                                .text_color(rgb(0x79b7ff))
                                .child("↻"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .relative()
                                        .text_size(px(22.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("Wallpaper rotation")
                                        .child(self.semantic(
                                            "rotation-title".into(),
                                            "Wallpaper rotation".into(),
                                            AccessibilityRole::StaticText,
                                            String::new(),
                                            true,
                                            false,
                                            false,
                                        )),
                                )
                                .when(
                                    !self.rotation.prefs.configured && !self.rotation.pending,
                                    |d| {
                                        d.child(div().text_color(rgb(MUTED)).child(self.text(
                                            "rotation-intro",
                                            "Choose artwork and a schedule.",
                                        )))
                                    },
                                ),
                        ),
                )
                .when(
                    self.rotation.prefs.configured || self.rotation.pending,
                    |d| d.child(self.rotation_live_card(cx)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .p_4()
                        .rounded(px(14.))
                        .bg(rgb(0x18181b))
                        .border_1()
                        .border_color(rgb(RAISED))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(self.text("rotation-source-label", "Artwork"))
                                .child(
                                    div()
                                        .flex()
                                        .gap_1()
                                        .p_1()
                                        .rounded(px(10.))
                                        .bg(rgb(CANVAS))
                                        .child(self.button(
                                            "rotation-entire",
                                            "Entire gallery",
                                            !busy,
                                            cx,
                                        ))
                                        .child(self.button(
                                            "rotation-selected",
                                            "Selected artworks",
                                            !busy,
                                            cx,
                                        )),
                                )
                                .child(div().text_color(rgb(MUTED)).child(self.text(
                                    "rotation-source-summary",
                                    if draft.source == RotationSource::EntireGallery {
                                        "Shuffle across the gallery."
                                    } else {
                                        "Play your collection in order."
                                    },
                                ))),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .pt_4()
                                .border_t_1()
                                .border_color(rgb(RAISED))
                                .child(
                                    div().flex_1().child(
                                        self.text("rotation-interval-label", "Change every"),
                                    ),
                                )
                                .child(
                                    div()
                                        .relative()
                                        .w(px(76.))
                                        .flex_none()
                                        .child(self.rotation_input.clone())
                                        .child(self.semantic(
                                            "rotation-interval".into(),
                                            "Change every, in minutes".into(),
                                            AccessibilityRole::TextField,
                                            self.rotation.minutes.clone(),
                                            !busy,
                                            false,
                                            false,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_sm()
                                        .text_color(rgb(MUTED))
                                        .child("minutes"),
                                ),
                        ),
                );
        if draft.source != self.rotation.prefs.source && self.rotation.prefs.configured {
            body = body.child(self.text(
                "rotation-save-consequence",
                "Saving this source will pause rotation.",
            ));
        } else if self.rotation_dirty() && self.rotation.wants_running {
            body = body.child(self.text(
                "rotation-save-consequence",
                "Saving changes starts a fresh countdown.",
            ));
        }
        if rotation::minutes(&self.rotation.minutes).is_err() {
            body = body.child(
                div()
                    .text_color(rgb(0xf0b2ac))
                    .child(self.text("rotation-invalid", "Enter a whole number from 1 to 1,440.")),
            );
        }
        if draft.source == RotationSource::Selected {
            body = body.child(self.text(
                "rotation-selected-count",
                format!("{} selected artworks", draft.selected.len()),
            ));
            if draft.selected.len() < 2 {
                body = body.child(self.text(
                    "rotation-selected-help",
                    "Add at least 2 artworks using Add to rotation while browsing.",
                ));
            }
            for (i, art) in draft.selected.iter().enumerate() {
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .rounded(px(10.))
                        .bg(rgb(CANVAS))
                        .child(self.text(
                            &format!("rotation-art-{i}"),
                            format!("{}. {} · {}", i + 1, art.title(), art.artist()),
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(self.rotation_row_button(
                                    format!("rotation-up-{i}"),
                                    "Move up".into(),
                                    !busy && i > 0,
                                    cx,
                                ))
                                .child(self.rotation_row_button(
                                    format!("rotation-down-{i}"),
                                    "Move down".into(),
                                    !busy && i + 1 < draft.selected.len(),
                                    cx,
                                ))
                                .child(self.rotation_row_button(
                                    format!("rotation-remove-{i}"),
                                    "Remove".into(),
                                    !busy,
                                    cx,
                                )),
                        ),
                );
            }
        }
        if let Some(error) = &self.rotation.error
            && !self.rotation.prefs.configured
            && !self.rotation.pending
        {
            body = body.child(
                div()
                    .text_color(rgb(0xf0b2ac))
                    .child(self.text("rotation-error", error.clone())),
            );
        }
        if let Some(error) = pinacora::platform::support_error() {
            body = body.child(self.text("rotation-platform", error));
        }
        for (i, (art, error)) in self.rotation.skipped.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(self.text(
                        &format!("rotation-skipped-{i}"),
                        format!("Skipped {} · {error}", art.title()),
                    ))
                    .child(self.rotation_row_button(
                        format!("rotation-retry-skipped-{i}"),
                        format!("Retry {}", art.title()),
                        !busy && !self.rotation.pending,
                        cx,
                    )),
            );
        }
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_color(rgb(MUTED)).child(self.text(
                    "rotation-description",
                    "Keeps running when Pinacora is closed.",
                )))
                .child(div().flex().child(self.button(
                    "rotation-details",
                    if self.rotation.details_open {
                        "How rotation works  ▾"
                    } else {
                        "How rotation works  ›"
                    },
                    true,
                    cx,
                ))),
        );
        if self.rotation.details_open {
            body = body.child(div().flex().flex_col().gap_3().text_sm().text_color(rgb(MUTED))
                .child(self.text("rotation-source-copy", if draft.source == RotationSource::EntireGallery {
                    "Shuffle across the entire gallery, independently of your search. Artwork is not repeated within a cycle; newly discovered artwork joins the next cycle."
                } else {
                    "Selected artworks play in the order above. Use Add to rotation while browsing to grow your collection."
                }))
                .child(self.text("rotation-interval-help", "Use whole minutes from 1 to 1,440. Resume starts a fresh countdown. Saving an interval keeps paused rotation paused."))
                .child(self.text("rotation-cache-copy", "Pinacora downloads the next wallpaper in advance. If it is not ready, the current wallpaper stays. Downloaded originals remain on this computer."))
                .child(self.text("rotation-login-copy", "The background service restores active rotation at login. Pause suspends changes; Stop turns rotation off and keeps your collection.")));
            if let Some(eligible) = self.rotation.eligible {
                body = body
                    .child(self.text("rotation-eligible", format!("{eligible} eligible artworks")));
            }
            if let Some(error) = &self.rotation.cache_diagnostic {
                body = body
                    .child(self.text("rotation-cache-error", format!("Catalogue cache: {error}")));
            }
        }
        let mut footer = div()
            .flex_none()
            .px_6()
            .py_4()
            .bg(rgb(0x1b1b1e))
            .border_t_1()
            .border_color(rgb(RAISED))
            .flex()
            .gap_2()
            .items_center();
        if self.rotation.prefs.configured && self.rotation.state != RotationState::Stopped {
            footer = footer.child(self.button(
                "rotation-stop",
                "Stop rotation",
                !busy && !self.rotation.pending,
                cx,
            ));
        }
        footer = footer.child(div().flex_1()).child(self.button(
            "rotation-close",
            if self.rotation_dirty() {
                "Cancel"
            } else {
                "Close"
            },
            true,
            cx,
        ));
        if self.rotation_dirty()
            && draft.source == RotationSource::Selected
            && draft.selected.len() < 2
        {
            footer = footer.child(self.button(
                "rotation-stop-save",
                "Stop rotation and save",
                !busy && rotation::minutes(&self.rotation.minutes).is_ok(),
                cx,
            ));
        } else if self.rotation_dirty() && self.rotation.prefs.configured {
            footer = footer.child(self.button("rotation-save", "Save changes", valid && !busy, cx));
        }
        if !self.rotation.prefs.configured || self.rotation.state == RotationState::Stopped {
            footer =
                footer.child(self.button("rotation-start", "Start rotation", valid && !busy, cx));
        }
        div()
            .id("rotation-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x000000bb))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .on_key_down(cx.listener(Self::rotation_key))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .w(px(560.))
                    .max_w_full()
                    .max_h(px(f32::from(window.viewport_size().height) - 48.))
                    .bg(rgb(SURFACE))
                    .rounded(px(16.))
                    .border_1()
                    .border_color(rgb(0x3a3a40))
                    .shadow(vec![BoxShadow {
                        color: rgba(0x00000066).into(),
                        offset: point(px(0.), px(16.)),
                        blur_radius: px(48.),
                        spread_radius: px(0.),
                    }])
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .id("rotation-content")
                            .min_h_0()
                            .w_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.rotation.scroll)
                            .child(body),
                    )
                    .child(footer),
            )
            .into_any_element()
    }
    fn rotation_live_card(&self, cx: &Context<Self>) -> AnyElement {
        let status_color = match self.rotation.state {
            RotationState::Running => 0x82cca4,
            RotationState::Failed => 0xf0b2ac,
            RotationState::Paused | RotationState::Stopped => 0xc0b7a7,
            _ => 0x79b7ff,
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(7.))
                                    .h(px(7.))
                                    .flex_none()
                                    .rounded_full()
                                    .bg(rgb(status_color)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_color(rgb(status_color))
                                    .child(self.text("rotation-status", self.rotation_label())),
                            ),
                    )
                    .child(self.rotation_actions(cx)),
            )
            .when(self.rotation.wants_running && !self.rotation.pending, |d| {
                if let Some(next) = self.rotation_next() {
                    d.child(div().text_sm().text_color(rgb(MUTED)).child(self.text(
                        "rotation-next",
                        format!("Next: {} · {}", next.title(), next.artist()),
                    )))
                } else {
                    d
                }
            })
            .when_some(self.rotation.error.as_ref(), |d, error| {
                d.child(
                    div()
                        .text_color(rgb(0xf0b2ac))
                        .child(self.text("rotation-live-error", error.clone())),
                )
            })
            .when(
                !self.rotation.notice.is_empty()
                    && self.rotation.notice
                        != "Rotation continues after closing Pinacora. Pause or stop it here.",
                |d| {
                    d.child(
                        div()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(self.text("rotation-notice", self.rotation.notice.clone())),
                    )
                },
            )
            .into_any_element()
    }
    fn rotation_actions(&self, cx: &Context<Self>) -> AnyElement {
        let busy = self.applying
            || self.rotation.command_busy
            || self.rotation.state == RotationState::Preparing;
        let mut actions = div().flex().flex_wrap().gap_2();
        if self.rotation.wants_running || self.rotation.pending {
            actions = actions.child(self.button(
                "rotation-pause",
                if self.rotation.pending {
                    "Cancel preparation"
                } else {
                    "Pause"
                },
                !self.applying && !self.rotation.command_busy,
                cx,
            ));
        } else if self.rotation.prefs.configured {
            actions = actions.child(self.button(
                "rotation-resume",
                "Resume",
                !busy && !self.rotation.pending,
                cx,
            ));
        }
        if self.rotation.wants_running {
            actions = actions.child(self.button(
                "rotation-now",
                "Change now",
                !busy && self.rotation.prepared_path.is_some() && self.rotation.wants_running,
                cx,
            ));
        }
        if self.rotation.error.is_some() {
            actions = actions.child(self.button(
                "rotation-retry",
                if self.rotation.pending {
                    "Retry preparation"
                } else if self.rotation.state == RotationState::Failed {
                    "Retry change"
                } else {
                    "Retry now"
                },
                !busy && !self.rotation.command_busy,
                cx,
            ));
        }
        actions.into_any_element()
    }
    fn rotation_label(&self) -> String {
        match self.rotation.state {
            RotationState::Stopped => "Off".into(),
            RotationState::Paused => "Paused".into(),
            RotationState::Preparing => "Preparing rotation…".into(),
            RotationState::Waiting => {
                if self.rotation.pending {
                    "Preparing next wallpaper · your current wallpaper stays in place".into()
                } else {
                    "Waiting for internet · your current wallpaper stays in place".into()
                }
            }
            RotationState::Applying => "Applying wallpaper…".into(),
            RotationState::Failed => if self.rotation.pending {
                "Rotation paused · preparation failed"
            } else {
                "Rotation paused · change failed"
            }
            .into(),
            RotationState::Running => {
                let remaining = self
                    .rotation
                    .deadline
                    .and_then(|deadline| deadline.duration_since(std::time::SystemTime::now()).ok())
                    .unwrap_or_default()
                    .as_secs();
                format!("Next change in {}:{:02}", remaining / 60, remaining % 60)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_recheck_failure_keeps_offer_and_busy_work_cannot_overlap() {
        let release = serde_json::from_value(serde_json::json!({
            "version": "9.0.0",
            "html_url": "https://github.com/gitfudge0/pinacora/releases/tag/v9.0.0",
            "asset": {"name": "fixture.zip", "browser_download_url": "https://github.com/gitfudge0/pinacora/releases/download/v9.0.0/fixture.zip", "size": 1},
            "checksum": null
        })).unwrap();
        let mut updates = Updates::default();
        assert!(updates.begin_check());
        assert!(!updates.begin_check());
        updates.checked(Ok(Some(release)));
        assert_eq!(updates.release.as_ref().unwrap().version, "9.0.0");
        assert!(updates.begin_check());
        updates.checked(Err("offline".into()));
        assert_eq!(updates.release.as_ref().unwrap().version, "9.0.0");
        assert_eq!(updates.error.as_deref(), Some("offline"));
        updates.installing = true;
        assert!(!updates.begin_check());
        updates.installing = false;
        assert!(updates.begin_check());
        updates.checked(Ok(None));
        assert!(updates.release.is_none());
        assert!(updates.error.is_none());
        assert!(updates.checked);
    }
    fn art() -> Artwork {
        Artwork {
            id: "one".into(),
            key: "originals/Édouard Manet - Café.jpg".into(),
            alt: "Café scene".into(),
            href: "/art/one".into(),
        }
    }
    #[test]
    fn hover_reversal_starts_at_the_current_scale() {
        let entering = hover_scale(1., true, eased(0.4));
        assert_eq!(hover_scale(entering, false, 0.), entering);
        assert_eq!(hover_scale(entering, false, 1.), 1.);
        assert_eq!(hover_scale(1., true, 1.), 1.025);
        assert!(entering > 1. && entering < 1.025);
    }
    #[test]
    fn delayed_reveal_stays_bounded_and_finishes() {
        assert_eq!(reveal_progress(0., 0.16), 0.);
        assert_eq!(reveal_progress(0.1, 0.16), 0.);
        assert_eq!(reveal_progress(1., 0.16), 1.);
        assert!(reveal_progress(0.5, 0.16) > 0.);
        assert_eq!(eased(-0.5), 0.);
        assert_eq!(eased(1.5), 1.);
    }
    #[test]
    fn duplicate_page_does_not_remove_items_or_selection() {
        let a = art();
        let mut loaded = vec![a.clone()];
        let returned = vec![a.clone()];
        assert!(!returned.is_empty());
        assert!(append_unique(&mut loaded, returned).is_empty());
        assert_eq!(loaded.len(), 1);
        assert!(selection_retained(&loaded, Some("one")));
        assert!(!selection_retained(&loaded, Some("missing")));
    }
    #[test]
    fn failed_page_retries_without_advancing() {
        assert_eq!(request_page(3, false), 4);
        assert_eq!(request_page(3, false), 4);
        assert_eq!(request_page(3, true), 1);
    }
    #[test]
    fn site_matches_are_displayed_without_local_title_filtering() {
        let mut search = SiteSearch::default();
        let epoch = search.begin("landscape");
        assert!(search.complete(epoch, Ok(vec![art()])));
        assert_eq!(search.results[0].title(), "Café scene");
        let epoch = search.begin("unmatched");
        assert!(search.complete(epoch, Ok(vec![])));
        assert!(search.results.is_empty());
        assert!(!search.loading);
        assert!(search.error.is_none());
    }
    #[test]
    fn search_epochs_discard_clear_retype_and_out_of_order_results() {
        let mut search = SiteSearch::default();
        assert!(search.browse_can_update_view(0));
        let old = search.begin(" café ");
        assert!(!search.browse_can_update_view(0));
        search.begin(" ");
        assert!(!search.loading);
        assert!(!search.browse_can_update_view(0));
        assert!(search.browse_can_update_view(search.epoch));
        let latest = search.begin("café");
        assert!(!search.complete(old, Ok(vec![art()])));
        assert!(search.results.is_empty());
        assert!(search.complete(latest, Err("offline".into())));
        assert!(!search.loading);
        assert_eq!(search.error.as_deref(), Some("offline"));
        let retry = search.begin("café");
        assert!(search.error.is_none());
        assert!(search.complete(retry, Ok(vec![art()])));
        assert_eq!(search.results.len(), 1);
        let short = search.begin("é");
        assert!(!search.current(short));
        assert!(search.results.is_empty());
    }
}
