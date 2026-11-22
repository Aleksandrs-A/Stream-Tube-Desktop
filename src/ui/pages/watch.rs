use crate::api::innertube;
use crate::api::json::compact_number;
use crate::api::services::{self, Votes};
use crate::api::types::*;
use crate::player::controller::{PlayerEvent, player};
use crate::store;
use crate::ui::app::{AutoplayNext, MenuItem, Route, UiState, navigate, open_menu};
use crate::ui::cards::*;
use crate::ui::widgets::*;
use crate::ui::*;
use std::collections::HashMap;

pub struct WatchPage {
    id: String,
    playlist_id: Option<String>,
    info: Load<WatchInfo>,
    votes: Option<Votes>,
    desc_expanded: bool,
    comments: Option<Load<CommentPage>>,
    comments_more_loading: bool,
    comment_sort_new: bool,
    replies: HashMap<String, Load<(Vec<Comment>, Option<String>)>>,
    related_more_loading: bool,
    scroll: ScrollHandle,
    local_playlist: Option<(i64, String, Vec<VideoItem>)>,
    _sub: Subscription,
}

impl WatchPage {
    pub fn new(id: String, playlist_id: Option<String>, start: Option<f64>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pl = player(cx);
        let sub = cx.subscribe(&pl, |this: &mut Self, _, ev: &PlayerEvent, cx| {
            match ev {
                PlayerEvent::Ended => this.on_ended(cx),
                PlayerEvent::Ambient => cx.notify(),
                _ => {}
            }
        });
        // local playlist "local:<id>"
        let mut local_playlist = None;
        if let Some(lp) = playlist_id.as_deref().and_then(|p| p.strip_prefix("local:")).and_then(|n| n.parse::<i64>().ok()) {
            let name = store::playlists().into_iter().find(|p| p.id == lp).map(|p| p.name).unwrap_or_default();
            local_playlist = Some((lp, name, store::playlist_items(lp)));
        }
        let playing = pl.read(cx).video.as_ref().map(|v| v.id.clone());
        if playing.as_deref() != Some(id.as_str()) {
            let stub = VideoItem { id: id.clone(), thumbnail: innertube::default_thumb(&id), ..Default::default() };
            pl.update(cx, |p, cx| {
                // keep queue if this video is in it
                if let Some(pos) = p.queue.iter().position(|v| v.id == id) {
                    p.queue_pos = Some(pos);
                } else if let Some((_, _, items)) = &local_playlist {
                    p.queue = items.clone();
                    p.queue_pos = items.iter().position(|v| v.id == id);
                } else {
                    p.queue.clear();
                    p.queue_pos = None;
                }
                p.play(stub, start, cx);
            });
        } else if let Some(t) = start {
            pl.update(cx, |p, _| p.seek_to(t));
        }
        cx.set_global(AutoplayNext(None));
        let mut s = Self {
            id,
            playlist_id,
            info: Load::Loading,
            votes: None,
            desc_expanded: false,
            comments: None,
            comments_more_loading: false,
            comment_sort_new: false,
            replies: HashMap::new(),
            related_more_loading: false,
            scroll: ScrollHandle::new(),
            local_playlist,
            _sub: sub,
        };
        s.load(cx);
        s
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let id = self.id.clone();
        let pid = self.playlist_id.clone().filter(|p| !p.starts_with("local:"));
        let ryd = store::settings_snapshot().ryd;
        cx.spawn(async move |this, cx| {
            let id2 = id.clone();
            let info_f = blocking(move || innertube::watch_next(&id2, pid.as_deref()));
            let id3 = id.clone();
            let votes_f = blocking(move || if ryd { services::ryd_votes(&id3).ok() } else { None });
            let (info, votes) = futures::join!(info_f, votes_f);
            let _ = this.update(cx, |this, cx| {
                this.votes = votes;
                match info {
                    Ok(w) => {
                        let s = store::settings_snapshot();
                        let pl = player(cx);
                        let chapters = w.chapters.clone();
                        let heatmap = w.heatmap.clone();
                        let full = VideoItem {
                            id: w.id.clone(),
                            title: w.title.clone(),
                            channel_name: w.channel.name.clone(),
                            channel_id: Some(w.channel.id.clone()).filter(|c| !c.is_empty()),
                            channel_avatar: w.channel.avatar.clone(),
                            thumbnail: innertube::default_thumb(&w.id),
                            views_text: Some(w.views_text.clone()),
                            published_text: Some(w.date_text.clone()),
                            is_live: w.is_live,
                            ..Default::default()
                        };
                        pl.update(cx, |p, cx| {
                            if p.video.as_ref().map(|v| v.id == full.id).unwrap_or(false) {
                                p.chapters = chapters;
                                p.heatmap = heatmap;
                                let dur = p.video.as_ref().and_then(|v| v.duration_secs);
                                let mut v = full.clone();
                                v.duration_secs = dur.or(Some(p.duration as u64));
                                v.duration_text = v.duration_secs.filter(|d| *d > 0).map(crate::api::json::format_duration);
                                if p.channel_id.is_none() {
                                    p.channel_id = v.channel_id.clone();
                                }
                                p.video = Some(v.clone());
                                store::history_add(&v);
                                // playlist queue
                                if let Some(wp) = &w.playlist {
                                    if !wp.items.is_empty() {
                                        p.queue = wp.items.clone();
                                        p.queue_pos = wp.items.iter().position(|x| x.id == full.id);
                                    }
                                }
                                cx.notify();
                            }
                        });
                        let next = w
                            .related
                            .iter()
                            .filter_map(|i| i.as_video())
                            .find(|v| !v.is_short && !v.is_live && !store::is_filtered(v, &s))
                            .cloned();
                        if let Some(n) = &next {
                            // warm up the autoplay candidate so the next video starts instantly
                            let id = n.id.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_secs(8));
                                crate::api::streams::prefetch(&id);
                            });
                        }
                        cx.set_global(AutoplayNext(next));
                        let comments_token = w.comments_token.clone();
                        this.info = Load::Ready(w);
                        if !s.hide_comments {
                            if let Some(t) = comments_token {
                                this.load_comments(t, false, cx);
                            }
                        }
                    }
                    Err(e) => this.info = Load::Error(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn on_ended(&mut self, cx: &mut Context<Self>) {
        let s = store::settings_snapshot();
        let next = cx.global::<AutoplayNext>().0.clone().filter(|_| s.autoplay_next);
        let looping = player(cx).read(cx).loop_video;
        if next.is_none() && !looping && s.exit_fullscreen_at_end && cx.global::<UiState>().fullscreen {
            if let Some(w) = cx.active_window() {
                let _ = w.update(cx, |_, window, cx| crate::ui::app::toggle_fullscreen(window, cx));
            }
        }
        if !s.autoplay_next {
            return;
        }
        if let Some(v) = next {
            toast(cx, format!("Up next: {}", v.title));
            let id = v.id.clone();
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                let _ = cx.update(|cx| {
                    let still_ended = player(cx).read(cx).eof;
                    if still_ended {
                        if let Some(w) = cx.active_window() {
                            let _ = w.update(cx, |_, window, cx| navigate(Route::Watch { id, playlist: None, start: None }, window, cx));
                        }
                    }
                });
            })
            .detach();
        }
    }

    fn load_comments(&mut self, token: String, append: bool, cx: &mut Context<Self>) {
        if !append {
            self.comments = Some(Load::Loading);
        }
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::comments(&token)).await;
            let _ = this.update(cx, |this, cx| {
                this.comments_more_loading = false;
                match (res, append) {
                    (Ok(page), true) => {
                        if let Some(Load::Ready(c)) = this.comments.as_mut() {
                            c.comments.extend(page.comments);
                            c.continuation = page.continuation;
                        }
                    }
                    (Ok(page), false) => this.comments = Some(Load::Ready(page)),
                    (Err(e), false) => this.comments = Some(Load::Error(e.to_string())),
                    (Err(_), true) => {
                        if let Some(Load::Ready(c)) = this.comments.as_mut() {
                            c.continuation = None;
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_replies(&mut self, comment_id: String, token: String, cx: &mut Context<Self>) {
        let append = matches!(self.replies.get(&comment_id), Some(Load::Ready(_)));
        if !append {
            self.replies.insert(comment_id.clone(), Load::Loading);
        }
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::comments(&token)).await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(p) => {
                        if let Some(Load::Ready((list, cont))) = this.replies.get_mut(&comment_id) {
                            list.extend(p.comments);
                            *cont = p.continuation;
                        } else {
                            this.replies.insert(comment_id, Load::Ready((p.comments, p.continuation)));
                        }
                    }
                    Err(e) => {
                        this.replies.insert(comment_id, Load::Error(e.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_more_related(&mut self, cx: &mut Context<Self>) {
        let Some(tok) = self.info.ready().and_then(|w| w.related_continuation.clone()) else {
            self.related_more_loading = false;
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::related_continue(&tok)).await;
            let _ = this.update(cx, |this, cx| {
                this.related_more_loading = false;
                if let (Ok(p), Some(w)) = (res, this.info.ready_mut()) {
                    w.related.extend(p.items);
                    w.related_continuation = p.continuation;
                } else if let Some(w) = this.info.ready_mut() {
                    w.related_continuation = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ───── sections ─────

    fn render_info(&mut self, w: &WatchInfo, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        let s = store::settings_snapshot();
        let subscribed = store::is_subscribed(&w.channel.id);
        let rating = store::rating(&w.id);
        let likes = self
            .votes
            .as_ref()
            .map(|v| compact_number(v.likes + (rating == 1) as u64))
            .or(w.likes_text.clone())
            .unwrap_or_else(|| "Like".into());
        let dislikes = self.votes.as_ref().map(|v| compact_number(v.dislikes));
        let ratio = self.votes.as_ref().filter(|v| v.likes + v.dislikes > 0).map(|v| v.likes as f32 / (v.likes + v.dislikes) as f32);
        let video_item = player(cx).read(cx).video.clone().unwrap_or_else(|| VideoItem { id: w.id.clone(), title: w.title.clone(), ..Default::default() });
        let cid = w.channel.id.clone();
        let title = if s.dearrow_titles {
            services::branding_cached(&w.id).flatten().and_then(|b| b.title).unwrap_or(w.title.clone())
        } else {
            w.title.clone()
        };
        let v_dl = video_item.clone();
        let v_save = video_item.clone();
        let v_more = video_item.clone();
        let share_id = w.id.clone();
        let channel = w.channel.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .pt_3()
            .child(div().text_xl().font_weight(FontWeight::BOLD).text_color(p.text).child(title))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .id("w-chan")
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .cursor_pointer()
                                    .on_click(move |_, window, cx| navigate(Route::Channel(cid.clone()), window, cx))
                                    .child(avatar(w.channel.avatar.as_deref(), px(40.), &w.channel.name, cx))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(w.channel.name.clone()))
                                            .when_some(w.channel.subscribers_text.clone(), |d, t| d.child(div().text_xs().text_color(p.text2).child(t))),
                                    ),
                            )
                            .child(div().ml_3().child(subscribe_button("w-sub".into(), &channel, subscribed, cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .h_9()
                                            .rounded_full()
                                            .bg(p.chip)
                                            .text_sm()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.text)
                                            .child(
                                                div()
                                                    .id("w-like")
                                                    .flex()
                                                    .items_center()
                                                    .h_full()
                                                    .gap_2()
                                                    .pl_4()
                                                    .pr_3()
                                                    .rounded_l_full()
                                                    .cursor_pointer()
                                                    .when(rating == 1, |d| d.bg(p.hover))
                                                    .hover(move |st| st.bg(p.hover))
                                                    .child(icon("thumbs-up").size_5().text_color(if rating == 1 { p.accent } else { p.text }))
                                                    .child(likes)
                                                    .on_click({
                                                        let id = w.id.clone();
                                                        move |_, _, cx| set_rating(&id, if rating == 1 { 0 } else { 1 }, cx)
                                                    }),
                                            )
                                            .child(div().w(px(1.)).h_6().bg(p.border))
                                            .child(
                                                div()
                                                    .id("w-dislike")
                                                    .flex()
                                                    .items_center()
                                                    .h_full()
                                                    .gap_2()
                                                    .pl_3()
                                                    .pr_4()
                                                    .rounded_r_full()
                                                    .cursor_pointer()
                                                    .when(rating == -1, |d| d.bg(p.hover))
                                                    .hover(move |st| st.bg(p.hover))
                                                    .child(icon("thumbs-down").size_5().text_color(if rating == -1 { p.accent } else { p.text }))
                                                    .when_some(dislikes, |d, t| d.child(t))
                                                    .on_click({
                                                        let id = w.id.clone();
                                                        move |_, _, cx| set_rating(&id, if rating == -1 { 0 } else { -1 }, cx)
                                                    }),
                                            ),
                                    )
                                    .when_some(ratio, |d, r| {
                                        d.child(div().mx_4().mt_1().h(px(2.)).rounded_full().bg(p.text3).child(div().h_full().w(relative(r)).bg(p.text)))
                                    }),
                            )
                            .child(pill("w-share", Some("share-2"), "Share", cx).on_click(move |_, _, cx| {
                                copy_to_clipboard(format!("https://youtu.be/{share_id}"), cx)
                            }))
                            .child(pill("w-dl", Some("download"), "Download", cx).on_click(move |ev, window, cx| {
                                let items: Vec<MenuItem> = crate::downloads::FORMATS
                                    .iter()
                                    .map(|(f, label)| {
                                        let v = v_dl.clone();
                                        let f = f.to_string();
                                        MenuItem::new(if f == "audio" { "music" } else { "film" }, *label, move |_, cx| {
                                            crate::downloads::start(v.clone(), Some(f.clone()), cx)
                                        })
                                    })
                                    .collect();
                                open_menu(ev.position(), items, window, cx);
                            }))
                            .child(pill("w-save", Some("bookmark"), "Save", cx).on_click(move |_, window, cx| {
                                crate::ui::app::open_save_dialog(v_save.clone(), window, cx)
                            }))
                            .child(
                                pill("w-more", Some("ellipsis"), "", cx).w_9().px_0().justify_center().on_click(move |ev, window, cx| {
                                    open_menu(ev.position(), video_menu_items(&v_more), window, cx);
                                }),
                            ),
                    ),
            )
            .when(!s.hide_description, |d| d.child(self.render_description(w, cx)))
            .into_any_element()
    }

    fn render_description(&mut self, w: &WatchInfo, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        let expanded = self.desc_expanded;
        let s = store::settings_snapshot();
        let views = if s.hide_view_counts { String::new() } else { w.views_text.clone() };
        let desc = if expanded {
            w.description.clone()
        } else {
            w.description.lines().take(3).collect::<Vec<_>>().join("\n")
        };
        let chapters = w.chapters.clone();
        div()
            .id("desc")
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_xl()
            .bg(p.chip)
            .when(!expanded, |d| d.cursor_pointer().hover(move |s| s.bg(p.hover)))
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.desc_expanded {
                    this.desc_expanded = true;
                    cx.notify();
                }
            }))
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(format!("{views}  {}", w.date_text)))
            .child(div().text_sm().text_color(p.text).child(rich_text("desc-text", desc, cx)))
            .when(expanded && !chapters.is_empty() && !s.hide_chapters, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .pt_3()
                        .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child("Chapters"))
                        .child(div().flex().gap_2().overflow_hidden().children(chapters.into_iter().enumerate().map(|(i, c)| {
                            let t = c.start;
                            div()
                                .id(SharedString::from(format!("chap-{i}")))
                                .flex()
                                .flex_col()
                                .flex_none()
                                .w(px(160.))
                                .gap_1()
                                .p_1()
                                .rounded_lg()
                                .cursor_pointer()
                                .hover(move |s| s.bg(p.hover))
                                .when_some(c.thumbnail.clone(), |d, th| d.child(img(SharedString::from(th)).w(px(152.)).h(px(86.)).rounded_md()))
                                .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(p.text).line_clamp(2).child(c.title.clone()))
                                .child(div().text_xs().text_color(p.link).child(crate::api::json::format_duration(t as u64)))
                                .on_click(move |_, _, cx| player(cx).update(cx, |p, _| p.seek_to(t)))
                        }))),
                )
            })
            .child(
                div()
                    .id("desc-toggle")
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text)
                    .cursor_pointer()
                    .child(if expanded { "Show less" } else { "...more" })
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.desc_expanded ^= true;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_comment(&mut self, c: &Comment, is_reply: bool, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        let cid = c.id.clone();
        let token = c.replies_token.clone();
        let replies = self.replies.get(&c.id);
        let author_id = c.author_id.clone();
        let mut el = div()
            .flex()
            .gap_4()
            .child(avatar(c.author_avatar.as_deref(), px(if is_reply { 24. } else { 40. }), &c.author, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .when(c.is_pinned, |d| d.child(div().flex().items_center().gap_1().text_xs().text_color(p.text2).child(icon("pin").size_3()).child("Pinned")))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id(SharedString::from(format!("ca-{}", c.id)))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .cursor_pointer()
                                    .when(c.is_creator, |d| d.px_2().rounded_full().bg(p.hover))
                                    .text_color(p.text)
                                    .child(c.author.clone())
                                    .on_click(move |_, window, cx| {
                                        if let Some(a) = author_id.clone() {
                                            navigate(Route::Channel(a), window, cx)
                                        }
                                    }),
                            )
                            .when(c.is_verified, |d| d.child(icon("badge-check").size_3().text_color(p.text2)))
                            .child(div().text_xs().text_color(p.text2).child(c.published.clone())),
                    )
                    .child(div().text_sm().text_color(p.text).child(rich_text(SharedString::from(format!("ct-{}", c.id)), c.text.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .text_xs()
                            .text_color(p.text2)
                            .child(div().flex().items_center().gap_1().child(icon("thumbs-up").size_4().text_color(p.text2)).child(c.likes.clone()))
                            .child(icon("thumbs-down").size_4().text_color(p.text2))
                            .when(c.hearted, |d| d.child(icon("heart").size_4().text_color(rgb(0xff0033)))),
                    )
                    .when_some(token.filter(|_| !is_reply), |d, tok| {
                        let open = replies.is_some();
                        let cid2 = cid.clone();
                        d.child(
                            div()
                                .id(SharedString::from(format!("cr-btn-{}", c.id)))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_3()
                                .py_1p5()
                                .rounded_full()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.link)
                                .cursor_pointer()
                                .hover(move |s| s.bg(p.hover))
                                .child(icon(if open { "chevron-up" } else { "chevron-down" }).size_4().text_color(p.link))
                                .child(format!("{} {}", c.reply_count, if c.reply_count == "1" { "reply" } else { "replies" }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.replies.contains_key(&cid2) {
                                        this.replies.remove(&cid2);
                                        cx.notify();
                                    } else {
                                        this.load_replies(cid2.clone(), tok.clone(), cx);
                                    }
                                })),
                        )
                    }),
            )
            .into_any_element();
        if let Some(r) = replies {
            let r: Option<Result<(Vec<Comment>, Option<String>), String>> = match r {
                Load::Loading => None,
                Load::Error(e) => Some(Err(e.clone())),
                Load::Ready((l, c)) => Some(Ok((l.clone(), c.clone()))),
            };
            let mut col = div().flex().flex_col().gap_4().pl(px(56.));
            match r {
                None => col = col.child(spinner(cx)),
                Some(Err(e)) => col = col.child(div().text_xs().text_color(p.text2).child(e)),
                Some(Ok((list, cont))) => {
                    for rc in &list {
                        col = col.child(self.render_comment(rc, true, cx));
                    }
                    if let Some(tok) = cont {
                        let cid3 = cid.clone();
                        col = col.child(
                            div()
                                .id(SharedString::from(format!("cr-more-{cid}")))
                                .text_sm()
                                .text_color(p.link)
                                .cursor_pointer()
                                .child("Show more replies")
                                .on_click(cx.listener(move |this, _, _, cx| this.load_replies(cid3.clone(), tok.clone(), cx))),
                        );
                    }
                }
            }
            el = div().flex().flex_col().gap_3().child(el).child(col).into_any_element();
        }
        el
    }

    fn render_comments(&mut self, w: &WatchInfo, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        if store::settings_snapshot().hide_comments {
            return div().into_any_element();
        }
        let Some(state) = &self.comments else {
            return div().py_4().text_sm().text_color(p.text2).child(if w.comments_token.is_none() { "Comments are turned off." } else { "" }).into_any_element();
        };
        let data = match state {
            Load::Loading => return spinner(cx).into_any_element(),
            Load::Error(e) => return div().py_4().text_sm().text_color(p.text2).child(format!("Could not load comments: {e}")).into_any_element(),
            Load::Ready(c) => c.clone(),
        };
        let sort_new = self.comment_sort_new;
        let (top, new) = (data.sort_top_token.clone(), data.sort_new_token.clone());
        let mut col = div().flex().flex_col().gap_6().pt_6().child(
            div()
                .flex()
                .items_center()
                .gap_6()
                .child(div().text_xl().font_weight(FontWeight::BOLD).text_color(p.text).child(format!("{} Comments", data.count_text.clone().or(w.comment_count_text.clone()).unwrap_or_default())))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .when_some(top, |d, t| {
                            d.child(chip("c-top", "Top", !sort_new, cx).on_click(cx.listener(move |this, _, _, cx| {
                                this.comment_sort_new = false;
                                this.load_comments(t.clone(), false, cx);
                            })))
                        })
                        .when_some(new, |d, t| {
                            d.child(chip("c-new", "Newest", sort_new, cx).on_click(cx.listener(move |this, _, _, cx| {
                                this.comment_sort_new = true;
                                this.load_comments(t.clone(), false, cx);
                            })))
                        }),
                ),
        );
        for c in &data.comments {
            col = col.child(self.render_comment(c, false, cx));
        }
        if data.continuation.is_some() && self.comments_more_loading {
            col = col.child(spinner(cx));
        }
        col.into_any_element()
    }

    fn render_queue_panel(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = pal(cx);
        let pl = player(cx);
        let (queue, pos) = {
            let r = pl.read(cx);
            (r.queue.clone(), r.queue_pos)
        };
        if queue.len() < 2 {
            return None;
        }
        let title = self
            .info
            .ready()
            .and_then(|w| w.playlist.as_ref().map(|p| p.title.clone()))
            .or(self.local_playlist.as_ref().map(|l| l.1.clone()))
            .unwrap_or_else(|| "Queue".into());
        let mut list = div().id("queue-list").flex().flex_col().max_h(px(400.)).overflow_y_scroll();
        for (i, v) in queue.iter().enumerate() {
            let plc = pl.clone();
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().w_5().text_xs().text_color(p.text2).text_center().child(if pos == Some(i) { "▶".to_string() } else { (i + 1).to_string() }))
                    .child(compact_row(
                        v,
                        pos == Some(i),
                        Some(Arc::new(move |_, cx: &mut App| plc.update(cx, |p, cx| p.play_queue_index(i, cx)))),
                        cx,
                    )),
            );
        }
        let plc2 = pl.clone();
        let plc3 = pl.clone();
        Some(
            div()
                .flex()
                .flex_col()
                .rounded_xl()
                .border_1()
                .border_color(p.border)
                .overflow_hidden()
                .mb_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .p_3()
                        .bg(p.surface)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_base().font_weight(FontWeight::BOLD).text_color(p.text).child(title))
                                .child(div().text_xs().text_color(p.text2).child(format!("{} / {}", pos.map(|p| p + 1).unwrap_or(0), queue.len()))),
                        )
                        .child(
                            div()
                                .flex()
                                .child(icon_button("q-shuffle", "shuffle", cx).on_click(move |_, _, cx| {
                                    plc2.update(cx, |p, cx| {
                                        use rand::seq::SliceRandom;
                                        let cur = p.queue_pos.and_then(|i| p.queue.get(i).cloned());
                                        p.queue.shuffle(&mut rand::rng());
                                        if let Some(c) = cur {
                                            p.queue_pos = p.queue.iter().position(|v| v.id == c.id);
                                        }
                                        cx.notify();
                                    })
                                }))
                                .child(icon_button("q-clear", "x", cx).on_click(move |_, _, cx| {
                                    plc3.update(cx, |p, cx| {
                                        p.queue.clear();
                                        p.queue_pos = None;
                                        cx.notify();
                                    })
                                })),
                        ),
                )
                .child(list)
                .into_any_element(),
        )
    }

    fn render_related(&mut self, w: &WatchInfo, cx: &mut Context<Self>) -> AnyElement {
        let p = pal(cx);
        let s = store::settings_snapshot();
        if s.hide_related {
            return div().into_any_element();
        }
        let autoplay = s.autoplay_next;
        let mut col = div().flex().flex_col().gap_2().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .pb_1()
                .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child("Up next"))
                .child(
                    div()
                        .id("autoplay")
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(p.text2)
                        .cursor_pointer()
                        .child("Autoplay")
                        .child(
                            div()
                                .w(px(36.))
                                .h(px(14.))
                                .rounded_full()
                                .bg(if autoplay { p.text2 } else { p.hover })
                                .relative()
                                .child(div().absolute().top(px(-3.)).left(if autoplay { px(16.) } else { px(-2.) }).size(px(20.)).rounded_full().bg(if autoplay { p.text } else { p.text3 })),
                        )
                        .on_click(|_, _, cx| {
                            store::update_settings(|s| s.autoplay_next ^= true);
                            cx.refresh_windows();
                        }),
                ),
        );
        let mut shorts = Vec::new();
        for it in &w.related {
            match it {
                Item::Video(v) if store::is_filtered(v, &s) => {}
                Item::Video(v) if v.is_short => shorts.push(v.clone()),
                Item::Video(v) => col = col.child(compact_row(v, false, None, cx)),
                Item::Playlist(pl) => {
                    let id = pl.id.clone();
                    col = col.child(
                        div()
                            .id(SharedString::from(format!("rp-{}", pl.id)))
                            .flex()
                            .gap_2()
                            .p_1()
                            .cursor_pointer()
                            .on_click(move |_, window, cx| navigate(Route::Playlist(id.clone()), window, cx))
                            .child(playlist_thumb(pl.thumbnail.clone(), pl.video_count_text.clone(), px(168.), px(94.), cx))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(p.text).line_clamp(2).child(pl.title.clone()))
                                    .when_some(pl.channel_name.clone(), |d, c| d.child(div().text_xs().text_color(p.text2).child(c))),
                            ),
                    );
                }
                Item::Channel(_) => {}
            }
        }
        if !shorts.is_empty() && !s.hide_shorts {
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .py_2()
                    .child(div().flex().items_center().gap_2().child(icon("zap").text_color(p.accent)).child(div().font_weight(FontWeight::SEMIBOLD).child("Shorts")))
                    .child(div().flex().gap_2().overflow_hidden().children(shorts.iter().take(3).map(|v| shorts_card(v, px(126.), cx)))),
            );
        }
        if self.related_more_loading {
            col = col.child(spinner(cx));
        }
        col.into_any_element()
    }
}

impl Render for WatchPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::perf::add(&crate::perf::PAGE_RENDERS, 1);
        let p = pal(cx);
        let ui = *cx.global::<UiState>();
        // infinite scroll: comments, then related
        if near_bottom(&self.scroll, 600.0) {
            if let Some(Load::Ready(c)) = &self.comments {
                if let (Some(tok), false) = (c.continuation.clone(), self.comments_more_loading) {
                    self.comments_more_loading = true;
                    let t = tok.clone();
                    cx.defer_in_entity(move |this, cx| this.load_comments(t, true, cx));
                }
            }
            let has_rel = self.info.ready().map(|w| w.related_continuation.is_some()).unwrap_or(false);
            if has_rel && !self.related_more_loading {
                self.related_more_loading = true;
                cx.defer_in_entity(|this, cx| this.load_more_related(cx));
            }
        }
        if ui.fullscreen {
            return div().size_full().bg(black()).child(crate::ui::slot::player_slot(cx)).into_any_element();
        }
        let vp = ui.viewport;
        let avail = vp.width - px(48.);
        let side_w = px(402.);
        let two_col = avail > px(1000.);
        let primary_w = if two_col { (avail - side_w - px(24.)).min(px(1280.)) } else { avail };
        let player_h = if ui.theater {
            (vp.width * (9. / 16.)).min(vp.height - px(56. + 120.))
        } else {
            primary_w * (9. / 16.)
        };
        let info = match &self.info {
            Load::Ready(w) => Some(w.clone()),
            _ => None,
        };
        let primary_info: AnyElement = match (&self.info, &info) {
            (_, Some(w)) => {
                let i = self.render_info(w, cx);
                let c = self.render_comments(w, cx);
                div().flex().flex_col().child(i).child(c).into_any_element()
            }
            (Load::Error(e), _) => div().py_4().text_color(p.text2).child(format!("Could not load video details: {e}")).into_any_element(),
            _ => spinner(cx).into_any_element(),
        };
        let queue_panel = self.render_queue_panel(cx);
        let related = info.as_ref().map(|w| self.render_related(w, cx));
        let secondary = div().flex().flex_col().w(if two_col { side_w } else { avail }).flex_none().children(queue_panel).children(related);
        let player_box = div().w(if ui.theater { vp.width } else { primary_w }).h(player_h).flex_none().when(!ui.theater, |d| d.rounded_xl().overflow_hidden()).child(crate::ui::slot::player_slot(cx));

        let body = if ui.theater {
            div()
                .flex()
                .flex_col()
                .child(div().w_full().bg(black()).flex().justify_center().child(player_box))
                .child(
                    div()
                        .flex()
                        .gap_6()
                        .px_6()
                        .pb_24()
                        .when(!two_col, |d| d.flex_col())
                        .child(div().w(primary_w).flex_none().child(primary_info))
                        .child(div().pt_3().child(secondary)),
                )
        } else {
            div()
                .flex()
                .justify_center()
                .gap_6()
                .px_6()
                .pt_6()
                .pb_24()
                .when(!two_col, |d| d.flex_col())
                .child(div().flex().flex_col().w(primary_w).flex_none().child(player_box).child(primary_info))
                .child(secondary)
        };
        page_scroll("watch-scroll", &self.scroll).child(body).into_any_element()
    }
}

/// Likes / dislikes a video locally and on the signed-in account.
fn set_rating(id: &str, r: i8, cx: &mut App) {
    store::set_rating(id, r);
    if crate::api::auth::signed_in() {
        let id = id.to_string();
        let kind = match r {
            1 => "like",
            -1 => "dislike",
            _ => "removelike",
        };
        cx.spawn(async move |cx| {
            if blocking(move || crate::api::auth::like(&id, kind)).await.is_err() {
                let _ = cx.update(|cx| toast(cx, "Couldn't update the rating on your YouTube account"));
            }
        })
        .detach();
    }
    cx.refresh_windows();
}
