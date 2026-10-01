use crate::onboarding::{self, Outcome};
use gpui::{
    Animation, AnimationExt, AnyElement, Context, FocusHandle, FontWeight, KeyDownEvent, ObjectFit,
    ScrollHandle, SharedString, Window, WindowControlArea, div, img, linear_color_stop,
    linear_gradient, point, prelude::*, px, rgb, rgba,
};
use reframed::catalogue::{Artwork, Detail};
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
                include_bytes!("../resources/app-icon.png").to_vec(),
            ))
        })
        .clone())
    .w(px(edge))
    .h(px(edge))
    .object_fit(ObjectFit::Contain)
}

struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
}
impl Tween {
    fn value(&self) -> f32 {
        let t = (self.start.elapsed().as_secs_f32() / self.duration.as_secs_f32()).min(1.);
        self.from + (self.to - self.from) * (1. - (1. - t).powi(3))
    }
    fn active(&self) -> bool {
        self.start.elapsed() < self.duration && self.from != self.to
    }
}
pub struct Gallery {
    artworks: Vec<Artwork>,
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
    search_all: bool,
    end: bool,
    selection_epoch: u64,
    recent_scroll: ScrollHandle,
    explore_scroll: ScrollHandle,
    hero_previews: HashMap<String, PathBuf>,
    hero_active: bool,
    motion_enabled: bool,
    interactions: HashMap<String, Tween>,
    shelf_motion: HashMap<&'static str, (ScrollHandle, Tween)>,
    selection_start: Instant,
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
        let mut app = Self {
            artworks: vec![],
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
            search_all: false,
            end: false,
            selection_epoch: 0,
            recent_scroll: ScrollHandle::new(),
            explore_scroll: ScrollHandle::new(),
            hero_previews: HashMap::new(),
            hero_active: false,
            motion_enabled: true,
            interactions: HashMap::new(),
            shelf_motion: HashMap::new(),
            selection_start: Instant::now(),
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
    fn intro_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        match event.keystroke.key.as_str() {
            "escape" => self.close_intro(Outcome::Skipped, cx),
            "enter" | "right" => self.intro_next(cx),
            "left" => self.intro_back(cx),
            _ => {}
        }
    }
    fn set_motion(
        &mut self,
        key: impl Into<String>,
        target: f32,
        duration: u64,
        cx: &mut Context<Self>,
    ) {
        let key = key.into();
        if self.interactions.get(&key).is_some_and(|t| t.to == target) {
            return;
        }
        let from = if self.motion_enabled {
            self.interactions.get(&key).map_or(0., Tween::value)
        } else {
            target
        };
        self.interactions.insert(
            key,
            Tween {
                from,
                to: target,
                start: Instant::now(),
                duration: Duration::from_millis(duration),
            },
        );
        cx.notify();
    }
    fn motion(&self, key: &str) -> f32 {
        self.interactions
            .get(key)
            .map_or(0., |t| if self.motion_enabled { t.value() } else { t.to })
    }
    fn selection_progress(&self) -> f32 {
        if !self.motion_enabled {
            return 1.;
        }
        let t = (self.selection_start.elapsed().as_secs_f32() / 0.35).min(1.);
        1. - (1. - t).powi(3)
    }
    fn toggle_motion(&mut self, cx: &mut Context<Self>) {
        self.motion_enabled = !self.motion_enabled;
        if !self.motion_enabled {
            for (_, (handle, tween)) in self.shelf_motion.drain() {
                handle.set_offset(point(px(tween.to), px(0.)));
            }
        }
        cx.notify();
    }
    fn load(&mut self, refresh: bool, cx: &mut Context<Self>) {
        if self.loading || self.applying || (!refresh && self.end) {
            return;
        }
        self.loading = true;
        self.status = "Loading the gallery…".into();
        let page = if refresh { 1 } else { self.page + 1 };
        let task = cx.background_executor().spawn(async move {
            reframed::catalogue::fetch_page(page).map_err(|e| format!("{e:#}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |app, cx| {
                app.loading = false;
                match result {
                    Ok(items) => {
                        if refresh {
                            app.end = false;
                            app.artworks.clear();
                            app.selection_epoch += 1;
                            app.selected = None;
                            app.detail = None;
                        }
                        app.page = page;
                        let mut added = vec![];
                        for item in items {
                            if !app.artworks.iter().any(|a| a.id == item.id) {
                                added.push(item.clone());
                                app.artworks.push(item);
                            }
                        }
                        app.end = added.is_empty();
                        app.status =
                            format!("{} artworks · Originals from Reframed", app.artworks.len());
                        for art in added {
                            app.preview(art, cx);
                        }
                        if app.selected.is_none()
                            && let Some(first) = app.artworks.first().cloned()
                        {
                            app.select(first, cx);
                        }
                    }
                    Err(e) => {
                        app.status = format!("Could not load gallery: {e}. Use Refresh to retry.")
                    }
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
        self.selection_start = Instant::now();
        let epoch = self.selection_epoch;
        self.selected = Some(art.id.clone());
        self.hero_queued = None;
        if !self.hero_previews.contains_key(&art.id) {
            self.hero_queued = Some((art.clone(), epoch));
            self.pump_hero(cx);
        }
        self.detail = None;
        self.status = "Loading artwork details…".into();
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
                match result {
                    Ok(detail) => {
                        app.detail = Some(detail);
                        app.status = "Ready to download the untouched original".into();
                    }
                    Err(e) => app.status = e,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        if self.applying || self.loading || self.intro_visible {
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
    fn key(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.intro_visible || self.applying || e.keystroke.modifiers.control {
            return;
        }
        self.shelf_motion.clear();
        self.recent_scroll.set_offset(point(px(0.), px(0.)));
        self.explore_scroll.set_offset(point(px(0.), px(0.)));
        if e.keystroke.modifiers.platform {
            match e.keystroke.key.as_str() {
                "a" => self.search_all = true,
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        if self.search_all {
                            self.query.clear();
                            self.search_all = false;
                        }
                        self.query.push_str(&text.replace(['\n', '\r'], " "));
                    }
                }
                _ => return,
            }
            cx.notify();
            return;
        }
        match e.keystroke.key.as_str() {
            "backspace" => {
                if self.search_all {
                    self.query.clear();
                    self.search_all = false;
                } else {
                    self.query.pop();
                }
            }
            "escape" => self.query.clear(),
            _ => {
                if let Some(text) = &e.keystroke.key_char {
                    if self.search_all {
                        self.query.clear();
                        self.search_all = false;
                    }
                    self.query.push_str(text);
                }
            }
        }
        cx.notify();
    }
}

const CANVAS: u32 = 0x090a0a;
const SURFACE: u32 = 0x171819;
const TEXT: u32 = 0xf4f4f0;
const MUTED: u32 = 0x858986;

fn fade<E: IntoElement + Styled + 'static>(
    element: E,
    key: SharedString,
    millis: u64,
    enabled: bool,
) -> AnyElement {
    if enabled {
        element
            .with_animation(
                key,
                Animation::new(Duration::from_millis(millis))
                    .with_easing(|t| 1. - (1. - t).powi(3)),
                |e, t| e.opacity(t),
            )
            .into_any_element()
    } else {
        element.into_any_element()
    }
}
fn blend(a: u32, b: u32, t: f32) -> gpui::Rgba {
    let channel = |shift| {
        let from = ((a >> shift) & 255u32) as f32;
        let to = ((b >> shift) & 255u32) as f32;
        (from + (to - from) * t).round() as u32
    };
    rgb((channel(16) << 16) | (channel(8) << 8) | channel(0))
}
impl Gallery {
    fn search(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let _ = window;
        div()
            .id("search")
            .w(px(218.))
            .h(px(34.))
            .flex()
            .items_center()
            .px_3()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key))
            .on_click(cx.listener(|app, _, window, cx| {
                app.focus.focus(window);
                cx.notify();
            }))
            .rounded_md()
            .border_1()
            .border_color(blend(
                0x393d3a,
                0x858986,
                self.motion("search-focus").max(self.motion("search-hover")),
            ))
            .on_hover(cx.listener(|app, hover, _, cx| {
                app.set_motion("search-hover", if *hover { 0.55 } else { 0. }, 180, cx)
            }))
            .bg(rgba(0x171819cc))
            .text_sm()
            .text_color(rgb(if self.query.is_empty() {
                0xb0b6b0
            } else {
                TEXT
            }))
            .child(div().min_w_0().truncate().child(if self.query.is_empty() {
                "Search artwork or artist".to_owned()
            } else {
                self.query.clone()
            }))
            .into_any_element()
    }
    fn header(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(64.))
            .pl(px(100.))
            .pr_8()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_8()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(app_icon(32.))
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Reframed"),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(0xd1d5d0))
                            .child("Gallery"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .window_control_area(WindowControlArea::Drag),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("motion-toggle")
                            .px_2()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(0xb4bcb5))
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
                            .id("show-walkthrough")
                            .px_2()
                            .py_2()
                            .text_xs()
                            .text_color(rgb(0xb4bcb5))
                            .cursor_pointer()
                            .opacity(if self.applying { 0.4 } else { 1. })
                            .child("Guide")
                            .on_click(cx.listener(|app, _, window, cx| app.show_intro(window, cx))),
                    )
                    .child(self.search(window, cx))
                    .child(
                        div()
                            .id("refresh")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(blend(SURFACE, 0x303431, self.motion("refresh")))
                            .text_sm()
                            .cursor_pointer()
                            .opacity(if self.applying || self.loading {
                                0.4
                            } else {
                                1.
                            })
                            .on_hover(cx.listener(|app, hover, _, cx| {
                                app.set_motion("refresh", if *hover { 1. } else { 0. }, 180, cx)
                            }))
                            .child(if self.loading {
                                "Loading…"
                            } else {
                                "Refresh"
                            })
                            .on_click(cx.listener(|app, _, _, cx| app.load(true, cx))),
                    ),
            )
            .into_any_element()
    }
    fn hero(
        &self,
        art: Option<Artwork>,
        height: f32,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let picture = art.as_ref().and_then(|a| {
            self.hero_previews
                .get(&a.id)
                .or_else(|| self.previews.get(&a.id).and_then(|p| p.as_ref().ok()))
        });
        let background = match picture {
            Some(path) => fade(
                img(path.clone())
                    .absolute()
                    .inset_0()
                    .w_full()
                    .h(px(height))
                    .object_fit(ObjectFit::Cover),
                SharedString::from(format!(
                    "hero-image-{}-{}",
                    self.selection_epoch,
                    path.display()
                )),
                350,
                self.motion_enabled,
            ),
            None => div()
                .absolute()
                .inset_0()
                .w_full()
                .h(px(height))
                .bg(rgb(SURFACE))
                .into_any_element(),
        };
        let mut hero = div()
            .relative()
            .w_full()
            .h(px(height))
            .flex_none()
            .overflow_hidden()
            .bg(rgb(SURFACE))
            .child(background)
            .child(div().absolute().inset_0().bg(linear_gradient(
                90.,
                linear_color_stop(rgba(0x090a0ae8), 0.),
                linear_color_stop(rgba(0x090a0a00), 0.82),
            )))
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.,
                linear_color_stop(rgba(0x090a0a00), 0.42),
                linear_color_stop(rgb(CANVAS), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(120.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(rgba(0x090a0a99), 0.),
                        linear_color_stop(rgba(0x090a0a00), 1.),
                    )),
            )
            .child(self.header(window, cx));
        if let Some(art) = art {
            let title = self
                .detail
                .as_ref()
                .map(|d| d.title.clone())
                .unwrap_or(art.alt.clone());
            let artist = self
                .detail
                .as_ref()
                .map(|d| d.artist.clone())
                .unwrap_or_else(|| art.artist().to_owned());
            let ready = self.detail.is_some() && !self.applying && !self.loading;
            let failed =
                self.previews.get(&art.id).is_some_and(|p| p.is_err()) && picture.is_none();
            let retry = art.clone();
            let content_width = (f32::from(window.viewport_size().width) * 0.52).min(650.);
            hero = hero.child(
                div()
                    .absolute()
                    .left(px(32.))
                    .bottom(px(40. + 8. * self.selection_progress()))
                    .opacity(self.selection_progress())
                    .w(px(content_width))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(36.))
                            .line_height(px(42.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_clamp(3)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .text_sm()
                            .text_color(rgb(0xd1d5d0))
                            .child(div().min_w_0().truncate().child(artist))
                            .when_some(
                                self.detail.as_ref().and_then(|d| d.dimensions.clone()),
                                |d, dimensions| {
                                    d.child(
                                        div()
                                            .flex_none()
                                            .text_color(rgb(0x9da59e))
                                            .child(dimensions),
                                    )
                                },
                            ),
                    )
                    .when(failed, |d| {
                        d.child(
                            div()
                                .id("hero-retry")
                                .text_sm()
                                .text_color(rgb(0xc5cbc5))
                                .cursor_pointer()
                                .child("Preview unavailable · Retry")
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    if !app.applying {
                                        app.previews.remove(&retry.id);
                                        app.preview(retry.clone(), cx);
                                        app.select(retry.clone(), cx);
                                    }
                                })),
                        )
                    })
                    .when(picture.is_none() && !failed, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(rgb(0xa7afa8))
                                .child("Loading preview…"),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .mt_2()
                            .child(
                                div()
                                    .id("apply")
                                    .px_5()
                                    .py_3()
                                    .rounded_md()
                                    .bg(blend(TEXT, 0xdce2db, self.motion("apply")))
                                    .text_color(rgb(CANVAS))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_sm()
                                    .cursor_pointer()
                                    .opacity(if ready { 1. } else { 0.4 })
                                    .on_hover(cx.listener(|app, hover, _, cx| {
                                        app.set_motion(
                                            "apply",
                                            if *hover { 1. } else { 0. },
                                            180,
                                            cx,
                                        )
                                    }))
                                    .child(if self.applying {
                                        "Downloading…"
                                    } else {
                                        "Set wallpaper"
                                    })
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if ready {
                                            app.apply(cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .id("view-source")
                                    .px_4()
                                    .py_3()
                                    .rounded_md()
                                    .bg(rgba(
                                        0xffffff00 | ((24. + 19. * self.motion("source")) as u32),
                                    ))
                                    .text_sm()
                                    .cursor_pointer()
                                    .on_hover(cx.listener(|app, hover, _, cx| {
                                        app.set_motion(
                                            "source",
                                            if *hover { 1. } else { 0. },
                                            180,
                                            cx,
                                        )
                                    }))
                                    .opacity(if self.applying { 0.4 } else { 1. })
                                    .child("View source")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if !app.applying
                                            && let Ok(url) = art.page_url()
                                        {
                                            cx.open_url(&url);
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0xa7afa8))
                            .child("All connected displays"),
                    ),
            );
        } else {
            hero = hero.child(
                div()
                    .absolute()
                    .left(px(32.))
                    .bottom(px(70.))
                    .text_3xl()
                    .child(if self.loading {
                        "Loading the gallery…"
                    } else {
                        "Choose an artwork"
                    }),
            );
        }
        hero.into_any_element()
    }
    fn tile(&self, art: Artwork, cx: &Context<Self>) -> AnyElement {
        let selected = self.selected.as_ref() == Some(&art.id);
        let picture = match self.previews.get(&art.id) {
            Some(Ok(path)) => fade(
                img(path.clone())
                    .size_full()
                    .rounded_lg()
                    .object_fit(ObjectFit::Cover),
                SharedString::from(format!("image-arrival-{}", art.id)),
                250,
                self.motion_enabled,
            ),
            Some(Err(_)) => div()
                .p_3()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Preview unavailable · click to retry")
                .into_any_element(),
            None => div()
                .p_3()
                .text_xs()
                .text_color(rgb(MUTED))
                .child("Loading…")
                .into_any_element(),
        };
        let click = art.clone();
        let hover_key = format!("tile-{}", art.id);
        let hover = if self.motion_enabled {
            self.motion(&hover_key)
        } else {
            0.
        };
        div()
            .id(SharedString::from(art.id.clone()))
            .w(px(260.))
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .relative()
            .top(px(-2. * hover))
            .opacity(1. - 0.04 * hover)
            .child(
                div()
                    .w_full()
                    .h(px(146.25))
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(rgb(SURFACE))
                    .border_1()
                    .border_color(if selected {
                        rgb(TEXT)
                    } else {
                        rgba(0xffffff00)
                    })
                    .child(picture),
            )
            .child(
                div()
                    .px_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(TEXT))
                            .truncate()
                            .child(art.alt.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .truncate()
                            .child(art.artist().to_owned()),
                    ),
            )
            .on_hover(cx.listener(move |app, hover, _, cx| {
                app.set_motion(hover_key.clone(), if *hover { 1. } else { 0. }, 150, cx)
            }))
            .on_click(cx.listener(move |app, _, _, cx| {
                if app.applying {
                    return;
                }
                if app.previews.get(&click.id).is_some_and(|p| p.is_err()) {
                    app.previews.remove(&click.id);
                    app.preview(click.clone(), cx);
                }
                app.select(click.clone(), cx);
            }))
            .into_any_element()
    }
    fn shelf(
        &self,
        id: &'static str,
        title: &'static str,
        viewport_width: f32,
        artworks: &[Artwork],
        cx: &Context<Self>,
    ) -> AnyElement {
        let handle = if id == "recent-shelf" {
            &self.recent_scroll
        } else {
            &self.explore_scroll
        };
        let previous = handle.clone();
        let next = handle.clone();
        div()
            .w(px(viewport_width))
            .min_w_0()
            .flex_none()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .px_8()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-previous")))
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(SURFACE))
                                    .text_xs()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x303431)))
                                    .child("Previous")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if !app.applying {
                                            app.move_shelf(id, &previous, -828., cx);
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("{id}-next")))
                                    .px_3()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(SURFACE))
                                    .text_xs()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x303431)))
                                    .child("Next")
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        if !app.applying {
                                            app.move_shelf(id, &next, 828., cx);
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id(id)
                    .w(px(viewport_width))
                    .min_w_0()
                    .h(px(210.))
                    .overflow_x_scroll()
                    .track_scroll(handle)
                    .child(
                        div()
                            .w(px(artworks.len() as f32 * 276. + 48.))
                            .flex_none()
                            .flex()
                            .gap_4()
                            .px_8()
                            .pb_3()
                            .children(artworks.iter().cloned().map(|art| self.tile(art, cx))),
                    ),
            )
            .into_any_element()
    }
    fn move_shelf(
        &mut self,
        id: &'static str,
        handle: &ScrollHandle,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        let max = f32::from(handle.max_offset().width);
        let current = f32::from(handle.offset().x);
        let old_target = self.shelf_motion.get(id).map_or(current, |(_, t)| t.to);
        let target = (old_target - delta).clamp(-max, 0.);
        if self.motion_enabled {
            self.shelf_motion.insert(
                id,
                (
                    handle.clone(),
                    Tween {
                        from: current,
                        to: target,
                        start: Instant::now(),
                        duration: Duration::from_millis(240),
                    },
                ),
            );
        } else {
            handle.set_offset(point(px(target), px(0.)));
        }
        cx.notify();
    }
    fn footer(&self, cx: &Context<Self>) -> AnyElement {
        div()
            .px_8()
            .pb_8()
            .pt_2()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .id("load-more")
                    .w(px(208.))
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(blend(SURFACE, 0x2b302c, self.motion("load-more")))
                    .text_sm()
                    .cursor_pointer()
                    .on_hover(cx.listener(|app, hover, _, cx| {
                        app.set_motion("load-more", if *hover { 1. } else { 0. }, 180, cx)
                    }))
                    .opacity(if self.applying || self.loading {
                        0.4
                    } else {
                        1.
                    })
                    .child(if self.loading {
                        "Loading…"
                    } else if self.end {
                        "You’ve reached the end"
                    } else {
                        "Load more artwork"
                    })
                    .on_click(cx.listener(|app, _, _, cx| app.load(false, cx))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(format!("{} artworks loaded", self.artworks.len()))
                    .child(
                        div()
                            .id("gallery-source")
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(TEXT)))
                            .child("Artwork from reframed.gallery")
                            .on_click(cx.listener(|app, _, _, cx| {
                                if !app.applying {
                                    cx.open_url("https://www.reframed.gallery/");
                                }
                            })),
                    ),
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
                "Browse the artwork shelves with Previous and Next. Search by title or artist to find a piece among the artworks you’ve loaded.",
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
                    .child(description),
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
        self.set_motion(
            "search-focus",
            if self.focus.is_focused(window) {
                1.
            } else {
                0.
            },
            180,
            cx,
        );
        let mut animating = self.interactions.values().any(Tween::active)
            || self.selection_start.elapsed() < Duration::from_millis(350);
        self.shelf_motion.retain(|_, (handle, tween)| {
            handle.set_offset(point(px(tween.value()), px(0.)));
            let active = tween.active();
            animating |= active;
            active
        });
        if self.motion_enabled && animating {
            window.request_animation_frame();
        }
        let status = div().px_8().text_xs().text_color(rgb(0x9ba39c)).child(
            self.intro_save_error
                .clone()
                .unwrap_or_else(|| self.status.clone()),
        );
        let status = if self.motion_enabled && (self.loading || self.applying) {
            status
                .with_animation(
                    "loading-pulse",
                    Animation::new(Duration::from_millis(1300))
                        .repeat()
                        .with_easing(|t| 0.65 + 0.35 * (std::f32::consts::PI * t).sin()),
                    |e, t| e.opacity(t),
                )
                .into_any_element()
        } else {
            fade(
                status,
                SharedString::from(format!("status-{}-{}", self.status_epoch, self.status)),
                200,
                self.motion_enabled,
            )
        };
        let query = self.query.to_lowercase();
        let filtered: Vec<_> = self
            .artworks
            .iter()
            .filter(|a| {
                format!("{} {}", a.title(), a.artist())
                    .to_lowercase()
                    .contains(&query)
            })
            .cloned()
            .collect();
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| self.artworks.iter().find(|a| &a.id == id))
            .cloned();
        let split = filtered.len().min(12);
        let hero_height = (f32::from(window.viewport_size().width) * 0.375).clamp(405., 520.);
        let page = div()
            .id("page-scroll")
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(CANVAS))
            .text_color(rgb(TEXT))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(self.hero(selected, hero_height, window, cx))
                    .child(status)
                    .when(filtered.is_empty(), |d| {
                        d.child(div().px_8().py_6().text_sm().text_color(rgb(MUTED)).child(
                            if self.loading {
                                "Loading artworks…"
                            } else {
                                "No artwork found. Clear your search or Refresh."
                            },
                        ))
                    })
                    .when(!filtered.is_empty(), |d| {
                        d.child(self.shelf(
                            "recent-shelf",
                            "Recent artworks",
                            f32::from(window.viewport_size().width),
                            &filtered[..split],
                            cx,
                        ))
                    })
                    .when(filtered.len() > split, |d| {
                        d.child(self.shelf(
                            "explore-shelf",
                            "More to explore",
                            f32::from(window.viewport_size().width),
                            &filtered[split..],
                            cx,
                        ))
                    })
                    .child(self.footer(cx)),
            );
        if self.intro_visible && !self.intro_focus.is_focused(window) {
            self.intro_focus.focus(window);
        }
        div()
            .relative()
            .size_full()
            .bg(rgb(CANVAS))
            .text_color(rgb(TEXT))
            .child(page)
            .when(self.intro_visible, |d| d.child(self.intro(window, cx)))
    }
}
