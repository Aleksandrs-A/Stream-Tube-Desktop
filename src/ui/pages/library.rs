use crate::store;
use crate::ui::app::{Route, confirm, navigate, prompt_text};
use crate::ui::cards::*;
use crate::ui::widgets::*;
use crate::ui::*;
use gpui_kit::component::input::{Input, InputState};

pub struct HistoryPage {
    filter: Entity<InputState>,
    scroll: ScrollHandle,
    limit: usize,
}

impl HistoryPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search watch history"));
        cx.observe(&filter, |_, _, cx| cx.notify()).detach();
        Self { filter, scroll: ScrollHandle::new(), limit: 100 }
    }
}

impl Render for HistoryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let s = store::settings_snapshot();
        if near_bottom(&self.scroll, 800.0) {
            self.limit += 100;
        }
        let q = self.filter.read(cx).value().to_string();
        let entries = store::history_list(&q, self.limit);
        let width = content_width(cx).min(px(1100.));
        let mut list = div().flex().flex_col().gap_4().flex_1().min_w_0();
        let mut cur_day = String::new();
        for h in &entries {
            let day = chrono::DateTime::from_timestamp(h.watched_at, 0)
                .map(|d| d.with_timezone(&chrono::Local).format("%A, %B %-d, %Y").to_string())
                .unwrap_or_default();
            if day != cur_day {
                list = list.child(div().pt_4().text_lg().font_weight(FontWeight::BOLD).text_color(p.text).child(day.clone()));
                cur_day = day;
            }
            let id = h.video.id.clone();
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(video_row(&h.video, px(246.), cx)))
                    .child(icon_button(SharedString::from(format!("hx-{}", h.video.id)), "x", cx).on_click(move |_, _, cx| {
                        store::history_remove(&id);
                        cx.default_global::<Watched>().0.remove(&id);
                        cx.refresh_windows();
                    })),
            );
        }
        if entries.is_empty() {
            list = list.child(empty_state("history", if q.is_empty() { "Your watch history is empty" } else { "No matches" }, "", cx));
        }
        let side = div()
            .flex()
            .flex_col()
            .gap_4()
            .w(px(320.))
            .flex_none()
            .child(div().flex().items_center().gap_2().border_b_1().border_color(p.border).pb_1().child(icon("search").size_4().text_color(p.text2)).child(Input::new(&self.filter).appearance(false).cleanable(true)))
            .child(
                pill("h-clear", Some("trash-2"), "Clear all watch history", cx).on_click(|_, window, cx| {
                    confirm("Clear watch history?", "All watch history and resume positions will be deleted.", "Clear", window, cx, |_, cx| {
                        store::history_clear();
                        cx.default_global::<Watched>().0.clear();
                        cx.refresh_windows();
                    });
                }),
            )
            .child(
                pill("h-pause", Some(if s.history_enabled { "pause" } else { "play" }), if s.history_enabled { "Pause watch history" } else { "Turn on watch history" }, cx).on_click(|_, _, cx| {
                    store::update_settings(|s| s.history_enabled ^= true);
                    cx.refresh_windows();
                }),
            )
            .child(
                pill("h-search", Some("search-x"), "Clear search history", cx).on_click(|_, _, cx| {
                    store::search_history_clear();
                    toast(cx, "Search history cleared");
                }),
            );
        page_scroll("hist-scroll", &self.scroll).child(
            div().flex().justify_center().px_6().pt_6().pb_24().child(
                div()
                    .w(width)
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(section_title("Watch history", cx).text_3xl())
                    .child(div().flex().gap_8().child(list).child(side)),
            ),
        )
    }
}

pub struct LibraryPage {
    scroll: ScrollHandle,
}

impl LibraryPage {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self { scroll: ScrollHandle::new() }
    }
}

impl Render for LibraryPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let width = content_width(cx);
        let (_, w) = grid_columns(width, 260.0);
        let pls = store::playlists();
        let mut cards: Vec<AnyElement> = pls
            .iter()
            .map(|pl| {
                let id = pl.id;
                div()
                    .id(SharedString::from(format!("lp-{id}")))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w(w)
                    .cursor_pointer()
                    .on_click(move |_, window, cx| navigate(Route::LocalPlaylist(id), window, cx))
                    .child(playlist_thumb(pl.thumbnail.clone(), Some(format!("{} videos", pl.count)), w, w * (9. / 16.), cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(pl.protected, |d| d.child(icon(if pl.name == "Watch later" { "clock" } else { "heart" }).size_4().text_color(p.text2)))
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(pl.name.clone())),
                    )
                    .child(div().text_xs().text_color(p.text2).child("Local • Private"))
                    .into_any_element()
            })
            .collect();
        cards.push(
            div()
                .id("lp-new")
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .w(w)
                .h(w * (9. / 16.))
                .rounded_xl()
                .border_2()
                .border_dashed()
                .border_color(p.border)
                .cursor_pointer()
                .hover(move |s| s.bg(p.surface))
                .child(icon("plus").size_8().text_color(p.text2))
                .child(div().text_sm().text_color(p.text2).child("New playlist"))
                .on_click(|_, window, cx| {
                    prompt_text("New playlist", "Playlist name", window, cx, |name, _, cx| {
                        store::playlist_create(&name);
                        cx.refresh_windows();
                    });
                })
                .into_any_element(),
        );
        let saved = store::saved_playlists();
        let history: Vec<_> = store::history_list("", 12).into_iter().map(|h| h.video).collect();
        let (_, hw) = grid_columns(width, 220.0);
        page_scroll("lib-scroll", &self.scroll).child(
            div()
                .flex()
                .flex_col()
                .gap_10()
                .px_6()
                .pt_6()
                .pb_24()
                .when(!history.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(section_title("History", cx))
                                    .child(pill("lib-hist", None, "View all", cx).on_click(|_, window, cx| navigate(Route::History, window, cx))),
                            )
                            .child(div().flex().gap_4().overflow_hidden().children(history.iter().take(6).map(|v| video_card(v, hw, true, cx)))),
                    )
                })
                .child(div().flex().flex_col().gap_4().child(section_title("Your playlists", cx)).child(grid(cards, width)))
                .when(!saved.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(section_title("Saved playlists", cx))
                            .child(grid(saved.iter().map(|pl| playlist_card(pl, w, cx)).collect(), width)),
                    )
                }),
        )
    }
}
