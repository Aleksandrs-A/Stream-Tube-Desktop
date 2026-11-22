//! Application shell: routing, top bar, sidebar, overlays, keyboard shortcuts.
use super::pages::*;
use super::*;
use crate::api::innertube;
use crate::api::types::VideoItem;
use crate::player::controller::{Player, player};
use crate::player::view::PlayerView;
use crate::store;
use gpui_kit::component::input::{Input, InputEvent, InputState};

// ───────────────────────── routes ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Explore(usize),
    Shorts(Option<String>),
    Subscriptions,
    Channels,
    History,
    Library,
    LocalPlaylist(i64),
    Playlist(String),
    Search(String),
    Channel(String),
    Watch { id: String, playlist: Option<String>, start: Option<f64> },
    Settings,
    Downloads,
}

impl Route {
    fn title(&self) -> String {
        match self {
            Route::Home => "Home".into(),
            Route::Explore(i) => innertube::EXPLORE.get(*i).map(|e| e.0.to_string()).unwrap_or_default(),
            Route::Shorts(_) => "Shorts".into(),
            Route::Subscriptions => "Subscriptions".into(),
            Route::Channels => "Channels".into(),
            Route::History => "History".into(),
            Route::Library => "Playlists".into(),
            Route::LocalPlaylist(_) | Route::Playlist(_) => "Playlist".into(),
            Route::Search(q) => q.clone(),
            Route::Channel(_) => "Channel".into(),
            Route::Watch { .. } => "Watch".into(),
            Route::Settings => "Settings".into(),
            Route::Downloads => "Downloads".into(),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct UiState {
    pub theater: bool,
    pub fullscreen: bool,
    pub sidebar_open: bool,
    pub content_width: Pixels,
    pub viewport: Size<Pixels>,
}
impl Global for UiState {}

pub struct AppHandle(pub WeakEntity<AppView>);
impl Global for AppHandle {}

pub fn navigate(route: Route, window: &mut Window, cx: &mut App) {
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| a.go(route, window, cx));
    }
}

pub fn go_back(window: &mut Window, cx: &mut App) {
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| a.back(window, cx));
    }
}

/// Leaves the watch page while keeping playback (mini player).
pub fn go_back_keep_playing(window: &mut Window, cx: &mut App) {
    if cx.global::<UiState>().fullscreen {
        toggle_fullscreen(window, cx);
    }
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| {
            if a.history.is_empty() {
                a.go(Route::Home, window, cx);
            } else {
                a.back(window, cx);
            }
        });
    }
}

pub fn toggle_fullscreen(window: &mut Window, cx: &mut App) {
    window.toggle_fullscreen();
    let ui = cx.global_mut::<UiState>();
    ui.fullscreen = !ui.fullscreen;
    cx.refresh_windows();
}

/// Called by the player when the queue advances so the watch page follows.
pub fn sync_watch_route(id: String, cx: &mut App) {
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| {
            if let Route::Watch { .. } = a.route {
                a.pending_watch_sync = Some(id);
                cx.notify();
            }
        });
    }
}

pub fn has_autoplay_next(cx: &App) -> bool {
    cx.try_global::<AutoplayNext>().map(|a| a.0.is_some()).unwrap_or(false)
}

/// The video the watch page proposes to autoplay next (first related).
#[derive(Default)]
pub struct AutoplayNext(pub Option<VideoItem>);
impl Global for AutoplayNext {}

pub fn play_next(window: &mut Window, cx: &mut App) {
    let p = player(cx);
    let (pos, len) = {
        let pl = p.read(cx);
        (pl.queue_pos, pl.queue.len())
    };
    if let Some(i) = pos.filter(|i| i + 1 < len) {
        p.update(cx, |p, cx| p.play_queue_index(i + 1, cx));
        return;
    }
    if let Some(v) = cx.default_global::<AutoplayNext>().0.clone() {
        navigate(Route::Watch { id: v.id, playlist: None, start: None }, window, cx);
    }
}

// ───────────────────────── menus & dialogs ─────────────────────────

#[derive(Clone)]
pub struct MenuItem {
    pub icon: &'static str,
    pub label: SharedString,
    pub action: Arc<dyn Fn(&mut Window, &mut App)>,
}

impl MenuItem {
    pub fn new(icon: &'static str, label: impl Into<SharedString>, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { icon, label: label.into(), action: Arc::new(f) }
    }
}

#[derive(Default)]
pub struct OpenMenu(pub Option<(Point<Pixels>, Vec<MenuItem>)>);
impl Global for OpenMenu {}

pub fn open_menu(pos: Point<Pixels>, items: Vec<MenuItem>, _window: &mut Window, cx: &mut App) {
    cx.set_global(OpenMenu(Some((pos, items))));
    cx.refresh_windows();
}

pub enum Dialog {
    SignIn,
    SaveToPlaylist(VideoItem),
    TextPrompt { title: String, placeholder: String, on_submit: Arc<dyn Fn(String, &mut Window, &mut App)> },
    Confirm { title: String, body: String, confirm: String, on_confirm: Arc<dyn Fn(&mut Window, &mut App)> },
}

/// State of the TV device-code sign-in shown in the sign-in dialog.
#[derive(Default)]
pub struct LoginFlow {
    code: Option<crate::api::auth::DeviceCode>,
    error: Option<String>,
    attempt: u64,
}

pub fn open_sign_in(_window: &mut Window, cx: &mut App) {
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| a.start_sign_in(cx));
    }
}

pub fn sign_out(cx: &mut App) {
    crate::api::auth::sign_out();
    toast(cx, "Signed out");
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| {
            a.reload_page = true;
            cx.notify();
        });
    }
}

pub fn open_save_dialog(v: VideoItem, window: &mut Window, cx: &mut App) {
    open_dialog(Dialog::SaveToPlaylist(v), window, cx);
}

pub fn open_dialog(d: Dialog, window: &mut Window, cx: &mut App) {
    if let Some(app) = cx.try_global::<AppHandle>().and_then(|h| h.0.upgrade()) {
        app.update(cx, |a, cx| {
            a.dialog_input.update(cx, |i, cx| i.set_value("", window, cx));
            if let Dialog::TextPrompt { .. } = &d {
                let fh = a.dialog_input.read(cx).focus_handle(cx);
                window.focus(&fh, cx);
            }
            a.dialog = Some(d);
            cx.notify();
        });
    }
}

pub fn prompt_text(title: &str, placeholder: &str, window: &mut Window, cx: &mut App, f: impl Fn(String, &mut Window, &mut App) + 'static) {
    open_dialog(
        Dialog::TextPrompt { title: title.into(), placeholder: placeholder.into(), on_submit: Arc::new(f) },
        window,
        cx,
    );
}

pub fn confirm(title: &str, body: &str, confirm: &str, window: &mut Window, cx: &mut App, f: impl Fn(&mut Window, &mut App) + 'static) {
    open_dialog(
        Dialog::Confirm { title: title.into(), body: body.into(), confirm: confirm.into(), on_confirm: Arc::new(f) },
        window,
        cx,
    );
}

// ───────────────────────── AppView ─────────────────────────

pub struct AppView {
    pub route: Route,
    page: AnyView,
    pub history: Vec<(Route, AnyView)>,
    forward: Vec<(Route, AnyView)>,
    search: Entity<InputState>,
    suggestions: Vec<String>,
    history_suggestions: Vec<String>,
    show_suggestions: bool,
    suggestion_serial: u64,
    sel_suggestion: Option<usize>,
    mini_view: Entity<PlayerView>,
    main_view: Entity<PlayerView>,
    had_video: bool,
    last_bounds_save: std::time::Instant,
    dialog: Option<Dialog>,
    dialog_input: Entity<InputState>,
    focus: FocusHandle,
    pending_watch_sync: Option<String>,
    login: LoginFlow,
    reload_page: bool,
    _subs: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.set_global(AppHandle(cx.entity().downgrade()));
        cx.set_global(UiState { sidebar_open: true, ..Default::default() });
        cx.set_global(OpenMenu::default());
        cx.set_global(AutoplayNext::default());
        cx.set_global(crate::ui::cards::Watched(store::progress_map()));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search or paste a YouTube link"));
        let dialog_input = cx.new(|cx| InputState::new(window, cx));
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&search, window, |this: &mut Self, state, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } => {
                let q = if let Some(i) = this.sel_suggestion {
                    this.all_suggestions().get(i).cloned().unwrap_or_default()
                } else {
                    state.read(cx).value().to_string()
                };
                this.submit_search(q, window, cx);
            }
            InputEvent::Change => {
                this.sel_suggestion = None;
                let q = state.read(cx).value().to_string();
                this.update_suggestions(q, cx);
            }
            InputEvent::Focus => {
                this.show_suggestions = true;
                let q = state.read(cx).value().to_string();
                this.update_suggestions(q, cx);
            }
            InputEvent::Blur => {
                // delay so clicks on suggestions register
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
                    let _ = this.update(cx, |a, cx| {
                        a.show_suggestions = false;
                        cx.notify();
                    });
                })
                .detach();
            }
        }));
        subs.push(cx.subscribe_in(&dialog_input, window, |this: &mut Self, state, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let v = state.read(cx).value().to_string();
                if let Some(Dialog::TextPrompt { on_submit, .. }) = this.dialog.take() {
                    if !v.trim().is_empty() {
                        on_submit(v.trim().to_string(), window, cx);
                    }
                }
                cx.notify();
            }
        }));
        let p = player(cx);
        // Only re-render the shell when the mini player appears/disappears; the
        // player views re-render themselves for every frame.
        subs.push(cx.observe(&p, |this: &mut Self, p, cx| {
            let has = p.read(cx).video.is_some();
            if has != this.had_video {
                this.had_video = has;
                cx.notify();
            }
        }));
        subs.push(cx.observe_window_bounds(window, |this: &mut Self, window, _| {
            // persist placement at most twice a second while the user drags/resizes
            if this.last_bounds_save.elapsed() > std::time::Duration::from_millis(500) {
                this.last_bounds_save = std::time::Instant::now();
                crate::save_window_bounds(window);
            }
        }));
        subs.push(cx.observe_window_appearance(window, |_, window, cx| {
            if store::settings_snapshot().theme == "system" {
                apply_theme(Some(window), cx);
            }
        }));
        window.on_window_should_close(cx, |window, _| {
            crate::save_window_bounds(window);
            true
        });
        let mini_view = cx.new(|cx| PlayerView::new(p.clone(), true, cx));
        let main_view = cx.new(|cx| PlayerView::new(p.clone(), false, cx));
        cx.set_global(crate::ui::slot::PlayerSlot::default());
        // a YouTube link passed on the command line opens directly
        let arg_route = std::env::args().nth(1).and_then(|a| innertube::parse_youtube_url(&a)).map(|r| match r {
            innertube::ResolvedUrl::Video(id) => Route::Watch { id, playlist: None, start: None },
            innertube::ResolvedUrl::Channel(id) => Route::Channel(id),
            innertube::ResolvedUrl::Playlist(id) => Route::Playlist(id),
        });
        let start = arg_route.unwrap_or_else(|| match store::settings_snapshot().start_page.as_str() {
            "subscriptions" => Route::Subscriptions,
            "history" => Route::History,
            _ => Route::Home,
        });
        let page = build_page(&start, window, cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            route: start,
            page,
            history: Vec::new(),
            forward: Vec::new(),
            search,
            suggestions: Vec::new(),
            history_suggestions: Vec::new(),
            show_suggestions: false,
            suggestion_serial: 0,
            sel_suggestion: None,
            mini_view,
            main_view,
            had_video: false,
            last_bounds_save: std::time::Instant::now(),
            dialog: None,
            dialog_input,
            focus,
            pending_watch_sync: None,
            login: LoginFlow::default(),
            reload_page: false,
            _subs: subs,
        }
    }

    fn start_sign_in(&mut self, cx: &mut Context<Self>) {
        use crate::api::auth::{self, PollResult};
        use std::time::{Duration, Instant};
        self.login.attempt += 1;
        let attempt = self.login.attempt;
        self.login.code = None;
        self.login.error = None;
        self.dialog = Some(Dialog::SignIn);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let fail = |this: &WeakEntity<Self>, cx: &mut AsyncApp, msg: String| {
                let _ = this.update(cx, |t, cx| {
                    if t.login.attempt == attempt {
                        t.login.error = Some(msg);
                        cx.notify();
                    }
                });
            };
            let dc = match blocking(auth::start_login).await {
                Ok(dc) => dc,
                Err(e) => return fail(&this, cx, crate::ui::friendly_error(&e.to_string())),
            };
            let deadline = Instant::now() + Duration::from_secs(dc.expires_in);
            let mut interval = dc.interval.max(3);
            let _ = this.update(cx, |t, cx| {
                if t.login.attempt == attempt {
                    t.login.code = Some(dc.clone());
                    cx.notify();
                }
            });
            let mut errors = 0;
            loop {
                cx.background_executor().timer(Duration::from_secs(interval)).await;
                let waiting = this
                    .update(cx, |t, _| t.login.attempt == attempt && matches!(t.dialog, Some(Dialog::SignIn)))
                    .unwrap_or(false);
                if !waiting {
                    return;
                }
                if Instant::now() > deadline {
                    return fail(&this, cx, "The code expired. Try again.".into());
                }
                let d = dc.clone();
                match blocking(move || auth::poll_login(&d)).await {
                    Ok(PollResult::Pending) => errors = 0,
                    Ok(PollResult::SlowDown) => interval += 2,
                    Ok(PollResult::Done(acc)) => {
                        let _ = this.update(cx, |t, cx| {
                            t.dialog = None;
                            t.login.code = None;
                            t.reload_page = true;
                            toast(cx, format!("Signed in as {}", acc.name));
                            cx.notify();
                        });
                        // bring the account's subscriptions into the local list
                        let res = blocking(auth::sync_subscriptions).await;
                        let _ = this.update(cx, |t, cx| {
                            if let Ok(n) = res {
                                if n > 0 {
                                    toast(cx, format!("Added {n} subscriptions from your account"));
                                    t.reload_page = true;
                                    cx.notify();
                                }
                            }
                        });
                        return;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        // network hiccups don't end the sign-in; a refusal from Google does
                        errors += 1;
                        let refused = msg.contains("expired") || msg.contains("cancelled") || msg.contains("failed");
                        if !refused && errors < 4 {
                            continue;
                        }
                        return fail(&this, cx, crate::ui::friendly_error(&msg));
                    }
                }
            }
        })
        .detach();
    }

    fn render_sign_in(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        let mut col = div().flex().flex_col().gap_4().child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(icon("circle-user").size_6().text_color(p.text))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child("Sign in to YouTube")),
        );
        if let Some(err) = &self.login.error {
            col = col.child(div().text_sm().text_color(p.text2).child(err.clone())).child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(pill("si-cancel", None, "Cancel", cx).on_click(cx.listener(|this, _, _, cx| {
                        this.dialog = None;
                        cx.notify();
                    })))
                    .child(pill("si-retry", Some("refresh-cw"), "Try again", cx).on_click(cx.listener(|this, _, _, cx| this.start_sign_in(cx)))),
            );
        } else if let Some(dc) = &self.login.code {
            let code = dc.user_code.clone();
            let url = dc.verification_url.clone();
            let shown_url = url.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.").to_string();
            let code2 = code.clone();
            let url2 = url.clone();
            col = col
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_sm()
                        .text_color(p.text2)
                        .child(
                            div().flex().gap_1().child("1. On any device, go to").child(
                                div()
                                    .id("si-url")
                                    .text_color(p.link)
                                    .cursor_pointer()
                                    .hover(|s| s.underline())
                                    .child(shown_url)
                                    .on_click(move |_, _, _| {
                                        let _ = open::that(&url2);
                                    }),
                            ),
                        )
                        .child("2. Enter this code and pick your account"),
                )
                .child(
                    div()
                        .id("si-code")
                        .flex()
                        .items_center()
                        .justify_center()
                        .py_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(p.border)
                        .bg(p.bg)
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.hover))
                        .text_2xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.text)
                        .child(code.clone())
                        .on_click(move |_, _, cx| copy_to_clipboard(code2.clone(), cx)),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_xs().text_color(p.text3).child("Waiting for approval…"))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(pill("si-cancel", None, "Cancel", cx).on_click(cx.listener(|this, _, _, cx| {
                                    this.dialog = None;
                                    cx.notify();
                                })))
                                .child(pill("si-open", Some("external-link"), "Copy code & open", cx).bg(p.link).text_color(white()).on_click(
                                    move |_, _, cx| {
                                        copy_to_clipboard(code.clone(), cx);
                                        let _ = open::that(&url);
                                    },
                                )),
                        ),
                );
        } else {
            col = col.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .py_4()
                    .child(spinner(cx))
                    .child(div().text_sm().text_color(p.text2).child("Getting a code…")),
            );
        }
        col.into_any_element()
    }

    fn account_button(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        if !crate::api::auth::signed_in() {
            return div()
                .id("sign-in")
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .h_9()
                .px_3()
                .ml_2()
                .rounded_full()
                .border_1()
                .border_color(p.border)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.link)
                .cursor_pointer()
                .hover(move |s| s.bg(p.hover))
                .child(icon("circle-user").size_5().text_color(p.link))
                .child("Sign in")
                .on_click(cx.listener(|this, _, _, cx| this.start_sign_in(cx)))
                .into_any_element();
        }
        let acc = crate::api::auth::account().unwrap_or_default();
        div()
            .id("account")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size_10()
            .ml_1()
            .rounded_full()
            .cursor_pointer()
            .hover(move |s| s.bg(p.hover))
            .child(crate::ui::avatar(acc.avatar.as_deref(), px(30.), &acc.name, cx))
            .on_click(move |ev, window, cx| {
                let label = match &acc.handle {
                    Some(h) => format!("{}  ·  {}", acc.name, h),
                    None => acc.name.clone(),
                };
                let items = vec![
                    MenuItem::new("circle-user", label, |_, _| {}),
                    MenuItem::new("refresh-cw", "Sync subscriptions", |_, cx| {
                        cx.spawn(async move |cx| {
                            let res = blocking(crate::api::auth::sync_subscriptions).await;
                            let _ = cx.update(|cx| {
                                match res {
                                    Ok(0) => toast(cx, "Subscriptions are up to date"),
                                    Ok(n) => toast(cx, format!("Added {n} subscriptions")),
                                    Err(e) => toast(cx, crate::ui::friendly_error(&e.to_string())),
                                }
                                cx.refresh_windows();
                            });
                        })
                        .detach();
                    }),
                    MenuItem::new("log-out", "Sign out", |_, cx| sign_out(cx)),
                ];
                open_menu(ev.position(), items, window, cx);
            })
            .into_any_element()
    }

    pub fn go(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        // items without a known channel/video/playlist have nothing to open
        match &route {
            Route::Channel(id) | Route::Playlist(id) | Route::Watch { id, .. } if id.is_empty() => return,
            _ => {}
        }
        cx.set_global(OpenMenu(None));
        if route == self.route {
            // re-navigating to the same place refreshes it (like clicking Home again)
            if !matches!(route, Route::Watch { .. }) {
                self.page = build_page(&route, window, cx);
                cx.notify();
            }
            return;
        }
        if cx.global::<UiState>().fullscreen && !matches!(route, Route::Watch { .. }) {
            toggle_fullscreen(window, cx);
        }
        let page = build_page(&route, window, cx);
        let old_route = std::mem::replace(&mut self.route, route);
        let old_page = std::mem::replace(&mut self.page, page);
        // Don't keep stale watch pages in history for the same video
        self.history.push((old_route, old_page));
        if self.history.len() > 40 {
            self.history.remove(0);
        }
        self.forward.clear();
        window.set_window_title(&format!("{} - Stream Tube", self.route.title()));
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((r, p)) = self.history.pop() {
            let cur_r = std::mem::replace(&mut self.route, r);
            let cur_p = std::mem::replace(&mut self.page, p);
            self.forward.push((cur_r, cur_p));
            // If we return to a watch page for a different video than is playing, rebuild it
            if let Route::Watch { id, .. } = &self.route {
                let playing = player(cx).read(cx).video.as_ref().map(|v| v.id.clone());
                if playing.as_deref() != Some(id.as_str()) {
                    self.page = build_page(&self.route.clone(), window, cx);
                }
            }
            window.set_window_title(&format!("{} - Stream Tube", self.route.title()));
            cx.notify();
        }
    }

    pub fn forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((r, p)) = self.forward.pop() {
            let cur_r = std::mem::replace(&mut self.route, r);
            let cur_p = std::mem::replace(&mut self.page, p);
            self.history.push((cur_r, cur_p));
            window.set_window_title(&format!("{} - Stream Tube", self.route.title()));
            cx.notify();
        }
    }

    fn all_suggestions(&self) -> Vec<String> {
        let mut v = self.history_suggestions.clone();
        for s in &self.suggestions {
            if !v.contains(s) {
                v.push(s.clone());
            }
        }
        v.truncate(14);
        v
    }

    fn update_suggestions(&mut self, q: String, cx: &mut Context<Self>) {
        self.show_suggestions = true;
        self.history_suggestions = store::search_history(&q, if q.is_empty() { 10 } else { 4 });
        self.suggestion_serial += 1;
        let serial = self.suggestion_serial;
        if q.trim().is_empty() || q.contains("youtu") {
            self.suggestions.clear();
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
            let still = this.update(cx, |a, _| a.suggestion_serial == serial).unwrap_or(false);
            if !still {
                return;
            }
            let res = blocking(move || innertube::suggestions(&q).unwrap_or_default()).await;
            let _ = this.update(cx, |a, cx| {
                if a.suggestion_serial == serial {
                    a.suggestions = res;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn submit_search(&mut self, q: String, window: &mut Window, cx: &mut Context<Self>) {
        let q = q.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.show_suggestions = false;
        self.sel_suggestion = None;
        self.search.update(cx, |s, cx| s.set_value(q.clone(), window, cx));
        window.focus(&self.focus, cx);
        // Direct links
        if let Some(r) = innertube::parse_youtube_url(&q) {
            let route = match r {
                innertube::ResolvedUrl::Video(id) => {
                    let t = q.split(['?', '&']).find_map(|kv| kv.strip_prefix("t=")).and_then(|t| t.trim_end_matches('s').parse::<f64>().ok());
                    let list = q.split(['?', '&']).find_map(|kv| kv.strip_prefix("list=")).map(str::to_string);
                    Route::Watch { id, playlist: list, start: t }
                }
                innertube::ResolvedUrl::Channel(id) => Route::Channel(id),
                innertube::ResolvedUrl::Playlist(id) => Route::Playlist(id),
            };
            self.go(route, window, cx);
            return;
        }
        if q.contains("youtube.com/") || q.starts_with('@') {
            let url = if q.starts_with('@') { format!("https://www.youtube.com/{q}") } else { q.clone() };
            cx.spawn_in(window, async move |this, cx| {
                let r = blocking(move || innertube::resolve_url(&url)).await;
                let _ = this.update_in(cx, |a, window, cx| match r {
                    Ok(innertube::ResolvedUrl::Channel(id)) => a.go(Route::Channel(id), window, cx),
                    Ok(innertube::ResolvedUrl::Playlist(id)) => a.go(Route::Playlist(id), window, cx),
                    Ok(innertube::ResolvedUrl::Video(id)) => a.go(Route::Watch { id, playlist: None, start: None }, window, cx),
                    Err(e) => toast(cx, format!("Could not open link: {}", crate::ui::friendly_error(&e.to_string()))),
                });
            })
            .detach();
            return;
        }
        store::search_history_add(&q);
        self.go(Route::Search(q), window, cx);
    }

    // ───── keyboard ─────

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let dialog_focused = self.dialog_input.read(cx).focus_handle(cx).is_focused(window);
        let k = ev.keystroke.key.as_str();
        let m = &ev.keystroke.modifiers;
        if k == "escape" {
            if self.dialog.is_some() {
                self.dialog = None;
                cx.notify();
                return;
            }
            if cx.global::<OpenMenu>().0.is_some() {
                cx.set_global(OpenMenu(None));
                cx.notify();
                return;
            }
            if search_focused {
                self.show_suggestions = false;
                window.focus(&self.focus, cx);
                cx.notify();
                return;
            }
            if cx.global::<UiState>().fullscreen {
                toggle_fullscreen(window, cx);
                return;
            }
        }
        if search_focused || dialog_focused {
            if search_focused && (k == "down" || k == "up") {
                let n = self.all_suggestions().len();
                if n > 0 {
                    self.sel_suggestion = Some(match (self.sel_suggestion, k) {
                        (None, "down") => 0,
                        (None, _) => n - 1,
                        (Some(i), "down") => (i + 1) % n,
                        (Some(i), _) => (i + n - 1) % n,
                    });
                    cx.notify();
                }
            }
            return;
        }
        if m.alt && k == "left" {
            self.back(window, cx);
            return;
        }
        if m.alt && k == "right" {
            self.forward(window, cx);
            return;
        }
        if m.control && (k == "l" || k == "k") || k == "/" {
            let fh = self.search.read(cx).focus_handle(cx);
            window.focus(&fh, cx);
            return;
        }
        if m.control || m.platform || m.alt {
            return;
        }
        if matches!(self.route, Route::Shorts(_)) {
            if let Some(page) = self.page.clone().downcast::<shorts::ShortsPage>().ok() {
                if k == "down" || k == "up" {
                    page.update(cx, |s, cx| s.step(if k == "down" { 1 } else { -1 }, window, cx));
                    return;
                }
            }
        }
        let p = player(cx);
        if p.read(cx).video.is_none() {
            return;
        }
        let step = store::settings_snapshot().seek_step;
        let handled = p.update(cx, |pl: &mut Player, cx| {
            let handled = match (k, m.shift) {
                ("space", _) | ("k", false) => {
                    pl.toggle_pause();
                    true
                }
                ("j", false) => {
                    pl.seek_by(-10.0);
                    true
                }
                ("l", false) => {
                    pl.seek_by(10.0);
                    true
                }
                ("left", _) => {
                    pl.seek_by(-step);
                    true
                }
                ("right", _) => {
                    pl.seek_by(step);
                    true
                }
                ("up", _) => {
                    let v = pl.volume;
                    pl.set_volume(v + 5.0);
                    true
                }
                ("down", _) => {
                    let v = pl.volume;
                    pl.set_volume(v - 5.0);
                    true
                }
                ("m", false) => {
                    pl.toggle_mute();
                    true
                }
                ("c", false) => {
                    let next = if pl.caption.is_some() {
                        None
                    } else {
                        let lang = store::settings_snapshot().captions_lang.clone();
                        pl.streams.as_ref().and_then(|s| {
                            s.captions.iter().position(|c| c.lang.starts_with(&lang)).or(if s.captions.is_empty() { None } else { Some(0) })
                        })
                    };
                    pl.set_caption(next, cx);
                    true
                }
                (",", false) if pl.paused => {
                    pl.frame_step(false);
                    true
                }
                (".", false) if pl.paused => {
                    pl.frame_step(true);
                    true
                }
                (",", true) | ("<", _) => {
                    let s = pl.speed;
                    pl.set_speed(s - 0.25);
                    crate::ui::toast(cx, format!("Speed {:.2}x", (s - 0.25).max(0.25)));
                    true
                }
                (".", true) | (">", _) => {
                    let s = pl.speed;
                    pl.set_speed(s + 0.25);
                    crate::ui::toast(cx, format!("Speed {:.2}x", s + 0.25));
                    true
                }
                ("home", _) => {
                    pl.seek_to(0.0);
                    true
                }
                ("end", _) => {
                    let d = pl.duration;
                    pl.seek_to(d - 1.0);
                    true
                }
                (d, false) if d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()) => {
                    let n: f64 = d.parse().unwrap();
                    let dur = pl.duration;
                    pl.seek_to(dur * n / 10.0);
                    true
                }
                ("p", true) => {
                    if let Some(i) = pl.queue_pos.filter(|i| *i > 0) {
                        pl.play_queue_index(i - 1, cx);
                    }
                    true
                }
                ("a", false) => {
                    let a = !pl.audio_only;
                    pl.set_audio_only(a, cx);
                    true
                }
                _ => false,
            };
            cx.notify();
            handled
        });
        if handled {
            return;
        }
        match (k, m.shift) {
            ("f", false) => toggle_fullscreen(window, cx),
            ("t", false) => {
                cx.global_mut::<UiState>().theater ^= true;
                cx.notify();
            }
            ("i", false) => {
                if matches!(self.route, Route::Watch { .. }) {
                    if self.history.is_empty() {
                        self.go(Route::Home, window, cx);
                    } else {
                        self.back(window, cx);
                    }
                } else if let Some(v) = p.read(cx).video.clone() {
                    self.go(Route::Watch { id: v.id, playlist: None, start: None }, window, cx);
                }
            }
            ("n", true) => play_next(window, cx),
            _ => {}
        }
    }

    // ───── chrome ─────

    fn render_topbar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let can_back = !self.history.is_empty();
        let suggestions = if self.show_suggestions { self.all_suggestions() } else { Vec::new() };
        let hist: std::collections::HashSet<String> = self.history_suggestions.iter().cloned().collect();
        let sel = self.sel_suggestion;
        let maximized = window.is_maximized();
        div()
            .id("topbar")
            .flex()
            .flex_none()
            .items_center()
            .h(px(56.))
            .pl_4()
            .bg(p.bg)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(icon_button("menu", "menu", cx).on_click(|_, _, cx| {
                        cx.global_mut::<UiState>().sidebar_open ^= true;
                        cx.refresh_windows();
                    }))
                    .child(
                        icon_button("back", "arrow-left", cx)
                            .when(!can_back, |d| d.opacity(0.35))
                            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx))),
                    )
                    .child(
                        div()
                            .id("logo")
                            .flex()
                            .items_center()
                            .gap_1()
                            .ml_2()
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.go(Route::Home, window, cx)))
                            .child(crate::ui::logo(px(28.)))
                            .child(crate::ui::wordmark(cx)),
                    ),
            )
            // only the empty gaps move the window: gpui reports the first matching control
            // area, so a drag area around the buttons would swallow their clicks
            .child(drag_gap("drag-l"))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    // YouTube-width search box that gives way on narrow windows
                    .flex_shrink(1.)
                    .min_w(px(220.))
                    .w(px(640.))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .h_10()
                            .items_center()
                            .pl_4()
                            .rounded_l_full()
                            .border_1()
                            .border_color(p.border)
                            .bg(if p.dark { rgb(0x121212).into() } else { p.bg })
                            .child(Input::new(&self.search).appearance(false).cleanable(true)),
                    )
                    .child(
                        div()
                            .id("search-btn")
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(64.))
                            .h_10()
                            .rounded_r_full()
                            .border_1()
                            .border_l_0()
                            .border_color(p.border)
                            .bg(p.chip)
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.hover))
                            .child(icon("search").text_color(p.text))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let q = this.search.read(cx).value().to_string();
                                this.submit_search(q, window, cx);
                            })),
                    )
                    .when(!suggestions.is_empty(), |d| {
                        d.child(
                            deferred(
                                div()
                                    .absolute()
                                    .top(px(44.))
                                    .left_0()
                                    .right(px(64.))
                                    .py_2()
                                    .rounded_xl()
                                    .bg(p.surface2)
                                    .border_1()
                                    .border_color(p.border)
                                    .shadow_lg()
                                    .children(suggestions.into_iter().enumerate().map(|(i, s)| {
                                        let is_hist = hist.contains(&s);
                                        let s2 = s.clone();
                                        let s3 = s.clone();
                                        div()
                                            .id(SharedString::from(format!("sug-{i}")))
                                            .flex()
                                            .items_center()
                                            .gap_3()
                                            .px_4()
                                            .py_1p5()
                                            .cursor_pointer()
                                            .when(sel == Some(i), |d| d.bg(p.hover))
                                            .hover(move |st| st.bg(p.hover))
                                            .child(icon(if is_hist { "history" } else { "search" }).size_4().text_color(p.text2))
                                            .child(div().flex_1().text_color(p.text).font_weight(FontWeight::MEDIUM).child(s))
                                            .when(is_hist, |d| {
                                                d.child(
                                                    div()
                                                        .id(SharedString::from(format!("sugx-{i}")))
                                                        .text_xs()
                                                        .text_color(p.link)
                                                        .child("Remove")
                                                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            store::search_history_remove(&s3);
                                                            let q = this.search.read(cx).value().to_string();
                                                            this.update_suggestions(q, cx);
                                                        })),
                                                )
                                            })
                                            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                                                this.submit_search(s2.clone(), window, cx);
                                            }))
                                    })),
                            )
                            .with_priority(2),
                        )
                    }),
            )
            .child(drag_gap("drag-r"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(icon_button("dl", "download", cx).on_click(cx.listener(|this, _, window, cx| this.go(Route::Downloads, window, cx))))
                    .child(icon_button("settings", "settings", cx).on_click(cx.listener(|this, _, window, cx| this.go(Route::Settings, window, cx))))
                    .child(self.account_button(cx))
                    .child(div().id("drag-c").w_3().h(px(56.)).window_control_area(WindowControlArea::Drag))
                    .child(window_controls(maximized, cx)),
            )
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        let ui = *cx.global::<UiState>();
        let route = self.route.clone();
        let is_watch = matches!(route, Route::Watch { .. });
        if is_watch && !ui.sidebar_open {
            return div().into_any_element();
        }
        let entry = |id: &'static str, ic: &'static str, label: &str, r: Route, active: bool, cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_6()
                .px_3()
                .h_10()
                .rounded_lg()
                .cursor_pointer()
                .text_sm()
                .text_color(p.text)
                .when(active, |d| d.bg(p.chip).font_weight(FontWeight::MEDIUM))
                .hover(move |s| s.bg(p.hover))
                .child(icon(ic).text_color(p.text))
                .child(label.to_string())
                .on_click(cx.listener(move |this, _, window, cx| this.go(r.clone(), window, cx)))
        };
        if !ui.sidebar_open {
            // compact rail
            let rail = |id: &'static str, ic: &'static str, label: &'static str, r: Route, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .py_4()
                    .w(px(64.))
                    .rounded_lg()
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.hover))
                    .child(icon(ic).text_color(p.text))
                    .child(div().text_size(px(10.)).text_color(p.text).child(label))
                    .on_click(cx.listener(move |this, _, window, cx| this.go(r.clone(), window, cx)))
            };
            return div()
                .flex()
                .flex_col()
                .flex_none()
                .w(px(72.))
                .px_1()
                .child(rail("r-home", "house", "Home", Route::Home, cx))
                .child(rail("r-shorts", "zap", "Shorts", Route::Shorts(None), cx))
                .child(rail("r-subs", "tv", "Subscriptions", Route::Subscriptions, cx))
                .child(rail("r-you", "library", "You", Route::Library, cx))
                .into_any_element();
        }
        let s = store::settings_snapshot();
        let subs = store::subscriptions();
        let section = |title: &str| {
            div()
                .px_3()
                .pt_4()
                .pb_1()
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(title.to_string())
        };
        let sep = || div().my_3().h(px(1.)).bg(p.border);
        let mut col = div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(240.))
            .h_full()
            .px_3()
            .pb_6()
            .overflow_y_scroll()
            .child(entry("s-home", "house", "Home", Route::Home, route == Route::Home, cx))
            .when(!s.hide_shorts, |d| d.child(entry("s-shorts", "zap", "Shorts", Route::Shorts(None), matches!(route, Route::Shorts(_)), cx)))
            .child(entry("s-subs", "tv", "Subscriptions", Route::Subscriptions, route == Route::Subscriptions, cx))
            .child(sep())
            .child(section("You"))
            .child(entry("s-hist", "history", "History", Route::History, route == Route::History, cx))
            .child(entry("s-pl", "list-video", "Playlists", Route::Library, route == Route::Library, cx));
        // built-in playlists quick links
        for pl in store::playlists().into_iter().filter(|p| p.protected) {
            let ic = if pl.name == "Watch later" { "clock" } else { "heart" };
            let active = route == Route::LocalPlaylist(pl.id);
            let id = pl.id;
            col = col.child(
                div()
                    .id(SharedString::from(format!("s-lp-{id}")))
                    .flex()
                    .items_center()
                    .gap_6()
                    .px_3()
                    .h_10()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(p.text)
                    .when(active, |d| d.bg(p.chip))
                    .hover(move |s| s.bg(p.hover))
                    .child(icon(ic).text_color(p.text))
                    .child(pl.name.clone())
                    .on_click(cx.listener(move |this, _, window, cx| this.go(Route::LocalPlaylist(id), window, cx))),
            );
        }
        col = col
            .child(entry("s-dl", "download", "Downloads", Route::Downloads, route == Route::Downloads, cx))
            .child(sep())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section("Subscriptions"))
                    .child(
                        div()
                            .id("s-manage")
                            .mt_3()
                            .text_xs()
                            .text_color(p.link)
                            .cursor_pointer()
                            .child("Manage")
                            .on_click(cx.listener(|this, _, window, cx| this.go(Route::Channels, window, cx))),
                    ),
            );
        for sub in subs.iter().take(60) {
            let cid = sub.channel_id.clone();
            let active = route == Route::Channel(cid.clone());
            col = col.child(
                div()
                    .id(SharedString::from(format!("s-sub-{}", sub.channel_id)))
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_3()
                    .h_10()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(p.text)
                    .when(active, |d| d.bg(p.chip))
                    .hover(move |s| s.bg(p.hover))
                    .child(avatar(sub.avatar.as_deref(), px(24.), &sub.name, cx))
                    .child(div().flex_1().truncate().child(sub.name.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| this.go(Route::Channel(cid.clone()), window, cx))),
            );
        }
        if !s.hide_explore {
            col = col.child(sep()).child(section("Explore"));
            let icons = ["music", "gamepad-2", "newspaper", "trophy", "radio", "shirt", "lightbulb"];
            for (i, (name, _)) in innertube::EXPLORE.iter().enumerate() {
                let active = route == Route::Explore(i);
                col = col.child(
                    div()
                        .id(SharedString::from(format!("s-ex-{i}")))
                        .flex()
                        .items_center()
                        .gap_6()
                        .px_3()
                        .h_10()
                        .rounded_lg()
                        .cursor_pointer()
                        .text_sm()
                        .text_color(p.text)
                        .when(active, |d| d.bg(p.chip))
                        .hover(move |s| s.bg(p.hover))
                        .child(icon(icons.get(i).copied().unwrap_or("compass")).text_color(p.text))
                        .child(name.to_string())
                        .on_click(cx.listener(move |this, _, window, cx| this.go(Route::Explore(i), window, cx))),
                );
            }
        }
        col = col.child(sep()).child(entry("s-set", "settings", "Settings", Route::Settings, route == Route::Settings, cx));
        col.into_any_element()
    }

    fn render_mini_player(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal(cx);
        let pl = player(cx);
        let video = pl.read(cx).video.clone()?;
        if matches!(self.route, Route::Watch { .. } | Route::Shorts(_)) {
            return None;
        }
        let id = video.id.clone();
        Some(
            div()
                .absolute()
                .right_4()
                .bottom_4()
                .w(px(400.))
                .rounded_xl()
                .overflow_hidden()
                .bg(p.surface2)
                .shadow_2xl()
                .border_1()
                .border_color(p.border)
                .child(div().w_full().h(px(225.)).child(self.mini_view.clone()))
                .child(
                    div()
                        .id("mini-info")
                        .flex()
                        .flex_col()
                        .px_3()
                        .py_2()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.go(Route::Watch { id: id.clone(), playlist: None, start: None }, window, cx)
                        }))
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(p.text).truncate().child(video.title.clone()))
                        .child(div().text_xs().text_color(p.text2).truncate().child(video.channel_name.clone())),
                )
                .into_any_element(),
        )
    }

    fn render_menu(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal(cx);
        let (pos, items) = cx.global::<OpenMenu>().0.clone()?;
        Some(
            deferred(
                anchored().position(pos).snap_to_window_with_margin(px(8.)).child(
                    div()
                        .id("ctx-menu")
                        .occlude()
                        .min_w(px(240.))
                        .py_2()
                        .rounded_xl()
                        .bg(p.surface2)
                        .border_1()
                        .border_color(p.border)
                        .shadow_xl()
                        .on_mouse_down_out(|_, _, cx| {
                            cx.set_global(OpenMenu(None));
                            cx.refresh_windows();
                        })
                        .children(items.into_iter().enumerate().map(|(i, it)| {
                            let act = it.action.clone();
                            div()
                                .id(SharedString::from(format!("mi-{i}")))
                                .flex()
                                .items_center()
                                .gap_4()
                                .px_4()
                                .h_9()
                                .cursor_pointer()
                                .text_sm()
                                .text_color(p.text)
                                .hover(move |s| s.bg(p.hover))
                                .child(icon(it.icon).size_5().text_color(p.text))
                                .child(it.label.clone())
                                .on_click(move |_, window, cx| {
                                    cx.set_global(OpenMenu(None));
                                    act(window, cx);
                                    cx.refresh_windows();
                                })
                        })),
                ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    fn render_dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal(cx);
        let d = self.dialog.as_ref()?;
        let wide = matches!(d, Dialog::SignIn);
        let body: AnyElement = match d {
            Dialog::SignIn => self.render_sign_in(cx),
            Dialog::SaveToPlaylist(v) => {
                let v = v.clone();
                let pls = store::playlists();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).pb_2().child("Save video to…"))
                    .children(pls.into_iter().map(|pl| {
                        let contains = store::playlist_contains(pl.id, &v.id);
                        let v2 = v.clone();
                        div()
                            .id(SharedString::from(format!("dlg-pl-{}", pl.id)))
                            .flex()
                            .items_center()
                            .gap_3()
                            .py_2()
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.hover))
                            .child(icon(if contains { "square-check" } else { "square" }).text_color(p.text))
                            .child(div().flex_1().text_sm().text_color(p.text).child(pl.name.clone()))
                            .child(icon(if pl.protected { "lock" } else { "globe-lock" }).size_4().text_color(p.text3))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                if store::playlist_contains(pl.id, &v2.id) {
                                    store::playlist_remove(pl.id, &v2.id);
                                    toast(cx, format!("Removed from {}", pl.name));
                                } else {
                                    store::playlist_add(pl.id, &v2);
                                    toast(cx, format!("Saved to {}", pl.name));
                                }
                                cx.notify();
                            }))
                    }))
                    .child(
                        div()
                            .id("dlg-new")
                            .flex()
                            .items_center()
                            .gap_3()
                            .mt_2()
                            .py_2()
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.hover))
                            .child(icon("plus").text_color(p.text))
                            .child(div().text_sm().text_color(p.text).child("Create new playlist"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let v3 = v.clone();
                                this.dialog = None;
                                cx.notify();
                                let v4 = v3.clone();
                                cx.defer_in(window, move |_, window, cx| {
                                    prompt_text("New playlist", "Playlist name", window, cx, move |name, _, cx| {
                                        let id = store::playlist_create(&name);
                                        store::playlist_add(id, &v4);
                                        toast(cx, format!("Saved to {name}"));
                                    });
                                });
                            })),
                    )
                    .into_any_element()
            }
            Dialog::TextPrompt { title, .. } => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title.clone()))
                .child(div().border_b_1().border_color(p.text2).pb_1().child(Input::new(&self.dialog_input).appearance(false)))
                .child(
                    div().flex().justify_end().gap_2().child(pill("dlg-cancel", None, "Cancel", cx).on_click(cx.listener(|this, _, _, cx| {
                        this.dialog = None;
                        cx.notify();
                    }))).child(
                        pill("dlg-ok", None, "Create", cx).bg(p.link).text_color(white()).on_click(cx.listener(|this, _, window, cx| {
                            let v = this.dialog_input.read(cx).value().to_string();
                            if let Some(Dialog::TextPrompt { on_submit, .. }) = this.dialog.take() {
                                if !v.trim().is_empty() {
                                    on_submit(v.trim().to_string(), window, cx);
                                }
                            }
                            cx.notify();
                        })),
                    ),
                )
                .into_any_element(),
            Dialog::Confirm { title, body, confirm, .. } => div()
                .flex()
                .flex_col()
                .gap_4()
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(title.clone()))
                .child(div().text_sm().text_color(p.text2).child(body.clone()))
                .child(
                    div().flex().justify_end().gap_2().child(pill("dlg-cancel", None, "Cancel", cx).on_click(cx.listener(|this, _, _, cx| {
                        this.dialog = None;
                        cx.notify();
                    }))).child(
                        pill("dlg-ok", None, confirm.clone(), cx).bg(rgb(0xcc0000)).text_color(white()).on_click(cx.listener(|this, _, window, cx| {
                            if let Some(Dialog::Confirm { on_confirm, .. }) = this.dialog.take() {
                                on_confirm(window, cx);
                            }
                            cx.notify();
                        })),
                    ),
                )
                .into_any_element(),
        };
        Some(
            deferred(
                div()
                    .id("dialog-backdrop")
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(hsla(0., 0., 0., 0.5))
                    .occlude()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dialog = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id("dialog")
                            .w(px(if wide { 420. } else { 380. }))
                            .max_h(px(520.))
                            .overflow_y_scroll()
                            .p_5()
                            .rounded_xl()
                            .bg(p.surface2)
                            .shadow_2xl()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(body),
                    ),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }

    fn render_toasts(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let toasts = cx.default_global::<Toasts>().items.clone();
        div()
            .absolute()
            .left_4()
            .bottom_4()
            .flex()
            .flex_col()
            .gap_2()
            .children(toasts.into_iter().map(|t| {
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .bg(rgb(0x323232))
                    .text_sm()
                    .text_color(white())
                    .shadow_lg()
                    .child(t.text.clone())
                    .when_some(t.action.clone(), |d, (label, f)| {
                        let id = t.id;
                        d.child(
                            div()
                                .id(SharedString::from(format!("toast-{id}")))
                                .text_color(rgb(0x3ea6ff))
                                .font_weight(FontWeight::SEMIBOLD)
                                .cursor_pointer()
                                .child(label)
                                .on_click(move |_, window, cx| {
                                    f(window, cx);
                                    cx.default_global::<Toasts>().items.retain(|x| x.id != id);
                                    cx.refresh_windows();
                                }),
                        )
                    })
            }))
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf::add(&crate::perf::APP_RENDERS, 1);
        if std::mem::take(&mut self.reload_page) {
            // account changed: feeds that depend on it are rebuilt (the watch page keeps playing)
            if matches!(self.route, Route::Home | Route::Subscriptions | Route::Channels) {
                self.page = build_page(&self.route, window, cx);
            }
        }
        if let Some(id) = self.pending_watch_sync.take() {
            if let Route::Watch { id: cur, .. } = &self.route {
                if *cur != id {
                    let r = Route::Watch { id, playlist: None, start: None };
                    let page = build_page(&r, window, cx);
                    let old = std::mem::replace(&mut self.route, r);
                    let old_page = std::mem::replace(&mut self.page, page);
                    self.history.push((old, old_page));
                }
            }
        }
        let p = pal(cx);
        let scale = store::settings_snapshot().ui_scale.clamp(0.75, 1.5);
        if (f32::from(window.rem_size()) - 16.0 * scale).abs() > 0.01 {
            window.set_rem_size(px(16.0 * scale));
        }
        let vp = window.viewport_size();
        let ui = *cx.global::<UiState>();
        let is_watch = matches!(self.route, Route::Watch { .. });
        let sidebar_w = if is_watch {
            0.0
        } else if ui.sidebar_open {
            240.0
        } else {
            72.0
        };
        {
            let u = cx.global_mut::<UiState>();
            u.viewport = vp;
            u.content_width = vp.width - px(sidebar_w) - px(48.);
        }
        let fullscreen = ui.fullscreen && is_watch;
        let mut root = div()
            .id("app-root")
            .track_focus(&self.focus)
            .key_context("StreamTube")
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.text)
            .font_family(".SystemUIFont")
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, window, cx| this.back(window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _, window, cx| this.forward(window, cx)),
            );
        let has_slot = matches!(self.route, Route::Watch { .. } | Route::Shorts(_));
        let page_area = div()
            .relative()
            .overflow_hidden()
            .child(self.page.clone().cached(StyleRefinement::default().size_full()))
            .when(has_slot, |d| d.child(crate::ui::slot::SlotFollower::new(cx, self.main_view.clone().into())));
        if fullscreen {
            root = root.child(page_area.size_full());
        } else {
            let sidebar = if is_watch {
                // on watch pages the sidebar is an overlay drawer (like YouTube)
                None
            } else {
                Some(self.render_sidebar(cx))
            };
            root = root.child(self.render_topbar(window, cx)).child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar)
                    .child(page_area.flex_1().min_w_0().h_full()),
            );
            if is_watch && ui.sidebar_open && cx.global::<UiState>().sidebar_open {
                // drawer is closed by default on the watch page
            }
            root = root.children(self.render_mini_player(cx));
        }
        root = root.children(self.render_menu(cx)).children(self.render_dialog(cx)).child(self.render_toasts(cx));
        root
    }
}

/// Maximizes, or restores a maximized window.
pub fn toggle_maximize(window: &mut Window) {
    if !window.is_maximized() {
        window.zoom_window();
        return;
    }
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        #[link(name = "user32")]
        unsafe extern "system" {
            fn ShowWindowAsync(hwnd: isize, cmd: i32) -> i32;
        }
        if let Ok(h) = HasWindowHandle::window_handle(&*window) {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                const SW_RESTORE: i32 = 9;
                unsafe { ShowWindowAsync(w.hwnd.get(), SW_RESTORE) };
            }
        }
    }
}

/// Empty top-bar space that moves the window (with snap and double-click maximize).
fn drag_gap(id: &'static str) -> impl IntoElement {
    div().id(id).flex_1().min_w(px(24.)).h(px(56.)).window_control_area(WindowControlArea::Drag)
}

/// Minimize / maximize / close, drawn in the top bar. The OS handles the clicks
/// (WindowControlArea), so Snap Layouts on the maximize button work as usual.
fn window_controls(maximized: bool, cx: &App) -> impl IntoElement {
    let p = pal(cx);
    let btn = |id: &'static str, icon_name: &'static str, area: WindowControlArea, close: bool| {
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(px(46.))
            .h(px(56.))
            .text_color(p.text)
            .window_control_area(area)
            .when(close, |d| d.hover(|s| s.bg(rgb(0xc42b1c)).text_color(white())))
            .when(!close, move |d| d.hover(move |s| s.bg(p.hover)))
            .child(icon(icon_name).size_4().text_color(p.text))
            // Handle the click ourselves too: Windows only routes it to the caption button
            // when its hit test caught up with the cursor, which isn't always the case.
            // Stopping propagation keeps Windows from acting on it a second time.
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                match area {
                    WindowControlArea::Min => window.minimize_window(),
                    WindowControlArea::Max => toggle_maximize(window),
                    WindowControlArea::Close => {
                        crate::save_window_bounds(window);
                        window.remove_window();
                    }
                    WindowControlArea::Drag => {}
                }
            })
    };
    div()
        .flex()
        .h_full()
        .self_start()
        .child(btn("win-min", "minus", WindowControlArea::Min, false))
        .child(btn("win-max", if maximized { "copy" } else { "square" }, WindowControlArea::Max, false))
        .child(btn("win-close", "x", WindowControlArea::Close, true))
}

fn build_page(route: &Route, window: &mut Window, cx: &mut App) -> AnyView {
    match route {
        Route::Home => cx.new(|cx| home::HomePage::new(0, window, cx)).into(),
        Route::Explore(i) => cx.new(|cx| home::HomePage::new(i + 1, window, cx)).into(),
        Route::Shorts(id) => cx.new(|cx| shorts::ShortsPage::new(id.clone(), window, cx)).into(),
        Route::Subscriptions => cx.new(|cx| subscriptions::FeedPage::new(window, cx)).into(),
        Route::Channels => cx.new(|cx| subscriptions::ChannelsPage::new(window, cx)).into(),
        Route::History => cx.new(|cx| library::HistoryPage::new(window, cx)).into(),
        Route::Library => cx.new(|cx| library::LibraryPage::new(window, cx)).into(),
        Route::LocalPlaylist(id) => cx.new(|cx| playlist::PlaylistPage::local(*id, window, cx)).into(),
        Route::Playlist(id) => cx.new(|cx| playlist::PlaylistPage::remote(id.clone(), window, cx)).into(),
        Route::Search(q) => cx.new(|cx| search::SearchPage::new(q.clone(), window, cx)).into(),
        Route::Channel(id) => cx.new(|cx| channel::ChannelPage::new(id.clone(), window, cx)).into(),
        Route::Watch { id, playlist, start } => {
            cx.new(|cx| watch::WatchPage::new(id.clone(), playlist.clone(), *start, window, cx)).into()
        }
        Route::Settings => cx.new(|cx| settings::SettingsPage::new(window, cx)).into(),
        Route::Downloads => cx.new(|cx| downloads::DownloadsPage::new(window, cx)).into(),
    }
}
