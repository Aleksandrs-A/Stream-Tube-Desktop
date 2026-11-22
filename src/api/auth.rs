//! Google account sign-in using YouTube's TV device flow (the same method SmartTube
//! uses): the app shows a code, the user enters it at google.com/device, and the
//! resulting token authorizes the TV Innertube client for the personal home feed,
//! subscriptions, likes and watch history.
use super::http;
use super::innertube::collect_items;
use super::types::*;
use anyhow::{Result, anyhow};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// Public OAuth client of the YouTube TV app.
const CLIENT_ID: &str = "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "SboVhoG9s0rNafixCSGGKXAT";
const SCOPE: &str = "http://gdata.youtube.com https://www.googleapis.com/auth/youtube-paid-content";
const TV_VERSION: &str = "7.20251120.13.00";
const TV_UA: &str = "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub handle: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Stored {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
    account: Option<Account>,
}

#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval: u64,
    pub expires_in: u64,
}

fn state() -> &'static Mutex<Option<Stored>> {
    static S: std::sync::OnceLock<Mutex<Option<Stored>>> = std::sync::OnceLock::new();
    S.get_or_init(|| Mutex::new(crate::store::kv_get("auth").and_then(|s| serde_json::from_str(&s).ok())))
}

fn persist(s: &Option<Stored>) {
    match s {
        Some(v) => crate::store::kv_set("auth", &serde_json::to_string(v).unwrap_or_default()),
        None => crate::store::kv_set("auth", ""),
    }
}

pub fn signed_in() -> bool {
    state().lock().as_ref().map(|s| !s.refresh_token.is_empty()).unwrap_or(false)
}

pub fn account() -> Option<Account> {
    state().lock().as_ref().and_then(|s| s.account.clone())
}

pub fn sign_out() {
    let mut s = state().lock();
    *s = None;
    persist(&s);
}

fn oauth_post(url: &str, body: Value) -> Result<(u16, Value)> {
    http::post_json_status(url, &body, &[("User-Agent", TV_UA)])
}

/// Step 1: get a code for the user to enter at google.com/device.
pub fn start_login() -> Result<DeviceCode> {
    let device_id: String = {
        use rand::Rng;
        let mut r = rand::rng();
        (0..32).map(|_| format!("{:x}", r.random_range(0..16u8))).collect()
    };
    let (_, v) = oauth_post(
        "https://www.youtube.com/o/oauth2/device/code",
        json!({ "client_id": CLIENT_ID, "scope": SCOPE, "device_id": device_id, "device_model": "ytlr::" }),
    )?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(DeviceCode {
        device_code: s("device_code").ok_or_else(|| anyhow!("no device code"))?,
        user_code: s("user_code").ok_or_else(|| anyhow!("no user code"))?,
        verification_url: s("verification_url").unwrap_or_else(|| "https://www.google.com/device".into()),
        interval: v.get("interval").and_then(Value::as_u64).unwrap_or(5),
        expires_in: v.get("expires_in").and_then(Value::as_u64).unwrap_or(1800),
    })
}

pub enum PollResult {
    Pending,
    SlowDown,
    Done(Account),
}

/// Step 2: poll until the user has approved the code.
pub fn poll_login(dc: &DeviceCode) -> Result<PollResult> {
    let (_, v) = oauth_post(
        "https://www.youtube.com/o/oauth2/token",
        json!({
            "client_id": CLIENT_ID, "client_secret": CLIENT_SECRET, "code": dc.device_code,
            "grant_type": "http://oauth.net/grant_type/device/1.0"
        }),
    )?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return match err {
            "authorization_pending" => Ok(PollResult::Pending),
            "slow_down" => Ok(PollResult::SlowDown),
            "expired_token" => Err(anyhow!("The code expired. Try again.")),
            "access_denied" => Err(anyhow!("Sign-in was cancelled.")),
            e => Err(anyhow!("Sign-in failed: {e}")),
        };
    }
    let access = v.get("access_token").and_then(Value::as_str).ok_or_else(|| anyhow!("no access token"))?;
    let refresh = v.get("refresh_token").and_then(Value::as_str).unwrap_or("");
    let expires = v.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
    {
        let mut s = state().lock();
        *s = Some(Stored {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_at: crate::api::json::now() + expires - 60,
            account: None,
        });
        persist(&s);
    }
    let acc = fetch_account().unwrap_or_else(|_| Account { name: "YouTube account".into(), ..Default::default() });
    {
        let mut s = state().lock();
        if let Some(st) = s.as_mut() {
            st.account = Some(acc.clone());
        }
        persist(&s);
    }
    Ok(PollResult::Done(acc))
}

/// A valid access token, refreshed when needed.
pub fn access_token() -> Option<String> {
    let cur = state().lock().clone()?;
    if cur.expires_at > crate::api::json::now() && !cur.access_token.is_empty() {
        return Some(cur.access_token);
    }
    let (status, v) = oauth_post(
        "https://www.youtube.com/o/oauth2/token",
        json!({
            "client_id": CLIENT_ID, "client_secret": CLIENT_SECRET,
            "refresh_token": cur.refresh_token, "grant_type": "refresh_token"
        }),
    )
    .ok()?;
    if let Some(tok) = v.get("access_token").and_then(Value::as_str) {
        let exp = v.get("expires_in").and_then(Value::as_i64).unwrap_or(3600);
        let mut s = state().lock();
        if let Some(st) = s.as_mut() {
            st.access_token = tok.to_string();
            st.expires_at = crate::api::json::now() + exp - 60;
        }
        persist(&s);
        return Some(tok.to_string());
    }
    // the grant was revoked from the Google account page
    if status == 400 || status == 401 {
        sign_out();
    }
    None
}

fn tv_context() -> Value {
    let s = crate::store::settings_snapshot();
    json!({ "client": { "clientName": "TVHTML5", "clientVersion": TV_VERSION, "hl": s.language, "gl": s.region } })
}

/// Authorized call to the TV Innertube client.
pub fn tv_call(endpoint: &str, mut body: Value) -> Result<Value> {
    let token = access_token().ok_or_else(|| anyhow!("not signed in"))?;
    body["context"] = tv_context();
    let auth = format!("Bearer {token}");
    http::post_json(
        &format!("https://www.youtube.com/youtubei/v1/{endpoint}?prettyPrint=false"),
        &body,
        &[
            ("User-Agent", TV_UA),
            ("Authorization", &auth),
            ("X-YouTube-Client-Name", "7"),
            ("X-YouTube-Client-Version", TV_VERSION),
            ("Origin", "https://www.youtube.com"),
        ],
    )
}

fn fetch_account() -> Result<Account> {
    let r = tv_call("account/accounts_list", json!({ "accountReadMask": { "returnOwner": true } }))?;
    let item = crate::api::json::find(&r, "accountItem").or_else(|| crate::api::json::find(&r, "accountItemRenderer")).unwrap_or(&r);
    let name = item
        .get("accountName")
        .map(crate::api::json::text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "YouTube account".into());
    let handle = item.get("channelHandle").map(crate::api::json::text).filter(|s| !s.is_empty());
    let avatar = item.get("accountPhoto").and_then(crate::api::json::best_thumb);
    Ok(Account { name, handle, avatar })
}

/// Videos only, deduplicated, in feed order.
fn videos(page: Page) -> Page {
    let mut seen = std::collections::HashSet::new();
    Page {
        items: page
            .items
            .into_iter()
            .filter_map(|i| match i {
                Item::Video(mut v) if seen.insert(v.id.clone()) => {
                    if v.published_ts.is_none() {
                        v.published_ts = v.published_text.as_deref().and_then(crate::api::json::parse_relative_time);
                    }
                    Some(Item::Video(v))
                }
                _ => None,
            })
            .collect(),
        continuation: page.continuation,
    }
}

/// The signed-in user's personal home feed (YouTube's own recommendations).
pub fn home() -> Result<Page> {
    let r = tv_call("browse", json!({ "browseId": "FEwhat_to_watch" }))?;
    Ok(videos(collect_items(&r)))
}

/// The signed-in user's subscriptions feed.
pub fn subscriptions_feed() -> Result<Page> {
    let r = tv_call("browse", json!({ "browseId": "FEsubscriptions" }))?;
    Ok(videos(collect_items(&r)))
}

pub fn browse_continue(token: &str) -> Result<Page> {
    let r = tv_call("browse", json!({ "continuation": token }))?;
    Ok(videos(collect_items(&r)))
}

/// Channels the account is subscribed to (the per-channel tabs of the TV subscriptions page).
pub fn subscribed_channels() -> Result<Vec<ChannelItem>> {
    let r = tv_call("browse", json!({ "browseId": "FEsubscriptions" }))?;
    let mut tabs = Vec::new();
    crate::api::json::find_all(&r, "tabRenderer", &mut tabs);
    let mut out: Vec<ChannelItem> = Vec::new();
    for t in tabs {
        let Some(id) = crate::api::json::find(t, "browseEndpoint").and_then(|b| b.get("browseId")).and_then(Value::as_str) else { continue };
        if !id.starts_with("UC") || out.iter().any(|c| c.id == id) {
            continue;
        }
        let name = t.get("title").map(crate::api::json::text).unwrap_or_default();
        let avatar = t.get("thumbnail").and_then(crate::api::json::best_thumb);
        out.push(ChannelItem { id: id.to_string(), name, avatar, ..Default::default() });
    }
    Ok(out)
}

/// The account's subscriptions feed, following a few continuations.
pub fn subscriptions_videos(max_pages: usize) -> Result<Vec<VideoItem>> {
    let mut page = subscriptions_feed()?;
    let mut out: Vec<VideoItem> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for n in 0..max_pages {
        out.extend(page.items.drain(..).filter_map(|i| match i {
            Item::Video(v) if seen.insert(v.id.clone()) => Some(v),
            _ => None,
        }));
        match page.continuation.take() {
            Some(tok) if n + 1 < max_pages => match browse_continue(&tok) {
                Ok(p) => page = p,
                Err(_) => break,
            },
            _ => break,
        }
    }
    Ok(out)
}

/// Copies the account's subscriptions into the local list. Returns how many were new.
pub fn sync_subscriptions() -> Result<usize> {
    let chans = subscribed_channels()?;
    let mut added = 0;
    for c in chans {
        if !crate::store::is_subscribed(&c.id) {
            added += 1;
        }
        crate::store::subscribe(&c.id, &c.name, c.avatar.as_deref());
    }
    Ok(added)
}

pub fn like(video_id: &str, kind: &str) -> Result<()> {
    // kind: "like" | "dislike" | "removelike"
    tv_call(&format!("like/{kind}"), json!({ "target": { "videoId": video_id } }))?;
    Ok(())
}

pub fn subscribe(channel_id: &str, on: bool) -> Result<()> {
    let ep = if on { "subscription/subscribe" } else { "subscription/unsubscribe" };
    tv_call(ep, json!({ "channelIds": [channel_id] }))?;
    Ok(())
}

/// Adds the video to the account's YouTube watch history, which is what keeps the
/// home recommendations personal.
pub fn mark_watched(video_id: &str) -> Result<()> {
    let token = access_token().ok_or_else(|| anyhow!("not signed in"))?;
    let r = tv_call("player", json!({ "videoId": video_id, "racyCheckOk": true, "contentCheckOk": true }))?;
    let base = r
        .pointer("/playbackTracking/videostatsPlaybackUrl/baseUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no playback tracking url"))?;
    let cpn: String = {
        use rand::Rng;
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut rng = rand::rng();
        (0..16).map(|_| A[rng.random_range(0..A.len())] as char).collect()
    };
    let url = format!("{base}&ver=2&cpn={cpn}");
    let auth = format!("Bearer {token}");
    http::get_bytes(&url, &[("Authorization", &auth), ("User-Agent", TV_UA)])?;
    Ok(())
}
