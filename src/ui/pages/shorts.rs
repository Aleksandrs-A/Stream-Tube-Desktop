use crate::api::innertube::{self, SearchFilter};
use crate::api::json::compact_number;
use crate::api::services;
use crate::api::types::*;
use crate::player::controller::player;
use crate::store;
use crate::ui::app::{Route, UiState, navigate};
use crate::ui::*;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct ShortsPage {
    list: Vec<VideoItem>,
    idx: usize,
    loading: bool,
    votes: HashMap<String, services::Votes>,
    last_step: Instant,
    /// wheel / touchpad gesture tracking: one Short per gesture
    wheel_acc: f32,
    last_wheel: Instant,
    wheel_used: bool,
    continuation: Option<String>,
}

impl ShortsPage {
    pub fn new(start: Option<String>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.on_release(|_, cx| {
            let l = store::settings_snapshot().loop_video;
            player(cx).update(cx, |p, _| p.set_loop(l));
        })
        .detach();
        let mut s = Self {
            list: Vec::new(),
            idx: 0,
            loading: true,
            votes: HashMap::new(),
            last_step: Instant::now(),
            wheel_acc: 0.0,
            last_wheel: Instant::now(),
            wheel_used: false,
            continuation: None,
        };
        if let Some(id) = &start {
            s.list.push(VideoItem { id: id.clone(), is_short: true, thumbnail: innertube::default_thumb(id), ..Default::default() });
            s.play_current(cx);
        }
        let start2 = start.clone();
        cx.spawn(async move |this, cx| {
            let res = blocking(move || {
                // subscription shorts first, then a general shorts search
                let (feed, _) = crate::feed::cached_feed(None);
                let mut out: Vec<VideoItem> = feed.into_iter().filter(|v| v.is_short).take(30).collect();
                let f = SearchFilter { duration: 1, kind: 1, ..Default::default() };
                let mut cont = None;
                if let Ok(p) = innertube::search("#shorts", &f) {
                    cont = p.continuation;
                    out.extend(p.items.into_iter().filter_map(|i| match i {
                        Item::Video(mut v) => {
                            v.is_short = true;
                            Some(v)
                        }
                        _ => None,
                    }));
                }
                (out, cont)
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                let s = store::settings_snapshot();
                let mut seen: std::collections::HashSet<String> = this.list.iter().map(|v| v.id.clone()).collect();
                for v in res.0 {
                    if !store::is_filtered(&VideoItem { is_short: false, ..v.clone() }, &s) && seen.insert(v.id.clone()) {
                        this.list.push(v);
                    }
                }
                this.continuation = res.1;
                if start2.is_none() && !this.list.is_empty() {
                    this.play_current(cx);
                }
                cx.notify();
            });
        })
        .detach();
        s
    }

    fn play_current(&mut self, cx: &mut Context<Self>) {
        let Some(v) = self.list.get(self.idx).cloned() else { return };
        let pl = player(cx);
        pl.update(cx, |p, cx| {
            p.queue.clear();
            p.queue_pos = None;
            p.set_loop(true);
            p.play(v.clone(), Some(0.0), cx);
        });
        if store::settings_snapshot().ryd && !self.votes.contains_key(&v.id) {
            let id = v.id.clone();
            cx.spawn(async move |this, cx| {
                let id2 = id.clone();
                if let Ok(vt) = blocking(move || services::ryd_votes(&id2)).await {
                    let _ = this.update(cx, |t, cx| {
                        t.votes.insert(id, vt);
                        cx.notify();
                    });
                }
            })
            .detach();
        }
        // prefetch more when near the end
        if self.idx + 3 >= self.list.len() {
            if let Some(tok) = self.continuation.take() {
                cx.spawn(async move |this, cx| {
                    if let Ok(p) = blocking(move || innertube::search_continue(&tok)).await {
                        let _ = this.update(cx, |t, cx| {
                            t.continuation = p.continuation;
                            for i in p.items {
                                if let Item::Video(mut v) = i {
                                    v.is_short = true;
                                    if !t.list.iter().any(|x| x.id == v.id) {
                                        t.list.push(v);
                                    }
                                }
                            }
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
        }
        cx.notify();
    }

    pub fn step(&mut self, d: i32, _window: &mut Window, cx: &mut Context<Self>) {
        if self.last_step.elapsed() < Duration::from_millis(350) {
            return;
        }
        let n = self.idx as i32 + d;
        if n < 0 || n as usize >= self.list.len() {
            return;
        }
        self.last_step = Instant::now();
        self.idx = n as usize;
        self.play_current(cx);
    }
}

impl Render for ShortsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let ui = *cx.global::<UiState>();
        if self.list.is_empty() {
            return div().size_full().child(if self.loading { spinner(cx).into_any_element() } else { empty_state("zap", "No Shorts found", "", cx).into_any_element() }).into_any_element();
        }
        let v = self.list[self.idx].clone();
        let h = (ui.viewport.height - px(56. + 32.)).max(px(400.));
        let w = h * (9. / 16.);
        let votes = self.votes.get(&v.id).cloned();
        let cid = v.channel_id.clone();
        let watch_id = v.id.clone();
        let share_id = v.id.clone();
        let action = |id: &'static str, ic: &'static str, label: String, cx: &App| {
            let p = pal(cx);
            div()
                .id(id)
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .child(div().flex().items_center().justify_center().size_12().rounded_full().bg(p.chip).hover(move |s| s.bg(p.hover)).child(icon(ic).size_6().text_color(p.text)))
                .child(div().text_xs().font_weight(FontWeight::MEDIUM).text_color(p.text).child(label))
        };
        let can_up = self.idx > 0;
        let can_down = self.idx + 1 < self.list.len();
        div()
            .id("shorts")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap_4()
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, window, cx| {
                // A touchpad swipe is a stream of small deltas and a wheel notch one big one.
                // Sum them, move one Short past a threshold, and ignore the rest of that
                // gesture (it ends after a short pause).
                let dy: f32 = ev.delta.pixel_delta(window.line_height()).y.into();
                if this.last_wheel.elapsed() > Duration::from_millis(220) {
                    this.wheel_acc = 0.0;
                    this.wheel_used = false;
                }
                this.last_wheel = Instant::now();
                if this.wheel_used {
                    return;
                }
                this.wheel_acc += dy;
                if this.wheel_acc.abs() >= 48.0 {
                    let d = if this.wheel_acc < 0.0 { 1 } else { -1 };
                    this.wheel_acc = 0.0;
                    this.wheel_used = true;
                    this.last_step = Instant::now() - Duration::from_secs(1);
                    this.step(d, window, cx);
                }
            }))
            .child(
                div()
                    .relative()
                    .w(w)
                    .h(h)
                    .rounded_xl()
                    .overflow_hidden()
                    .bg(black())
                    .child(crate::ui::slot::player_slot(cx))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom(px(70.))
                            .px_4()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .id("sh-chan")
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .cursor_pointer()
                                    .on_click(move |_, window, cx| {
                                        if let Some(c) = cid.clone() {
                                            navigate(Route::Channel(c), window, cx);
                                        }
                                    })
                                    .child(avatar(v.channel_avatar.as_deref(), px(32.), &v.channel_name, cx))
                                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(white()).child(v.channel_name.clone())),
                            )
                            .child(div().text_sm().text_color(white()).line_clamp(2).child(v.title.clone())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .h(h)
                    .gap_4()
                    .pb_4()
                    .child(action("sh-like", "thumbs-up", votes.as_ref().map(|v| compact_number(v.likes)).unwrap_or_else(|| "Like".into()), cx))
                    .child(action("sh-dislike", "thumbs-down", votes.as_ref().map(|v| compact_number(v.dislikes)).unwrap_or_else(|| "Dislike".into()), cx))
                    .child(action("sh-share", "share-2", "Share".into(), cx).on_click(move |_, _, cx| copy_to_clipboard(format!("https://youtube.com/shorts/{share_id}"), cx)))
                    .child(action("sh-watch", "monitor-play", "Full".into(), cx).on_click(move |_, window, cx| {
                        navigate(Route::Watch { id: watch_id.clone(), playlist: None, start: None }, window, cx)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(icon_button("sh-up", "arrow-up", cx).size_12().bg(p.chip).when(!can_up, |d| d.opacity(0.3)).on_click(cx.listener(|t, _, window, cx| t.step(-1, window, cx))))
                    .child(icon_button("sh-down", "arrow-down", cx).size_12().bg(p.chip).when(!can_down, |d| d.opacity(0.3)).on_click(cx.listener(|t, _, window, cx| t.step(1, window, cx)))),
            )
            .into_any_element()
    }
}
