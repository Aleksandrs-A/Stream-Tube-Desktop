use crate::api::innertube::{self, EXPLORE};
use crate::api::types::*;
use crate::store;
use crate::ui::cards::*;
use crate::ui::widgets::*;
use crate::ui::*;

/// Marks continuations of the signed-in home feed (they need the TV client).
const ACCOUNT: &str = "acct:";

pub struct HomePage {
    chip: usize,
    items: Load<Vec<Item>>,
    continuation: Option<String>,
    loading_more: bool,
    scroll: ScrollHandle,
    serial: u64,
}

impl HomePage {
    pub fn new(chip: usize, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut s = Self { chip, items: Load::Loading, continuation: None, loading_more: false, scroll: ScrollHandle::new(), serial: 0 };
        s.load(cx);
        s
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.items = Load::Loading;
        self.continuation = None;
        self.serial += 1;
        let serial = self.serial;
        let chip = self.chip;
        cx.spawn(async move |this, cx| {
            let res = blocking(move || -> anyhow::Result<(Vec<Item>, Option<String>)> {
                if chip == 0 && crate::api::auth::signed_in() {
                    // YouTube's own recommendations for the account
                    if let Ok(p) = crate::api::auth::home() {
                        if !p.items.is_empty() {
                            return Ok((p.items, p.continuation.map(|c| format!("{ACCOUNT}{c}"))));
                        }
                    }
                }
                if chip == 0 {
                    let subs = store::subscriptions();
                    if !subs.is_empty() {
                        let (mut feed, oldest) = crate::feed::cached_feed(None);
                        let stale = oldest.map(|o| crate::api::json::now() - o > 3600).unwrap_or(true);
                        if feed.is_empty() || stale {
                            feed = crate::feed::refresh(None, |_, _| {}).0;
                        }
                        // mix in some popular music & gaming for variety
                        let mut items: Vec<Item> = feed.into_iter().take(80).map(Item::Video).collect();
                        if let Ok(p) = innertube::explore(EXPLORE[0].1) {
                            let extra: Vec<Item> = p.items.into_iter().filter(|i| matches!(i, Item::Video(_))).take(12).collect();
                            let mut out = Vec::new();
                            let mut ex = extra.into_iter();
                            for (i, it) in items.drain(..).enumerate() {
                                out.push(it);
                                if i % 8 == 7 {
                                    if let Some(e) = ex.next() {
                                        out.push(e);
                                    }
                                }
                            }
                            items = out;
                        }
                        return Ok((items, None));
                    }
                    // no subscriptions: popular from several categories, interleaved
                    let lists: Vec<Vec<Item>> = std::thread::scope(|s| {
                        let hs: Vec<_> = EXPLORE
                            .iter()
                            .take(4)
                            .map(|(_, id)| s.spawn(move || innertube::explore(id).map(|p| p.items).unwrap_or_default()))
                            .collect();
                        hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
                    });
                    let mut out = Vec::new();
                    let mut iters: Vec<_> = lists.into_iter().map(|l| l.into_iter().filter(|i| matches!(i, Item::Video(_)))).collect();
                    let mut seen = std::collections::HashSet::new();
                    loop {
                        let mut any = false;
                        for it in iters.iter_mut() {
                            if let Some(x) = it.next() {
                                any = true;
                                if let Item::Video(v) = &x {
                                    if seen.insert(v.id.clone()) {
                                        out.push(x);
                                    }
                                }
                            }
                        }
                        if !any {
                            break;
                        }
                    }
                    Ok((out, None))
                } else {
                    let p = innertube::explore(EXPLORE[chip - 1].1)?;
                    Ok((p.items, p.continuation))
                }
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                if this.serial != serial {
                    return;
                }
                match res {
                    Ok((items, cont)) => {
                        this.items = Load::Ready(items);
                        this.continuation = cont;
                    }
                    Err(e) => this.items = Load::Error(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(tok) = self.continuation.clone() else {
            self.loading_more = false;
            return;
        };
        cx.spawn(async move |this, cx| {
            let res = blocking(move || match tok.strip_prefix(ACCOUNT) {
                Some(t) => crate::api::auth::browse_continue(t).map(|mut p| {
                    p.continuation = p.continuation.map(|c| format!("{ACCOUNT}{c}"));
                    p
                }),
                None => innertube::browse_continue(&tok),
            })
            .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                if let Ok(p) = res {
                    if let Some(items) = this.items.ready_mut() {
                        items.extend(p.items);
                    }
                    this.continuation = p.continuation;
                } else {
                    this.continuation = None;
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for HomePage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let s = store::settings_snapshot();
        let has_more = self.continuation.is_some();
        maybe_load_more(&self.scroll, &mut self.loading_more, has_more, cx, |this, cx| this.load_more(cx));
        let width = content_width(cx);
        let (_, card_w) = grid_columns(width, 300.0);
        let mut chips = div().flex().gap_3().py_3().px_6().bg(p.bg).child(
            chip("chip-all", if store::subscriptions().is_empty() && !crate::api::auth::signed_in() { "Popular" } else { "All" }, self.chip == 0, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.chip = 0;
                    this.load(cx);
                })),
        );
        if !s.hide_explore {
            for (i, (name, _)) in EXPLORE.iter().enumerate() {
                chips = chips.child(chip(SharedString::from(format!("chip-{i}")), *name, self.chip == i + 1, cx).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.chip = i + 1;
                        this.load(cx);
                    },
                )));
            }
        }
        let body: AnyElement = match &self.items {
            Load::Loading => spinner(cx).into_any_element(),
            Load::Error(e) => error_state(e, cx).into_any_element(),
            Load::Ready(items) => {
                let items: Vec<Item> = items
                    .iter()
                    .filter(|i| match i {
                        Item::Video(v) => !store::is_filtered(v, &s) && !(s.hide_watched_in_feed && is_watched(&v.id, cx)),
                        _ => true,
                    })
                    .cloned()
                    .collect();
                if items.is_empty() {
                    empty_state("tv", "Nothing here yet", "", cx).into_any_element()
                } else {
                    let (videos, shorts): (Vec<Item>, Vec<Item>) =
                        items.into_iter().partition(|i| !matches!(i, Item::Video(v) if v.is_short));
                    let mut col = div().flex().flex_col().gap_8();
                    let cards: Vec<AnyElement> = videos.iter().map(|i| grid_item(i, card_w, true, cx)).collect();
                    let split = cards.len().min(12);
                    let mut cards = cards.into_iter();
                    col = col.child(grid(cards.by_ref().take(split).collect(), width));
                    if !shorts.is_empty() && !s.hide_shorts {
                        col = col.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_4()
                                .child(div().flex().items_center().gap_2().child(icon("zap").text_color(p.accent)).child(section_title("Shorts", cx)))
                                .child(div().flex().gap_4().overflow_hidden().children(shorts.iter().take(8).filter_map(|i| i.as_video()).map(|v| shorts_card(v, px(200.), cx)))),
                        );
                    }
                    col = col.child(grid(cards.collect(), width));
                    if self.loading_more {
                        col = col.child(spinner(cx));
                    }
                    col.into_any_element()
                }
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(chips.overflow_hidden())
            .child(page_scroll("home-scroll", &self.scroll).child(div().px_6().pt_2().pb_24().child(body)))
    }
}
