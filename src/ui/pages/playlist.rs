use crate::api::innertube;
use crate::api::types::*;
use crate::player::controller::player;
use crate::store;
use crate::ui::app::{Route, confirm, navigate, prompt_text};
use crate::ui::cards::*;
use crate::ui::widgets::*;
use crate::ui::*;

enum Source {
    Remote(String),
    Local(i64),
}

pub struct PlaylistPage {
    source: Source,
    title: String,
    owner: Option<(String, Option<String>)>,
    count_text: Option<String>,
    description: Option<String>,
    videos: Load<Vec<VideoItem>>,
    continuation: Option<String>,
    loading_more: bool,
    scroll: ScrollHandle,
    protected: bool,
    info: Option<PlaylistItem>,
}

impl PlaylistPage {
    pub fn remote(id: String, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut s = Self::base(Source::Remote(id.clone()));
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::playlist(&id)).await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(p) => {
                        this.title = p.info.title.clone();
                        this.owner = p.info.channel_name.clone().map(|n| (n, p.info.channel_id.clone()));
                        this.count_text = p.info.video_count_text.clone();
                        this.description = p.description.clone();
                        this.videos = Load::Ready(p.videos);
                        this.continuation = p.continuation;
                        this.info = Some(p.info);
                    }
                    Err(e) => this.videos = Load::Error(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        s.title = "Playlist".into();
        s
    }

    pub fn local(id: i64, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        let mut s = Self::base(Source::Local(id));
        s.reload_local();
        s
    }

    fn base(source: Source) -> Self {
        Self {
            source,
            title: String::new(),
            owner: None,
            count_text: None,
            description: None,
            videos: Load::Loading,
            continuation: None,
            loading_more: false,
            scroll: ScrollHandle::new(),
            protected: false,
            info: None,
        }
    }

    fn reload_local(&mut self) {
        if let Source::Local(id) = self.source {
            let pl = store::playlists().into_iter().find(|p| p.id == id);
            self.title = pl.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| "Playlist".into());
            self.protected = pl.as_ref().map(|p| p.protected).unwrap_or(false);
            let items = store::playlist_items(id);
            self.count_text = Some(format!("{} videos", items.len()));
            self.owner = Some(("You".into(), None));
            self.videos = Load::Ready(items);
        }
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(tok) = self.continuation.clone() else {
            self.loading_more = false;
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::browse_continue(&tok)).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                match res {
                    Ok(p) => {
                        if let Some(v) = this.videos.ready_mut() {
                            v.extend(p.items.into_iter().filter_map(|i| match i {
                                Item::Video(v) => Some(v),
                                _ => None,
                            }));
                        }
                        this.continuation = p.continuation;
                    }
                    Err(_) => this.continuation = None,
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn playlist_param(&self) -> String {
        match &self.source {
            Source::Remote(id) => id.clone(),
            Source::Local(id) => format!("local:{id}"),
        }
    }

    fn play(&self, shuffle: bool, start_idx: usize, window: &mut Window, cx: &mut App) {
        let Some(videos) = self.videos.ready().cloned() else { return };
        if videos.is_empty() {
            return;
        }
        let mut q = videos;
        let mut idx = start_idx;
        if shuffle {
            use rand::seq::SliceRandom;
            q.shuffle(&mut rand::rng());
            idx = 0;
        }
        let first = q[idx].id.clone();
        player(cx).update(cx, |p, _| {
            p.queue = q;
            p.queue_pos = Some(idx);
        });
        let list = if shuffle { None } else { Some(self.playlist_param()) };
        navigate(Route::Watch { id: first, playlist: list, start: None }, window, cx);
    }
}

impl Render for PlaylistPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let has_more = self.continuation.is_some();
        maybe_load_more(&self.scroll, &mut self.loading_more, has_more, cx, |this, cx| this.load_more(cx));
        let videos = self.videos.ready().cloned().unwrap_or_default();
        let thumb = videos.first().map(|v| v.thumbnail.clone());
        let local_id = if let Source::Local(id) = self.source { Some(id) } else { None };
        let remote_info = self.info.clone();
        let saved = remote_info.as_ref().map(|i| store::is_playlist_saved(&i.id)).unwrap_or(false);
        let owner = self.owner.clone();
        let header = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_4()
            .w(px(360.))
            .p_6()
            .rounded_2xl()
            .bg(linear_gradient(180., linear_color_stop(p.hover, 0.), linear_color_stop(p.bg, 1.)))
            .child(
                div()
                    .w_full()
                    .h(px(176.))
                    .rounded_xl()
                    .overflow_hidden()
                    .bg(p.surface)
                    .when_some(thumb, |d, t| d.child(img(SharedString::from(t)).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(div().text_2xl().font_weight(FontWeight::BOLD).text_color(p.text).child(self.title.clone()))
            .when_some(owner, |d, (name, cid)| {
                d.child(
                    div()
                        .id("pl-owner")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.text)
                        .cursor_pointer()
                        .child(name)
                        .on_click(move |_, window, cx| {
                            if let Some(c) = cid.clone() {
                                navigate(Route::Channel(c), window, cx)
                            }
                        }),
                )
            })
            .when_some(self.count_text.clone(), |d, c| d.child(div().text_xs().text_color(p.text2).child(c)))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .when_some(remote_info, |d, info| {
                        d.child(
                            icon_button("pl-save", if saved { "bookmark-check" } else { "bookmark-plus" }, cx).bg(p.chip).on_click(move |_, _, cx| {
                                store::save_playlist(&info, !saved);
                                toast(cx, if saved { "Removed from library" } else { "Saved to library" });
                                cx.refresh_windows();
                            }),
                        )
                    })
                    .when_some(local_id.filter(|_| !self.protected), |d, id| {
                        d.child(icon_button("pl-rename", "pencil", cx).bg(p.chip).on_click(cx.listener(move |_, _, window, cx| {
                            prompt_text("Rename playlist", "Name", window, cx, move |name, _, cx| {
                                store::playlist_rename(id, &name);
                                cx.refresh_windows();
                            });
                        })))
                        .child(icon_button("pl-del", "trash-2", cx).bg(p.chip).on_click(move |_, window, cx| {
                            confirm("Delete playlist?", "This cannot be undone.", "Delete", window, cx, move |window, cx| {
                                store::playlist_delete(id);
                                navigate(Route::Library, window, cx);
                            });
                        }))
                    }),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        pill("pl-play", Some("play"), "Play all", cx)
                            .flex_1()
                            .justify_center()
                            .bg(p.chip_active_bg)
                            .text_color(p.chip_active_fg)
                            .on_click(cx.listener(|this, _, window, cx| this.play(false, 0, window, cx))),
                    )
                    .child(pill("pl-shuffle", Some("shuffle"), "Shuffle", cx).flex_1().justify_center().on_click(cx.listener(|this, _, window, cx| this.play(true, 0, window, cx)))),
            )
            .when_some(self.description.clone().filter(|d| !d.is_empty()), |d, desc| d.child(div().text_xs().text_color(p.text2).child(desc)));

        let list: AnyElement = match &self.videos {
            Load::Loading => spinner(cx).into_any_element(),
            Load::Error(e) => error_state(e, cx).into_any_element(),
            Load::Ready(v) if v.is_empty() => empty_state("list-video", "No videos in this playlist yet", "", cx).into_any_element(),
            Load::Ready(vs) => {
                let mut col = div().flex().flex_col().gap_1().flex_1().min_w_0();
                for (i, v) in vs.iter().enumerate() {
                    let vid = v.id.clone();
                    let row = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w_6().text_sm().text_color(p.text2).text_center().child((i + 1).to_string()))
                        .child(div().flex_1().min_w_0().child(compact_row(
                            v,
                            false,
                            Some({
                                let entity = cx.entity().downgrade();
                                Arc::new(move |window: &mut Window, cx: &mut App| {
                                    if let Some(e) = entity.upgrade() {
                                        let page = e.read(cx);
                                        // play from this index keeping the playlist as queue
                                        let videos = page.videos.ready().cloned().unwrap_or_default();
                                        let param = page.playlist_param();
                                        player(cx).update(cx, |p, _| {
                                            p.queue = videos;
                                            p.queue_pos = Some(i);
                                        });
                                        let id = vid.clone();
                                        navigate(Route::Watch { id, playlist: Some(param), start: None }, window, cx);
                                    }
                                })
                            }),
                            cx,
                        )))
                        .when_some(local_id, |d, lid| {
                            let (a, b, c) = (v.id.clone(), v.id.clone(), v.id.clone());
                            d.child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .child(icon_button(SharedString::from(format!("up-{i}")), "chevron-up", cx).size_7().on_click(cx.listener(move |this, _, _, cx| {
                                        store::playlist_move(lid, &a, -1);
                                        this.reload_local();
                                        cx.notify();
                                    })))
                                    .child(icon_button(SharedString::from(format!("dn-{i}")), "chevron-down", cx).size_7().on_click(cx.listener(move |this, _, _, cx| {
                                        store::playlist_move(lid, &b, 1);
                                        this.reload_local();
                                        cx.notify();
                                    }))),
                            )
                            .child(icon_button(SharedString::from(format!("rm-{i}")), "trash-2", cx).on_click(cx.listener(move |this, _, _, cx| {
                                store::playlist_remove(lid, &c);
                                this.reload_local();
                                cx.notify();
                            })))
                        });
                    col = col.child(row);
                }
                if self.loading_more {
                    col = col.child(spinner(cx));
                }
                col.into_any_element()
            }
        };
        page_scroll("pl-scroll", &self.scroll).child(div().flex().items_start().gap_6().px_6().pt_6().pb_24().child(header).child(list))
    }
}
