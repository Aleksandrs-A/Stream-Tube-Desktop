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
pub(crate) enum Token {
    Url(String),
    Time(f64),
    Hashtag(String),
    Handle(String),
}

pub(crate) fn tokenize(text: &str) -> Vec<(std::ops::Range<usize>, Token)> {
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

/// Escapes text so Markdown shows it literally.
fn md_escape(t: &str, out: &mut String) {
    for c in t.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
}

/// Plain text with links, timestamps, #hashtags and @handles as Markdown links
/// (app-internal schemes for the non-URL ones), line breaks kept.
fn to_markdown(text: &str) -> String {
    let mut md = String::with_capacity(text.len() + 64);
    let mut last = 0;
    for (r, tok) in tokenize(text) {
        md_escape(&text[last..r.start], &mut md);
        let label = &text[r.clone()];
        md.push('[');
        md_escape(label, &mut md);
        md.push_str("](");
        match tok {
            Token::Url(u) => {
                md.push('<');
                md.push_str(&u.replace(['<', '>', ' '], ""));
                md.push('>');
            }
            Token::Time(t) => md.push_str(&format!("st-time:{}", t as u64)),
            Token::Hashtag(h) => md.push_str(&format!("st-tag:{}", crate::api::http::urlencode(&h))),
            Token::Handle(h) => md.push_str(&format!("st-handle:{}", crate::api::http::urlencode(&h))),
        }
        md.push(')');
        last = r.end;
    }
    md_escape(&text[last..], &mut md);
    // single newlines are hard breaks, not spaces
    md.replace("\r\n", "\n").replace('\n', "  \n")
}

fn open_link(url: &str, window: &mut Window, cx: &mut App) {
    if let Some(t) = url.strip_prefix("st-time:") {
        let t: f64 = t.parse().unwrap_or(0.0);
        crate::player::controller::player(cx).update(cx, |p, cx| {
            p.seek_to(t);
            p.set_paused(false);
            cx.notify();
        });
    } else if let Some(h) = url.strip_prefix("st-tag:") {
        crate::ui::app::navigate(crate::ui::app::Route::Search(crate::api::http::urldecode(h)), window, cx);
    } else if let Some(h) = url.strip_prefix("st-handle:") {
        let url = format!("https://www.youtube.com/{}", crate::api::http::urldecode(h));
        cx.spawn(async move |cx| {
            if let Ok(crate::api::innertube::ResolvedUrl::Channel(id)) = blocking(move || crate::api::innertube::resolve_url(&url)).await {
                let _ = cx.update(|cx| {
                    if let Some(w) = cx.active_window() {
                        let _ = w.update(cx, |_, window, cx| crate::ui::app::navigate(crate::ui::app::Route::Channel(id), window, cx));
                    }
                });
            }
        })
        .detach();
    } else if let Some(r) = crate::api::innertube::parse_youtube_url(url) {
        let route = match r {
            crate::api::innertube::ResolvedUrl::Video(id) => crate::ui::app::Route::Watch { id, playlist: None, start: None },
            crate::api::innertube::ResolvedUrl::Channel(id) => crate::ui::app::Route::Channel(id),
            crate::api::innertube::ResolvedUrl::Playlist(id) => crate::ui::app::Route::Playlist(id),
        };
        crate::ui::app::navigate(route, window, cx);
    } else {
        let _ = open::that(url);
    }
}

/// Selectable text (drag to select, Ctrl+C) with clickable links, timestamps,
/// hashtags and handles. Right-click copies the whole text.
pub fn rich_text(id: impl Into<ElementId>, text: String, cx: &App) -> AnyElement {
    let p = pal(cx);
    let urls: Vec<String> = tokenize(&text).into_iter().filter_map(|(_, t)| if let Token::Url(u) = t { Some(u) } else { None }).collect();
    let view = gpui_kit::base::TextView::markdown(id, to_markdown(&text))
        .selectable(true)
        .on_link_click(|url, _, window, cx| open_link(url, window, cx));
    let copy_text = text.clone();
    let _ = p;
    div()
        .w_full()
        .child(view)
        .on_mouse_down(MouseButton::Right, move |ev: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let t = copy_text.clone();
            let mut items = vec![crate::ui::app::MenuItem::new("copy", "Copy all text", move |_, cx| crate::ui::copy_to_clipboard(t.clone(), cx))];
            for u in urls.iter().take(3) {
                let u2 = u.clone();
                let label = if u.chars().count() > 40 { format!("Copy link {}…", u.chars().take(40).collect::<String>()) } else { format!("Copy link {u}") };
                items.push(crate::ui::app::MenuItem::new("link", label, move |_, cx| crate::ui::copy_to_clipboard(u2.clone(), cx)));
            }
            crate::ui::app::open_menu(ev.position, items, window, cx);
        })
        .into_any_element()
}

/// First timestamp ("1:23", "1:02:03") mentioned in a text, in seconds.
pub fn first_timestamp(text: &str) -> Option<f64> {
    tokenize(text).into_iter().find_map(|(_, t)| if let Token::Time(s) = t { Some(s) } else { None })
}
