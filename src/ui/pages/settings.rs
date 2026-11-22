use crate::api::services::SB_CATEGORIES;
use crate::store::{self, Settings};
use crate::ui::widgets::*;
use crate::ui::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};

#[derive(Clone, Copy, PartialEq)]
enum Section {
    General,
    Player,
    SponsorBlock,
    Content,
    Subscriptions,
    Privacy,
    Network,
    Downloads,
    Data,
    About,
}

const SECTIONS: &[(Section, &str, &str)] = &[
    (Section::General, "General", "palette"),
    (Section::Player, "Player", "circle-play"),
    (Section::SponsorBlock, "SponsorBlock & DeArrow", "shield-check"),
    (Section::Content, "Content filters", "funnel"),
    (Section::Subscriptions, "Subscriptions", "tv"),
    (Section::Privacy, "Privacy", "lock"),
    (Section::Network, "Backend & network", "network"),
    (Section::Downloads, "Downloads", "download"),
    (Section::Data, "Import / export", "database"),
    (Section::About, "About", "info"),
];

pub struct SettingsPage {
    section: Section,
    scroll: ScrollHandle,
    inputs: Vec<(&'static str, Entity<InputState>)>,
    keyword: Entity<InputState>,
    ytdlp_version: Option<String>,
    _subs: Vec<Subscription>,
}

type Setter = fn(&mut Settings, String);

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = store::settings_snapshot();
        let fields: Vec<(&'static str, String, &'static str, Setter)> = vec![
            ("region", s.region.clone(), "US", |s, v| s.region = v.to_uppercase()),
            ("language", s.language.clone(), "en", |s, v| s.language = v),
            ("sb_server", s.sponsorblock_server.clone(), "https://sponsor.ajay.app", |s, v| s.sponsorblock_server = v),
            ("ytdlp", s.ytdlp_path.clone(), "yt-dlp (from PATH)", |s, v| s.ytdlp_path = v),
            ("proxy", s.proxy.clone(), "e.g. socks5://127.0.0.1:9050 or http://host:port", |s, v| s.proxy = v),
            ("external", s.external_player.clone(), "mpv (from PATH) or full path to vlc.exe", |s, v| s.external_player = v),
            ("download_dir", s.download_dir.clone(), "Download folder", |s, v| s.download_dir = v),
            ("captions_lang", s.captions_lang.clone(), "en", |s, v| s.captions_lang = v),
            ("sb_min", format!("{}", s.sb_min_duration), "0", |s, v| s.sb_min_duration = v.parse().unwrap_or(0.0)),
        ];
        let mut inputs = Vec::new();
        let mut subs = Vec::new();
        for (key, val, ph, setter) in fields {
            let st = cx.new(|cx| {
                let mut i = InputState::new(window, cx).placeholder(ph);
                i.set_value(val, window, cx);
                i
            });
            subs.push(cx.subscribe_in(&st, window, move |_, st, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    let v = st.read(cx).value().trim().to_string();
                    store::update_settings(|s| setter(s, v));
                    if matches!(ev, InputEvent::PressEnter { .. }) {
                        toast(cx, "Saved");
                    }
                }
            }));
            inputs.push((key, st));
        }
        let keyword = cx.new(|cx| InputState::new(window, cx).placeholder("Add a keyword and press Enter"));
        subs.push(cx.subscribe_in(&keyword, window, |_, st, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let v = st.read(cx).value().trim().to_string();
                if !v.is_empty() {
                    store::update_settings(|s| {
                        if !s.blocked_keywords.contains(&v) {
                            s.blocked_keywords.push(v)
                        }
                    });
                    st.update(cx, |i, cx| i.set_value("", window, cx));
                    cx.refresh_windows();
                }
            }
        }));
        let mut page = Self { section: Section::General, scroll: ScrollHandle::new(), inputs, keyword, ytdlp_version: None, _subs: subs };
        cx.spawn(async move |this, cx| {
            let v = blocking(|| {
                crate::api::streams::ytdlp_path().and_then(|p| {
                    let mut c = std::process::Command::new(p);
                    c.arg("--version");
                    crate::api::streams::hide_window(&mut c);
                    c.output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                })
            })
            .await;
            let _ = this.update(cx, |t, cx| {
                t.ytdlp_version = v;
                cx.notify();
            });
        })
        .detach();
        page.section = Section::General;
        page
    }

    fn input(&self, key: &str) -> Entity<InputState> {
        self.inputs.iter().find(|(k, _)| *k == key).map(|(_, e)| e.clone()).unwrap()
    }
}

/// Applies a changed setting to the running app (theme, palette, current playback).
fn after_change(_id: &str, window: &mut Window, cx: &mut App) {
    apply_theme(Some(window), cx);
    crate::player::controller::apply_settings(cx);
    cx.refresh_windows();
}

fn card(cx: &App) -> Div {
    let p = pal(cx);
    div().flex().flex_col().rounded_xl().bg(p.surface).px_5().py_2()
}

fn row(title: &str, desc: &str, control: impl IntoElement, cx: &App) -> Div {
    let p = pal(cx);
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .py_3()
        .border_b_1()
        .border_color(p.border)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_0p5()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(p.text).child(title.to_string()))
                .when(!desc.is_empty(), |d| d.child(div().text_xs().text_color(p.text2).child(desc.to_string()))),
        )
        .child(control)
}

fn switch(id: &'static str, on: bool, set: fn(&mut Settings, bool), cx: &App) -> impl IntoElement {
    let p = pal(cx);
    div()
        .id(id)
        .flex_none()
        .relative()
        .w(px(40.))
        .h(px(22.))
        .rounded_full()
        .cursor_pointer()
        .bg(if on { p.link } else { p.hover })
        .child(div().absolute().top(px(3.)).left(if on { px(21.) } else { px(3.) }).size(px(16.)).rounded_full().bg(white()))
        .on_click(move |_, window, cx| {
            store::update_settings(|s| set(s, !on));
            after_change(id, window, cx);
        })
}

fn choice<T: PartialEq + Clone + 'static>(id: &'static str, options: &[(T, &str)], cur: T, set: fn(&mut Settings, T), cx: &App) -> impl IntoElement {
    let mut d = div().flex().flex_wrap().gap_2().justify_end().max_w(px(520.));
    for (i, (v, label)) in options.iter().enumerate() {
        let v2 = v.clone();
        d = d.child(chip(SharedString::from(format!("{id}-{i}")), label.to_string(), *v == cur, cx).on_click(move |_, window, cx| {
            let v3 = v2.clone();
            store::update_settings(|s| set(s, v3));
            after_change(id, window, cx);
        }));
    }
    d
}

fn text_field(st: &Entity<InputState>, w: f32, cx: &App) -> impl IntoElement {
    let p = pal(cx);
    div().w(px(w)).px_3().py_1().rounded_lg().border_1().border_color(p.border).bg(p.bg).child(Input::new(st).appearance(false))
}

impl SettingsPage {
    fn general(&self, cx: &mut Context<Self>) -> Div {
        let s = store::settings_snapshot();
        let account = crate::api::auth::account().filter(|_| crate::api::auth::signed_in());
        let account_ctl = if account.is_some() {
            pill("acct-out", Some("log-out"), "Sign out", cx).on_click(|_, _, cx| crate::ui::app::sign_out(cx))
        } else {
            pill("acct-in", Some("circle-user"), "Sign in", cx).on_click(|_, window, cx| crate::ui::app::open_sign_in(window, cx))
        };
        let account_desc = account.map(|a| a.name).unwrap_or_else(|| "Personal recommendations, subscriptions, likes and history".into());
        card(cx)
            .child(row("YouTube account", &account_desc, account_ctl, cx))
            .child(row("Theme", "", choice("theme", &[("dark".to_string(), "Dark"), ("light".to_string(), "Light"), ("system".to_string(), "System")], s.theme.clone(), |s, v| s.theme = v, cx), cx))
            .child(row(
                "Accent color",
                "",
                choice(
                    "accent",
                    &[(0xff0033u32, "Red"), (0x3ea6ff, "Blue"), (0x2ba640, "Green"), (0xab47bc, "Purple"), (0xffa000, "Amber"), (0xff4081, "Pink")],
                    s.accent,
                    |s, v| s.accent = v,
                    cx,
                ),
                cx,
            ))
            .child(row("Start page", "", choice("start", &[("home".to_string(), "Home"), ("subscriptions".to_string(), "Subscriptions"), ("history".to_string(), "History")], s.start_page.clone(), |s, v| s.start_page = v, cx), cx))
            .child(row("Interface size", "", choice("scale", &[(0.9f32, "90%"), (1.0, "100%"), (1.1, "110%"), (1.25, "125%")], s.ui_scale, |s, v| s.ui_scale = v, cx), cx))
            .child(row("Density", "", choice("density", &[("compact".to_string(), "Compact"), ("default".to_string(), "Default"), ("comfortable".to_string(), "Comfortable")], s.density.clone(), |s, v| s.density = v, cx), cx))
            .child(row("Thumbnail corners", "", choice("radius", &[(0.0f32, "Square"), (6.0, "Small"), (12.0, "Rounded"), (18.0, "Large")], s.thumb_radius, |s, v| s.thumb_radius = v, cx), cx))
            .child(row("Grid columns", "", choice("cols", &[(0u32, "Auto"), (3, "3"), (4, "4"), (5, "5"), (6, "6")], s.grid_columns, |s, v| s.grid_columns = v, cx), cx))
            .child(row("Region", "e.g. US, IN", text_field(&self.input("region"), 120., cx), cx))
            .child(row("Language", "e.g. en, hi", text_field(&self.input("language"), 120., cx), cx))
    }

    fn player(&self, cx: &mut Context<Self>) -> Div {
        let s = store::settings_snapshot();
        card(cx)
            .child(row(
                "Default quality",
                "Maximum resolution to start videos at",
                choice("q", &[(2160u32, "4K"), (1440, "1440p"), (1080, "1080p"), (720, "720p"), (480, "480p"), (360, "360p")], s.default_quality, |s, v| s.default_quality = v, cx),
                cx,
            ))
            .child(row("Preferred codec", "", choice("codec", &[("vp9".to_string(), "VP9"), ("av1".to_string(), "AV1"), ("h264".to_string(), "H.264")], s.codec_pref.clone(), |s, v| s.codec_pref = v, cx), cx))
            .child(row("Default speed", "", choice("speed", &[(0.75f64, "0.75x"), (1.0, "1x"), (1.25, "1.25x"), (1.5, "1.5x"), (1.75, "1.75x"), (2.0, "2x")], s.default_speed, |s, v| s.default_speed = v, cx), cx))
            .child(row("Remember speed per channel", "", switch("rsp", s.remember_speed_per_channel, |s, v| s.remember_speed_per_channel = v, cx), cx))
            .child(row("Autoplay next video", "", switch("autoplay", s.autoplay_next, |s, v| s.autoplay_next = v, cx), cx))
            .child(row("Resume where you left off", "", switch("resume", s.resume_playback, |s, v| s.resume_playback = v, cx), cx))
            .child(row("Loop videos by default", "", switch("loop", s.loop_video, |s, v| s.loop_video = v, cx), cx))
            .child(row("Audio only mode", "", switch("ao", s.audio_only, |s, v| s.audio_only = v, cx), cx))
            .child(row("Hardware decoding", "Takes effect after restart", switch("hw", s.hwdec, |s, v| s.hwdec = v, cx), cx))
            .child(row("Captions on by default", "", switch("cc", s.captions_enabled, |s, v| s.captions_enabled = v, cx), cx))
            .child(row("Caption language", "", text_field(&self.input("captions_lang"), 120., cx), cx))
            .child(row("Caption size", "", choice("capsize", &[(30u32, "Small"), (40, "Medium"), (52, "Large"), (64, "Extra large")], s.caption_size, |s, v| s.caption_size = v, cx), cx))
            .child(row("Caption background", "", switch("capbg", s.caption_background, |s, v| s.caption_background = v, cx), cx))
            .child(row("Hide controls after", "", choice("hide", &[(1500u64, "1.5s"), (2500, "2.5s"), (4000, "4s"), (8000, "8s")], s.controls_hide_ms, |s, v| s.controls_hide_ms = v, cx), cx))
            .child(row("Seek bar preview thumbnails", "", switch("sbprev", s.show_storyboard, |s, v| s.show_storyboard = v, cx), cx))
            .child(row("Arrow key seek step", "", choice("step", &[(5.0f64, "5s"), (10.0, "10s"), (15.0, "15s"), (30.0, "30s")], s.seek_step, |s, v| s.seek_step = v, cx), cx))
            .child(row("External player", "Used by “Open in external player”", text_field(&self.input("external"), 320., cx), cx))

    }

    fn sponsorblock(&self, cx: &mut Context<Self>) -> Div {
        let p = pal(cx);
        let s = store::settings_snapshot();
        let mut c = card(cx)
            .child(row("Enable SponsorBlock", "", switch("sb", s.sponsorblock, |s, v| s.sponsorblock = v, cx), cx))
            .child(row("Show skip notifications", "With an “Unskip” button", switch("sbn", s.sb_notify, |s, v| s.sb_notify = v, cx), cx))
            .child(row("Minimum segment duration (seconds)", "", text_field(&self.input("sb_min"), 100., cx), cx))
            .child(row("Server", "", text_field(&self.input("sb_server"), 320., cx), cx));
        for (cat, name, color) in SB_CATEGORIES {
            let cur = s.sb_actions.get(*cat).cloned().unwrap_or_else(|| "off".into());
            let mut opts = div().flex().gap_2();
            for (v, l) in [("skip", "Auto skip"), ("ask", "Ask"), ("show", "Show in seek bar"), ("off", "Off")] {
                if *cat == "poi_highlight" && (v == "skip" || v == "ask") {
                    continue;
                }
                let cat2 = cat.to_string();
                opts = opts.child(chip(SharedString::from(format!("sb-{cat}-{v}")), l, cur == v, cx).on_click(move |_, _, cx| {
                    let c = cat2.clone();
                    store::update_settings(|s| {
                        s.sb_actions.insert(c, v.to_string());
                    });
                    cx.refresh_windows();
                }));
            }
            c = c.child(row(name, "", opts, cx).child(div().size_3().rounded_full().bg(rgb(*color))));
        }
        let _ = p;
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(c)
            .child(
                card(cx)
                    .child(row("DeArrow titles", "", switch("dat", s.dearrow_titles, |s, v| s.dearrow_titles = v, cx), cx))
                    .child(row("DeArrow thumbnails", "", switch("dath", s.dearrow_thumbnails, |s, v| s.dearrow_thumbnails = v, cx), cx))
                    .child(row("Return YouTube Dislike", "", switch("ryd", s.ryd, |s, v| s.ryd = v, cx), cx)),
            )
    }

    fn content(&self, cx: &mut Context<Self>) -> Div {
        let p = pal(cx);
        let s = store::settings_snapshot();
        let mut kw = div().flex().flex_wrap().gap_2();
        for k in s.blocked_keywords.iter() {
            let k2 = k.clone();
            kw = kw.child(
                div()
                    .id(SharedString::from(format!("kw-{k}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .bg(p.chip)
                    .text_sm()
                    .cursor_pointer()
                    .child(k.clone())
                    .child(icon("x").size_3().text_color(p.text2))
                    .on_click(move |_, _, cx| {
                        let k3 = k2.clone();
                        store::update_settings(|s| s.blocked_keywords.retain(|x| *x != k3));
                        cx.refresh_windows();
                    }),
            );
        }
        let mut chans = div().flex().flex_col();
        for (id, name) in s.blocked_channels.iter() {
            let id2 = id.clone();
            chans = chans.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .py_1()
                    .child(div().text_sm().child(name.clone()))
                    .child(pill(SharedString::from(format!("unb-{id}")), None, "Unblock", cx).on_click(move |_, _, cx| {
                        let i = id2.clone();
                        store::update_settings(|s| s.blocked_channels.retain(|(c, _)| *c != i));
                        cx.refresh_windows();
                    })),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                card(cx)
                    .child(row("Hide Shorts", "", switch("hs", s.hide_shorts, |s, v| s.hide_shorts = v, cx), cx))
                    .child(row("Hide live streams", "", switch("hl", s.hide_live, |s, v| s.hide_live = v, cx), cx))
                    .child(row("Hide upcoming / premieres", "", switch("hu", s.hide_upcoming, |s, v| s.hide_upcoming = v, cx), cx))
                    .child(row("Hide comments", "", switch("hc", s.hide_comments, |s, v| s.hide_comments = v, cx), cx))
                    .child(row("Hide related videos", "", switch("hr", s.hide_related, |s, v| s.hide_related = v, cx), cx))
                    .child(row("Hide description", "", switch("hd", s.hide_description, |s, v| s.hide_description = v, cx), cx))
                    .child(row("Hide view counts", "", switch("hv", s.hide_view_counts, |s, v| s.hide_view_counts = v, cx), cx))
                    .child(row("Hide channel avatars", "", switch("ha", s.hide_channel_avatars, |s, v| s.hide_channel_avatars = v, cx), cx))
                    .child(row("Hide chapters", "", switch("hch", s.hide_chapters, |s, v| s.hide_chapters = v, cx), cx))
                    .child(row("Hide Explore / trending", "", switch("he", s.hide_explore, |s, v| s.hide_explore = v, cx), cx))
                    .child(row("Hide watched videos in feeds", "", switch("hw2", s.hide_watched_in_feed, |s, v| s.hide_watched_in_feed = v, cx), cx)),
            )
            .child(
                card(cx)
                    .child(row("Blocked keywords", "", div().border_b_1().border_color(p.border).w(px(280.)).child(Input::new(&self.keyword).appearance(false)), cx))
                    .child(div().py_3().child(kw)),
            )
            .child(
                card(cx)
                    .child(row("Blocked channels", "", div(), cx))
                    .child(div().py_2().child(if s.blocked_channels.is_empty() { div().text_xs().text_color(p.text2).child("None").into_any_element() } else { chans.into_any_element() })),
            )
    }

    fn subscriptions(&self, cx: &mut Context<Self>) -> Div {
        let s = store::settings_snapshot();
        card(cx)
            .child(row(
                "Feed source",
                "",
                choice("fs", &[("full".to_string(), "Full"), ("rss".to_string(), "RSS")], s.feed_source.clone(), |s, v| s.feed_source = v, cx),
                cx,
            ))
            .child(row("Include Shorts in feed", "Extra request per channel", switch("fis", s.feed_include_shorts, |s, v| s.feed_include_shorts = v, cx), cx))
            .child(row("Include live streams in feed", "", switch("fil", s.feed_include_live, |s, v| s.feed_include_live = v, cx), cx))
    }

    fn privacy(&self, cx: &mut Context<Self>) -> Div {
        let s = store::settings_snapshot();
        card(cx)
            .child(row("Save watch history", "Stored only on this computer", switch("he2", s.history_enabled, |s, v| s.history_enabled = v, cx), cx))
            .child(row("Save search history", "", switch("se", s.search_history_enabled, |s, v| s.search_history_enabled = v, cx), cx))
            .child(row("Clear search history", "", pill("csh", Some("trash-2"), "Clear", cx).on_click(|_, _, cx| {
                store::search_history_clear();
                toast(cx, "Search history cleared");
            }), cx))
            .child(row("Clear watch history", "", pill("cwh", Some("trash-2"), "Clear", cx).on_click(|_, _, cx| {
                store::history_clear();
                cx.default_global::<crate::ui::cards::Watched>().0.clear();
                toast(cx, "Watch history cleared");
            }), cx))
    }

    fn network(&self, cx: &mut Context<Self>) -> Div {
        let s = store::settings_snapshot();
        card(cx)
            .child(row(
                "Stream backend",
                "",
                choice("sb2", &[("auto".to_string(), "Auto"), ("innertube".to_string(), "Built-in only"), ("ytdlp".to_string(), "yt-dlp only")], s.stream_backend.clone(), |s, v| s.stream_backend = v, cx),
                cx,
            ))
            .child(row(
                "yt-dlp path",
                &match &self.ytdlp_version {
                    Some(v) => format!("Detected yt-dlp {v}"),
                    None => "yt-dlp not found — install it for downloads in all qualities and the fallback extractor".into(),
                },
                text_field(&self.input("ytdlp"), 320., cx),
                cx,
            ))
            .child(row("Proxy", "Restart required", text_field(&self.input("proxy"), 320., cx), cx))
            .child(row("Update yt-dlp", "", pill("ytu", Some("refresh-cw"), "Update", cx).on_click(|_, _, cx| {
                cx.spawn(async move |cx| {
                    let out = blocking(|| {
                        crate::api::streams::ytdlp_path().map(|p| {
                            let mut c = std::process::Command::new(p);
                            c.arg("-U");
                            crate::api::streams::hide_window(&mut c);
                            c.output().map(|o| String::from_utf8_lossy(&o.stdout).lines().last().unwrap_or("").to_string()).unwrap_or_else(|e| e.to_string())
                        })
                    })
                    .await;
                    let _ = cx.update(|cx| toast(cx, out.unwrap_or_else(|| "yt-dlp not found".into())));
                })
                .detach();
            }), cx))
    }

    fn downloads(&self, cx: &mut Context<Self>) -> Div {
        let s = store::settings_snapshot();
        card(cx)
            .child(row("Download folder", "", text_field(&self.input("download_dir"), 360., cx), cx))
            .child(row(
                "Default format",
                "",
                choice("df", &crate::downloads::FORMATS.iter().map(|(k, l)| (k.to_string(), *l)).collect::<Vec<_>>(), s.download_format.clone(), |s, v| s.download_format = v, cx),
                cx,
            ))
    }

    fn data(&self, cx: &mut Context<Self>) -> Div {
        let p = pal(cx);
        let n_subs = store::subscriptions().len();
        let n_hist = store::progress_map().len();
        let n_pl = store::playlists().len();
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().text_sm().text_color(p.text2).child(format!("{n_subs} subscriptions • {n_hist} history entries • {n_pl} playlists • stored in {}", store::data_dir().display())))
            .child(
                card(cx)
                    .child(row(
                        "Import",
                        "Takeout, NewPipe, FreeTube, OPML or Stream Tube backup",
                        pill("imp", Some("upload"), "Choose file…", cx).on_click(|_, _, cx| {
                            let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Import".into()) });
                            cx.spawn(async move |cx| {
                                if let Ok(Ok(Some(paths))) = rx.await {
                                    for path in paths {
                                        let res = blocking(move || crate::import::import_file(&path)).await;
                                        let _ = cx.update(|cx| {
                                            match res {
                                                Ok(r) => toast(cx, r.to_string()),
                                                Err(e) => toast(cx, format!("Import failed: {e}")),
                                            }
                                            cx.set_global(crate::ui::cards::Watched(store::progress_map()));
                                            cx.refresh_windows();
                                        });
                                    }
                                }
                            })
                            .detach();
                        }),
                        cx,
                    ))
                    .child(row("Export full backup", "", export_button("exp-all", "streamtube-backup.json", ExportKind::Backup, cx), cx))
                    .child(row("Export subscriptions (NewPipe JSON)", "", export_button("exp-np", "subscriptions.json", ExportKind::Subs, cx), cx))
                    .child(row("Export subscriptions (YouTube CSV)", "", export_button("exp-csv", "subscriptions.csv", ExportKind::Subs, cx), cx))
                    .child(row("Export subscriptions (OPML)", "For RSS readers", export_button("exp-opml", "subscriptions.opml", ExportKind::Subs, cx), cx)),
            )
    }

    fn about(&self, cx: &mut Context<Self>) -> Div {
        let mpv = crate::player::mpv::available();
        card(cx)
            .child(row("Stream Tube", &format!("Version {}", env!("CARGO_PKG_VERSION")), div(), cx))
            .child(row("Video engine", if mpv { "libmpv" } else { "libmpv-2.dll not found" }, div(), cx))
            .child(row("yt-dlp", self.ytdlp_version.as_deref().unwrap_or("not installed"), div(), cx))

    }
}

#[derive(Clone, Copy)]
enum ExportKind {
    Backup,
    Subs,
}

fn export_button(id: &'static str, name: &'static str, kind: ExportKind, cx: &App) -> Stateful<Div> {
    pill(id, Some("download"), "Export…", cx).on_click(move |_, _, cx| {
        let dir = dirs::document_dir().unwrap_or_else(std::env::temp_dir);
        let rx = cx.prompt_for_new_path(&dir, Some(name));
        cx.spawn(async move |cx| {
            if let Ok(Ok(Some(path))) = rx.await {
                let p2 = path.clone();
                let res = blocking(move || match kind {
                    ExportKind::Backup => crate::import::export_backup(&p2),
                    ExportKind::Subs => crate::import::export_subscriptions(&p2),
                })
                .await;
                let _ = cx.update(|cx| match res {
                    Ok(_) => toast(cx, format!("Exported to {}", path.display())),
                    Err(e) => toast(cx, format!("Export failed: {e}")),
                });
            }
        })
        .detach();
    })
}

impl Render for SettingsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal(cx);
        let mut nav = div().flex().flex_col().gap_1().w(px(240.)).flex_none();
        for (sec, name, ic) in SECTIONS {
            let active = *sec == self.section;
            let sec = *sec;
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("sec-{name}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .h_10()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_sm()
                    .text_color(p.text)
                    .when(active, |d| d.bg(p.chip).font_weight(FontWeight::SEMIBOLD))
                    .hover(move |s| s.bg(p.hover))
                    .child(icon(ic).size_5().text_color(p.text))
                    .child(name.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = sec;
                        cx.notify();
                    })),
            );
        }
        let content = match self.section {
            Section::General => self.general(cx),
            Section::Player => self.player(cx),
            Section::SponsorBlock => self.sponsorblock(cx),
            Section::Content => self.content(cx),
            Section::Subscriptions => self.subscriptions(cx),
            Section::Privacy => self.privacy(cx),
            Section::Network => self.network(cx),
            Section::Downloads => self.downloads(cx),
            Section::Data => self.data(cx),
            Section::About => self.about(cx),
        };
        let title = SECTIONS.iter().find(|s| s.0 == self.section).map(|s| s.1).unwrap_or("");
        page_scroll("settings-scroll", &self.scroll).child(
            div().flex().justify_center().gap_8().px_6().pt_6().pb_24().child(nav).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .w(px(860.))
                    .child(section_title(title, cx).text_2xl())
                    .child(content),
            ),
        )
    }
}
