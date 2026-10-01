use crate::accessibility::{
    AccessibilityAction, AccessibilityBridge, AccessibilityNode, AccessibilityRole,
};
use crate::onboarding::{self, Outcome};
use crate::search_input::{Changed, Navigate, TextInput};
use gpui::{
    Animation, AnimationExt, AnyElement, Context, Entity, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, ObjectFit, ScrollHandle, SharedString, Window, WindowControlArea, div, img,
    linear_color_stop, linear_gradient, point, prelude::*, px, rgb, rgba,
};
use reframed::catalogue::{Artwork, Detail};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    time::Duration,
};
fn app_icon(edge: f32) -> gpui::Img {
    static ICON: OnceLock<Arc<gpui::Image>> = OnceLock::new();
    img(ICON
        .get_or_init(|| {
            Arc::new(gpui::Image::from_bytes(
                gpui::ImageFormat::Png,
                include_bytes!("../resources/app-icon.png").to_vec(),
            ))
        })
        .clone())
    .w(px(edge))
    .h(px(edge))
    .object_fit(ObjectFit::Contain)
}

pub struct Gallery {
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

    status_epoch: u64,
    hero_queued: Option<(Artwork, u64)>,
    intro_visible: bool,
    intro_step: usize,
    intro_epoch: u64,
    intro_backdrop_epoch: u64,
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
            "intro-skip",
            "intro-back",
            "intro-next",
        ]
        .into_iter()
        .map(|id| (id.to_owned(), cx.focus_handle().tab_stop(true)))
        .collect();
        let mut app = Self {
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

            status_epoch: 0,
            hero_queued: None,
            intro_visible,
            intro_step: 0,
            intro_epoch: 0,
            intro_backdrop_epoch: 0,
            intro_focus: cx.focus_handle(),
            intro_save_error: None,
        };
        app.load(true, cx);
        app
    }
    fn show_intro(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying {
            return;
        }
        self.intro_step = 0;
        self.intro_epoch += 1;
        self.intro_backdrop_epoch += 1;
        self.intro_visible = true;
        self.intro_focus.focus(window);
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
    fn intro_next(&mut self, cx: &mut Context<Self>) {
        if self.intro_step >= 3 {
            self.close_intro(Outcome::Completed, cx);
        } else {
            self.intro_step += 1;
            self.intro_epoch += 1;
            cx.notify();
        }
    }
    fn intro_back(&mut self, cx: &mut Context<Self>) {
        if self.intro_step > 0 {
            self.intro_step -= 1;
            self.intro_epoch += 1;
            cx.notify();
        }
    }
    fn intro_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "tab" => self.focus_step(event.keystroke.modifiers.shift, window, cx),
            "escape" => self.close_intro(Outcome::Skipped, cx),
            "enter" | "right" => self.intro_next(cx),
            "left" => self.intro_back(cx),
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
                reframed::catalogue::fetch_search(&query)
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
            reframed::catalogue::fetch_page(page).map_err(|e| format!("{e:#}"))
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
                reframed::download::fetch(&url, false)
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
            .spawn(async move { reframed::download::fetch(&url, false).map(|(path, _, _)| path) });
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
            reframed::catalogue::fetch_detail(&art).map_err(|e| format!("{e:#}"))
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
        self.applying = true;
        self.status = "Downloading and validating the full resolution original…".into();
        let task = cx.background_executor().spawn(async move {
            reframed::download::fetch(&detail.original, true).map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                // AppKit is called from GPUI's foreground/main thread, after network work completes.
                app.status = match result {
                    Ok((path, w, h)) => match reframed::platform::apply(&path) {
                        Ok(n) => format!("Wallpaper set on {n} display(s) · {w} × {h}"),
                        Err(e) => format!("Could not apply wallpaper: {e:#}"),
                    },
                    Err(e) => format!("Could not download original: {e}"),
                };
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
        if self.intro_visible {
            return false;
        }
        if e.keystroke.modifiers.platform && e.keystroke.key == "f" {
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
        if self.applying {
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
                if app.selection_epoch == epoch && app.selected_result_preview {
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
        div()
            .relative()
            .id(id)
            .track_focus(
                &self.controls[id]
                    .clone()
                    .tab_stop(enabled && !self.intro_visible)
                    .tab_index(control_tab_index(id)),
            )
            .tab_index(0)
            .tab_stop(enabled)
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(SURFACE))
            .when(matches!(id, "motion-toggle" | "show-walkthrough"), |d| {
                d.bg(rgba(0xffffff00)).text_xs().text_color(rgb(0xb4bcb5))
            })
            .when(id == "apply", |d| {
                d.px_5()
                    .py_3()
                    .bg(rgb(TEXT))
                    .text_color(rgb(CANVAS))
                    .font_weight(FontWeight::MEDIUM)
            })
            .text_sm()
            .when(enabled, |d| d.cursor_pointer())
            .opacity(if enabled { 1. } else { 0.4 })
            .focus(|s| s.border_1().border_color(rgb(TEXT)))
            .child(label)
            .child(self.semantic(
                id.into(),
                ax_label,
                AccessibilityRole::Button,
                String::new(),
                enabled,
                false,
                false,
            ))
            .on_click(cx.listener(move |app, _, window, cx| {
                if enabled {
                    app.controls[id].focus(window);
                    app.activate(id, window, cx);
                }
            }))
            .on_key_down(cx.listener(move |app, e: &KeyDownEvent, window, cx| {
                if app.controls[id].is_focused(window) && app.navigation_key(e, window, cx) {
                    return;
                }
                if enabled
                    && app.controls[id].is_focused(window)
                    && matches!(e.keystroke.key.as_str(), "enter" | "space")
                {
                    cx.stop_propagation();
                    app.activate(id, window, cx);
                }
            }))
            .into_any_element()
    }
    fn activate(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            "back-to-results" => self.back_to_results(window, cx),
            "intro-motion" => self.toggle_motion(cx),
            "intro-skip" => self.close_intro(Outcome::Skipped, cx),
            "intro-back" => self.intro_back(cx),
            "intro-next" => self.intro_next(cx),
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

const CANVAS: u32 = 0x090a0a;
const SURFACE: u32 = 0x171819;
const TEXT: u32 = 0xf4f4f0;
const MUTED: u32 = 0x858986;

impl Gallery {
    fn header(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .h(px(72.))
            .flex_none()
            .pl(px(100.))
            .pr_6()
            .flex()
            .items_center()
            .gap_2()
            .child(app_icon(28.))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Reframed"),
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
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0x393d3a))
                    .overflow_hidden()
                    .relative()
                    .child(self.search_input.clone())
                    .child(self.semantic(
                        "search".into(),
                        "Search Reframed".into(),
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
        let height = (f32::from(window.viewport_size().width) * 0.375).clamp(405., 520.);
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
                view = view.child(
                    img(path.clone())
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                );
            }
            view = view
                .child(div().absolute().inset_0().bg(linear_gradient(
                    90.,
                    linear_color_stop(rgba(0x090a0aee), 0.),
                    linear_color_stop(rgba(0x090a0a00), 0.82),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0x090a0a88), 0.),
                    linear_color_stop(rgba(0x090a0a00), 0.3),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0x090a0a00), 0.35),
                    linear_color_stop(rgba(0x090a0aff), 0.92),
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
                                    .text_color(rgb(0xd1d5d0))
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
                                d.child(self.text("detail-progress", "Loading artwork details…"))
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
                        div()
                            .flex()
                            .gap_3()
                            .child(self.button(
                                "apply",
                                if self.applying {
                                    "Downloading…"
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
                            .child(self.button("view-source", "View source", true, cx)),
                    )
                    .child(div().text_xs().text_color(rgb(0xb2bbb2)).child(self.text(
                        "display-scope",
                        "Untouched original · All connected displays",
                    )))
                    .when(!self.status.is_empty(), |d| {
                        d.child(
                            div()
                                .id("apply-status-scroll")
                                .h(px(56.))
                                .text_sm()
                                .text_color(rgb(0xc3d4c5))
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
            Some(Ok(path)) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
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
        div()
            .relative()
            .id(SharedString::from(format!("tile-{}", art.id)))
            .track_focus(
                &self.tiles[&art.id]
                    .clone()
                    .tab_stop(
                        !self.applying
                            && !self.intro_visible
                            && self.keyboard_tile.as_ref() == Some(&art.id),
                    )
                    .tab_index(30),
            )
            .tab_index(0)
            .tab_stop(self.keyboard_tile.as_ref() == Some(&art.id))
            .w(px(width))
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .p_1()
            .rounded_lg()
            .cursor_pointer()
            .border_1()
            .border_color(if selected {
                rgb(TEXT)
            } else {
                rgba(0xffffff00)
            })
            .focus(|s| s.border_color(rgb(0x98b8a8)))
            .hover(|s| s.bg(rgb(SURFACE)))
            .child(
                div()
                    .w_full()
                    .h(px((width - 10.) * 9. / 16.))
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgb(SURFACE))
                    .child(picture),
            )
            .child(
                div()
                    .text_sm()
                    .line_height(px(19.))
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
    fn intro(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let (title, description) = match self.intro_step {
            0 => (
                "Reframed",
                "Browse artwork from Reframed and set a full-resolution original as your wallpaper.",
            ),
            1 => (
                "Find your next view",
                "Browse the artwork grid below the cinematic preview. Search Reframed by title or artist. Selecting a result brings its preview into view; Back to results restores your browsing position.",
            ),
            2 => (
                "Make it yours",
                "Select an artwork to preview it. Set wallpaper downloads the untouched full-resolution original and applies it to every currently connected display.",
            ),
            _ => (
                "Keep exploring",
                "Load more artwork to expand the gallery. View source opens the original page, where you can explore the artwork and its attribution.",
            ),
        };
        let path = self.selected.as_ref().and_then(|id| {
            self.hero_previews
                .get(id)
                .or_else(|| self.previews.get(id).and_then(|p| p.as_ref().ok()))
        });
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        let background = path.map(|path| {
            let image = img(path.clone())
                .absolute()
                .object_fit(ObjectFit::Cover)
                .opacity(0.35);
            if self.motion_enabled {
                // The session key remains stable across manual steps and resolution upgrades.
                image
                    .with_animation(
                        SharedString::from(format!("intro-backdrop-{}", self.intro_backdrop_epoch)),
                        Animation::new(Duration::from_secs(24)).repeat(),
                        move |image, delta| {
                            let phase = std::f32::consts::TAU * delta;
                            let scale = 1.0525 + 0.0125 * phase.sin();
                            let drift = 8f32.min((width.min(height) * 0.02 - 1.).max(0.));
                            image
                                .w(px(width * scale))
                                .h(px(height * scale))
                                .left(px((width - width * scale) * 0.5 + drift * phase.sin()))
                                .top(px((height - height * scale) * 0.5 + drift * phase.cos()))
                        },
                    )
                    .into_any_element()
            } else {
                image
                    .left_0()
                    .top_0()
                    .w(px(width))
                    .h(px(height))
                    .into_any_element()
            }
        });
        let title_element = div()
            .relative()
            .child(self.semantic(
                "intro-title".into(),
                title.into(),
                AccessibilityRole::StaticText,
                String::new(),
                true,
                false,
                false,
            ))
            .text_size(px(if self.intro_step == 0 { 54. } else { 36. }))
            .line_height(px(if self.intro_step == 0 { 62. } else { 44. }))
            .font_weight(FontWeight::SEMIBOLD)
            .child(title);
        let copy = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_5()
            .children((self.intro_step == 0).then(|| app_icon(88.)))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0xa8b2a9))
                    .child(if self.intro_step == 0 {
                        "WELCOME".to_owned()
                    } else {
                        format!("{} / 3", self.intro_step)
                    }),
            )
            .child(title_element)
            .child(
                div()
                    .text_lg()
                    .line_height(px(28.))
                    .text_color(rgb(0xc3ccc4))
                    .child(self.text("intro-description", description)),
            );
        let copy = if self.motion_enabled {
            copy.with_animation(
                SharedString::from(format!("intro-step-{}", self.intro_epoch)),
                Animation::new(Duration::from_millis(if self.intro_step == 0 {
                    700
                } else {
                    320
                }))
                .with_easing(|t| 1. - (1. - t).powi(3)),
                |e, t| e.opacity(t).relative().top(px(12. * (1. - t))),
            )
            .into_any_element()
        } else {
            copy.into_any_element()
        };
        div()
            .id("walkthrough-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgb(CANVAS))
            .overflow_hidden()
            .track_focus(&self.intro_focus)
            .on_key_down(cx.listener(Self::intro_key))
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .children(background)
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(rgba(0x090a0ae6), 0.),
                linear_color_stop(rgba(0x090a0a99), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .top(px(16.))
                    .right(px(24.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("intro-motion")
                            .on_key_down(cx.listener(|app, e: &KeyDownEvent, window, cx| {
                                if e.keystroke.key == "tab" {
                                    cx.stop_propagation();
                                    app.focus_step(e.keystroke.modifiers.shift, window, cx);
                                    return;
                                }
                                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                                    app.activate("intro-motion", window, cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .relative()
                            .track_focus(
                                &self.controls["intro-motion"]
                                    .clone()
                                    .tab_stop(true)
                                    .tab_index(control_tab_index("intro-motion")),
                            )
                            .tab_index(0)
                            .child(self.semantic(
                                "intro-motion".into(),
                                "Motion".into(),
                                AccessibilityRole::Button,
                                String::new(),
                                true,
                                false,
                                false,
                            ))
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(0xa8b2a9))
                            .cursor_pointer()
                            .child(if self.motion_enabled {
                                "Motion on"
                            } else {
                                "Motion off"
                            })
                            .on_click(cx.listener(|app, _, _, cx| app.toggle_motion(cx))),
                    )
                    .child(
                        div()
                            .id("intro-skip")
                            .on_key_down(cx.listener(|app, e: &KeyDownEvent, window, cx| {
                                if e.keystroke.key == "tab" {
                                    cx.stop_propagation();
                                    app.focus_step(e.keystroke.modifiers.shift, window, cx);
                                    return;
                                }
                                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                                    app.activate("intro-skip", window, cx);
                                    cx.stop_propagation();
                                }
                            }))
                            .relative()
                            .track_focus(
                                &self.controls["intro-skip"]
                                    .clone()
                                    .tab_stop(true)
                                    .tab_index(control_tab_index("intro-skip")),
                            )
                            .tab_index(0)
                            .child(self.semantic(
                                "intro-skip".into(),
                                "Skip".into(),
                                AccessibilityRole::Button,
                                String::new(),
                                true,
                                false,
                                false,
                            ))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgba(0xffffff14))
                            .text_sm()
                            .cursor_pointer()
                            .hover(|s| s.bg(rgba(0xffffff28)))
                            .child("Skip")
                            .on_click(
                                cx.listener(|app, _, _, cx| app.close_intro(Outcome::Skipped, cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top(px(72.))
                    .bottom(px(32.))
                    .left(px(32.))
                    .right(px(32.))
                    .flex()
                    .justify_center()
                    .items_center()
                    .child(
                        div()
                            .w(px(560.))
                            .flex()
                            .flex_col()
                            .gap_8()
                            .child(copy)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .id("intro-back")
                                            .on_key_down(cx.listener(
                                                |app, e: &KeyDownEvent, window, cx| {
                                                    if e.keystroke.key == "tab" {
                                                        cx.stop_propagation();
                                                        app.focus_step(
                                                            e.keystroke.modifiers.shift,
                                                            window,
                                                            cx,
                                                        );
                                                        return;
                                                    }
                                                    if matches!(
                                                        e.keystroke.key.as_str(),
                                                        "enter" | "space"
                                                    ) {
                                                        app.activate("intro-back", window, cx);
                                                        cx.stop_propagation();
                                                    }
                                                },
                                            ))
                                            .relative()
                                            .track_focus(
                                                &self.controls["intro-back"]
                                                    .clone()
                                                    .tab_stop(self.intro_step > 0)
                                                    .tab_index(control_tab_index("intro-back")),
                                            )
                                            .tab_index(0)
                                            .tab_stop(self.intro_step > 0)
                                            .child(self.semantic(
                                                "intro-back".into(),
                                                "Back".into(),
                                                AccessibilityRole::Button,
                                                String::new(),
                                                self.intro_step > 0,
                                                false,
                                                false,
                                            ))
                                            .px_4()
                                            .py_3()
                                            .rounded_md()
                                            .text_sm()
                                            .cursor_pointer()
                                            .opacity(if self.intro_step > 0 { 1. } else { 0. })
                                            .child("Back")
                                            .on_click(
                                                cx.listener(|app, _, _, cx| app.intro_back(cx)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("intro-next")
                                            .on_key_down(cx.listener(
                                                |app, e: &KeyDownEvent, window, cx| {
                                                    if e.keystroke.key == "tab" {
                                                        cx.stop_propagation();
                                                        app.focus_step(
                                                            e.keystroke.modifiers.shift,
                                                            window,
                                                            cx,
                                                        );
                                                        return;
                                                    }
                                                    if matches!(
                                                        e.keystroke.key.as_str(),
                                                        "enter" | "space"
                                                    ) {
                                                        app.activate("intro-next", window, cx);
                                                        cx.stop_propagation();
                                                    }
                                                },
                                            ))
                                            .relative()
                                            .track_focus(
                                                &self.controls["intro-next"]
                                                    .clone()
                                                    .tab_stop(true)
                                                    .tab_index(control_tab_index("intro-next")),
                                            )
                                            .tab_index(0)
                                            .child(self.semantic(
                                                "intro-next".into(),
                                                "Continue walkthrough".into(),
                                                AccessibilityRole::Button,
                                                String::new(),
                                                true,
                                                false,
                                                false,
                                            ))
                                            .px_5()
                                            .py_3()
                                            .rounded_md()
                                            .bg(rgb(TEXT))
                                            .text_color(rgb(CANVAS))
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .cursor_pointer()
                                            .child(if self.intro_step == 0 {
                                                "Continue"
                                            } else if self.intro_step == 3 {
                                                "Start exploring"
                                            } else {
                                                "Next"
                                            })
                                            .on_click(
                                                cx.listener(|app, _, _, cx| app.intro_next(cx)),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x858f86))
                                    .child("Escape to skip · Enter to continue"),
                            ),
                    ),
            )
            .into_any_element()
    }
}
impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.intro_visible
            && (window.focused(cx).is_none() || self.intro_focus.is_focused(window))
        {
            self.focus.focus(window);
        }
        if !self.navigation_subscribed {
            self.navigation_subscribed = true;
            cx.subscribe_in(
                &self.search_input,
                window,
                |app, _, event: &Navigate, window, cx| app.focus_step(event.0, window, cx),
            )
            .detach();
        }
        self.search_input.update(cx, |input, cx| {
            input.set_tab_enabled(!self.intro_visible, cx)
        });
        self.ax_nodes.borrow_mut().clear();
        if let Some(receiver) = self.ax_receiver.take() {
            *self.ax.borrow_mut() = AccessibilityBridge::new(window, self.ax_sender.clone());
            let this = cx.entity().downgrade();
            window
                .spawn(cx, async move |cx| {
                    while let Ok(action) = receiver.recv().await {
                        let result = this.update_in(cx, |app, window, cx| {
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
                                    if id == "search" {
                                        app.search_input
                                            .update(cx, |input, cx| input.set_value(value, cx));
                                    }
                                }
                                AccessibilityAction::SetSelection(id, start, length) => {
                                    if id == "search" {
                                        app.search_input.update(cx, |input, cx| {
                                            input.set_selection(start, length, cx)
                                        });
                                    }
                                }
                                AccessibilityAction::Focus(id) => {
                                    if id == "search" {
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
        let intro_visible = self.intro_visible;
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
        if self.intro_visible && !self.intro_focus.contains_focused(window, cx) {
            self.intro_focus.focus(window);
        }
        div()
            .relative()
            .size_full()
            .bg(rgb(CANVAS))
            .text_color(rgb(TEXT))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
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
                            .bg(rgba(0x090a0ae8))
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
                    .bg(rgba(0x090a0a99))
                    .child(self.header(cx)),
            )
            .when(self.intro_visible, |d| d.child(self.intro(window, cx)))
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
                                    || node.id.starts_with("intro-");
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
                                if node.id == "search" {
                                    node.focused = input.focus_handle(cx).is_focused(window);
                                    node.selected_range = Some(input.read(cx).utf16_selection());
                                } else if let Some(handle) = controls.get(&node.id).or_else(|| {
                                    node.id.strip_prefix("tile-").and_then(|id| tiles.get(id))
                                }) {
                                    node.focused = handle.is_focused(window);
                                }
                            }
                            if intro_visible {
                                snapshot.retain(|node| node.id.starts_with("intro-"));
                            } else {
                                snapshot.retain(|node| !node.id.starts_with("intro-"));
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
        "motion-toggle" | "intro-motion" => 0,
        "show-walkthrough" | "intro-skip" => 1,
        "clear-search" | "intro-back" => 3,
        "refresh" | "intro-next" => 4,
        "show-new" => 10,
        "retry-catalogue" => 11,
        "back-to-results" => 20,
        "retry-preview" => 21,
        "retry-details" => 22,
        "apply" => 23,
        "view-source" => 24,
        "empty-clear-search" => 31,
        "load-more" => 40,
        _ => 50,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn art() -> Artwork {
        Artwork {
            id: "one".into(),
            key: "originals/Édouard Manet - Café.jpg".into(),
            alt: "Café scene".into(),
            href: "/art/one".into(),
        }
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
