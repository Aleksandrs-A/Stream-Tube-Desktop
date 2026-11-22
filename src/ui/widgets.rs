//! Shared page helpers.
use super::*;

pub enum Load<T> {
    Loading,
    Ready(T),
    Error(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }
    pub fn ready_mut(&mut self) -> Option<&mut T> {
        match self {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }
}

/// Vertical scroll container used by every page.
pub fn page_scroll(id: &'static str, handle: &ScrollHandle) -> Stateful<Div> {
    div().id(id).size_full().overflow_y_scroll().track_scroll(handle)
}

/// True when the scroll position is within `margin` of the bottom.
pub fn near_bottom(handle: &ScrollHandle, margin: f32) -> bool {
    let off: f32 = (-handle.offset().y).into();
    let max: f32 = handle.max_offset().y.into();
    max > 0.0 && off >= max - margin
}

pub fn content_width(cx: &App) -> Pixels {
    let w = cx.global::<crate::ui::app::UiState>().content_width;
    if w <= px(100.) { px(1000.) } else { w }
}

pub fn section_title(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = pal(cx);
    div().text_xl().font_weight(FontWeight::BOLD).text_color(p.text).child(text.into())
}

/// Sets up a "load more" trigger: call from render; spawns `f` when near the bottom.
pub fn maybe_load_more<T: 'static>(
    handle: &ScrollHandle,
    loading: &mut bool,
    has_more: bool,
    cx: &mut Context<T>,
    f: impl FnOnce(&mut T, &mut Context<T>) + 'static,
) {
    if has_more && !*loading && near_bottom(handle, 900.0) {
        *loading = true;
        cx.defer_in_entity(f);
    }
}

pub trait DeferExt<T> {
    fn defer_in_entity(&mut self, f: impl FnOnce(&mut T, &mut Context<T>) + 'static);
}

impl<T: 'static> DeferExt<T> for Context<'_, T> {
    fn defer_in_entity(&mut self, f: impl FnOnce(&mut T, &mut Context<T>) + 'static) {
        let weak = self.entity().downgrade();
        self.defer(move |cx| {
            if let Some(e) = weak.upgrade() {
                e.update(cx, f);
            }
        });
    }
}

// ───────────────────────── rich text (links, timestamps, hashtags) ─────────────────────────

#[derive(Clone)]
enum Token {
    Url(String),
    Time(f64),
    Hashtag(String),
    Handle(String),
}

fn tokenize(text: &str) -> Vec<(std::ops::Range<usize>, Token)> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let at_word_start = i == 0 || !text[..i].chars().last().map(|c| c.is_alphanumeric()).unwrap_or(false);
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest.find(|c: char| c.is_whitespace() || c == ')' || c == '"').unwrap_or(rest.len());
            let url = rest[..end].trim_end_matches(['.', ',', '!', '?']).to_string();
            out.push((i..i + url.len(), Token::Url(url.clone())));
            i += url.len().max(1);
            continue;
        }
        if at_word_start && bytes[i].is_ascii_digit() {
            let end = rest.find(|c: char| !(c.is_ascii_digit() || c == ':')).unwrap_or(rest.len());
            let cand = &rest[..end];
            if cand.contains(':') && !cand.ends_with(':') && cand.split(':').count() <= 3 && cand.split(':').skip(1).all(|p| p.len() == 2) {
                if let Some(secs) = crate::api::json::parse_duration(cand) {
                    out.push((i..i + end, Token::Time(secs as f64)));
                    i += end;
                    continue;
                }
            }
        }
        if at_word_start && (bytes[i] == b'#' || bytes[i] == b'@') && rest.len() > 1 {
            let end = rest[1..]
                .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
                .map(|e| e + 1)
                .unwrap_or(rest.len());
            let word = rest[..end].trim_end_matches('.');
            if word.len() > 1 {
                let tok = if bytes[i] == b'#' { Token::Hashtag(word.to_string()) } else { Token::Handle(word.to_string()) };
                out.push((i..i + word.len(), tok));
                i += word.len();
                continue;
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

pub fn rich_text(id: impl Into<ElementId>, text: String, cx: &App) -> AnyElement {
    let p = pal(cx);
    let toks = tokenize(&text);
    let highlights: Vec<_> = toks
        .iter()
        .map(|(r, _)| (r.clone(), HighlightStyle { color: Some(p.link), ..Default::default() }))
        .collect();
    let ranges: Vec<_> = toks.iter().map(|(r, _)| r.clone()).collect();
    let tokens: Vec<Token> = toks.into_iter().map(|(_, t)| t).collect();
    InteractiveText::new(id, StyledText::new(text).with_highlights(highlights))
        .on_click(ranges, move |ix, window, cx| match tokens[ix].clone() {
            Token::Url(u) => {
                if let Some(r) = crate::api::innertube::parse_youtube_url(&u) {
                    let route = match r {
                        crate::api::innertube::ResolvedUrl::Video(id) => crate::ui::app::Route::Watch { id, playlist: None, start: None },
                        crate::api::innertube::ResolvedUrl::Channel(id) => crate::ui::app::Route::Channel(id),
                        crate::api::innertube::ResolvedUrl::Playlist(id) => crate::ui::app::Route::Playlist(id),
                    };
                    crate::ui::app::navigate(route, window, cx);
                } else {
                    let _ = open::that(u);
                }
            }
            Token::Time(t) => {
                let pl = crate::player::controller::player(cx);
                pl.update(cx, |p, cx| {
                    p.seek_to(t);
                    p.set_paused(false);
                    cx.notify();
                });
            }
            Token::Hashtag(h) => crate::ui::app::navigate(crate::ui::app::Route::Search(h), window, cx),
            Token::Handle(h) => {
                let url = format!("https://www.youtube.com/{h}");
                cx.spawn(async move |cx| {
                    if let Ok(crate::api::innertube::ResolvedUrl::Channel(id)) =
                        blocking(move || crate::api::innertube::resolve_url(&url)).await
                    {
                        let _ = cx.update(|cx| {
                            if let Some(w) = cx.active_window() {
                                let _ = w.update(cx, |_, window, cx| crate::ui::app::navigate(crate::ui::app::Route::Channel(id), window, cx));
                            }
                        });
                    }
                })
                .detach();
            }
        })
        .into_any_element()
}
