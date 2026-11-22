//! Download manager (yt-dlp when available, native fallback) and external player launching.
use crate::api::streams::{self, hide_window};
use crate::api::types::VideoItem;
use crate::store;
use crate::ui::toast;
use gpui_kit::*;
use parking_lot::Mutex;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    Done(String),
    Failed(String),
    Cancelled,
}

#[derive(Clone)]
pub struct Job {
    pub id: u64,
    pub video: VideoItem,
    pub format: String,
    pub status: JobStatus,
    pub progress: f32,
    pub detail: String,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct Downloads {
    pub jobs: Arc<Mutex<Vec<Job>>>,
    next: u64,
    ticking: bool,
}

impl Global for Downloads {}

pub const FORMATS: &[(&str, &str)] = &[
    ("1080", "1080p (recommended)"),
    ("best", "Best available (up to 4K, large files)"),
    ("720", "Up to 720p"),
    ("480", "Up to 480p"),
    ("audio", "Audio only"),
];

/// Starts several downloads, asking for the folder at most once.
pub fn start_many(videos: Vec<VideoItem>, cx: &mut App) {
    if videos.is_empty() {
        return;
    }
    if !store::settings_snapshot().download_dir_chosen {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Save downloads here".into()) });
        cx.spawn(async move |cx| {
            let picked = rx.await.ok().and_then(|r| r.ok()).flatten().and_then(|p| p.into_iter().next());
            let _ = cx.update(|cx| match picked {
                Some(dir) => {
                    store::update_settings(|s| {
                        s.download_dir = dir.to_string_lossy().into_owned();
                        s.download_dir_chosen = true;
                    });
                    start_many(videos, cx);
                }
                None => toast(cx, "Download cancelled"),
            });
        })
        .detach();
        return;
    }
    let n = videos.len();
    for v in videos {
        start(v, None, cx);
    }
    if n > 1 {
        toast(cx, format!("Downloading {n} videos"));
    }
}

pub fn start(video: VideoItem, format: Option<String>, cx: &mut App) {
    // First download: ask where to save (once), like a browser does.
    if !store::settings_snapshot().download_dir_chosen {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Save downloads here".into()),
        });
        cx.spawn(async move |cx| {
            let picked = rx.await.ok().and_then(|r| r.ok()).flatten().and_then(|p| p.into_iter().next());
            let _ = cx.update(|cx| match picked {
                Some(dir) => {
                    store::update_settings(|s| {
                        s.download_dir = dir.to_string_lossy().into_owned();
                        s.download_dir_chosen = true;
                    });
                    start(video, format, cx);
                }
                None => toast(cx, "Download cancelled"),
            });
        })
        .detach();
        return;
    }
    let fmt = format.unwrap_or_else(|| store::settings_snapshot().download_format.clone());
    // separate video + audio streams need FFmpeg to be joined into one file
    if fmt != "audio" && fmt != "single" && streams::ytdlp_path().is_some() && ffmpeg_path().is_none() {
        if FFMPEG_FETCHING.load(Ordering::SeqCst) {
            // queued behind the FFmpeg download
            PENDING.lock().push((video, Some(fmt)));
            return;
        }
        let v2 = video.clone();
        crate::ui::toast_with_actions(
            cx,
            "Videos above 360p need FFmpeg to join video and audio",
            vec![
                (
                    "Get FFmpeg (91 MB)".into(),
                    Arc::new(move |_, cx: &mut App| {
                        PENDING.lock().push((video.clone(), Some(fmt.clone())));
                        fetch_ffmpeg(cx);
                    }),
                ),
                ("Download 360p".into(), Arc::new(move |_, cx: &mut App| start(v2.clone(), Some("single".into()), cx))),
            ],
        );
        return;
    }
    let d = cx.default_global::<Downloads>();
    d.next += 1;
    let job = Job {
        id: d.next,
        video: video.clone(),
        format: fmt,
        status: JobStatus::Queued,
        progress: 0.0,
        detail: String::new(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let jobs = d.jobs.clone();
    jobs.lock().push(job.clone());
    toast(cx, format!("Downloading “{}”", video.title));
    let jid = job.id;
    let cancel = job.cancel.clone();
    let jobs2 = jobs.clone();
    std::thread::spawn(move || {
        let update = |f: &dyn Fn(&mut Job)| {
            if let Some(j) = jobs2.lock().iter_mut().find(|j| j.id == jid) {
                f(j);
            }
        };
        // at most 3 downloads at a time; the rest wait their turn
        loop {
            if cancel.load(Ordering::SeqCst) {
                update(&|j| j.status = JobStatus::Cancelled);
                return;
            }
            let running = jobs2.lock().iter().filter(|j| j.status == JobStatus::Running).count();
            if running < 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        update(&|j| j.status = JobStatus::Running);
        let res = run_job(&job, &cancel, &|p, detail| {
            update(&|j| {
                j.progress = p;
                j.detail = detail.to_string();
            })
        })
        .map(|path| resolve_output(&path, &job.video.id).unwrap_or(path));
        match res {
            Ok(path) => update(&|j| {
                j.status = JobStatus::Done(path.clone());
                j.progress = 1.0;
            }),
            Err(e) if cancel.load(Ordering::SeqCst) => {
                let _ = e;
                update(&|j| j.status = JobStatus::Cancelled)
            }
            Err(e) => update(&|j| j.status = JobStatus::Failed(e.to_string())),
        }
    });
    ensure_ticking(cx);
}

fn ensure_ticking(cx: &mut App) {
    if cx.default_global::<Downloads>().ticking {
        return;
    }
    cx.default_global::<Downloads>().ticking = true;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
            let active = cx
                .update(|cx| {
                    cx.refresh_windows();
                    let d = cx.default_global::<Downloads>();
                    let active = d.jobs.lock().iter().any(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running));
                    if !active {
                        d.ticking = false;
                    }
                    if !active {
                        save_history(cx);
                    }
                    active
                });
            if !active {
                break;
            }
        }
    })
    .detach();
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { '_' } else { c })
        .collect::<String>()
        .trim()
        .chars()
        .take(150)
        .collect()
}

fn run_job(job: &Job, cancel: &AtomicBool, progress: &dyn Fn(f32, &str)) -> anyhow::Result<String> {
    let s = store::settings_snapshot();
    let dir = std::path::PathBuf::from(if s.download_dir.is_empty() { ".".into() } else { s.download_dir.clone() });
    std::fs::create_dir_all(&dir)?;
    let url = format!("https://www.youtube.com/watch?v={}", job.video.id);
    if let Some(bin) = streams::ytdlp_path() {
        let fmt = match job.format.as_str() {
            "audio" => "bestaudio[ext=m4a]/bestaudio".to_string(),
            // one file with video and audio (no FFmpeg needed)
            "single" => "b".to_string(),
            "best" => "bv*+ba/b".to_string(),
            h => format!("bv*[height<={h}]+ba/b[height<={h}]/b"),
        };
        match ytdlp_run(&bin, &dir, &url, &fmt, &s, cancel, progress) {
            Err(e) if e.to_string().contains("403") && !cancel.load(Ordering::SeqCst) => {
                // a stream URL was refused: retry preferring the HLS formats, which don't need a PO token
                progress(0.0, "Retrying…");
                let hls = format!("{}/{}", fmt.replace("bv*", "bv*[protocol^=m3u8]"), fmt);
                return ytdlp_run(&bin, &dir, &url, &hls, &s, cancel, progress);
            }
            other => return other,
        }
    }
    native_download(job, &dir, cancel, progress)
}

fn ytdlp_run(bin: &str, dir: &std::path::Path, url: &str, fmt: &str, s: &store::Settings, cancel: &AtomicBool, progress: &dyn Fn(f32, &str)) -> anyhow::Result<String> {
    {
        let mut cmd = Command::new(bin);
        if let Some(ff) = ffmpeg_path().filter(|f| f != "ffmpeg") {
            cmd.arg("--ffmpeg-location").arg(ff);
        }
        if s.ipv4_only {
            // YouTube's bot check is triggered far more often on IPv6
            cmd.arg("--force-ipv4");
        }
        // --print makes yt-dlp quiet, so progress is asked for explicitly in a fixed format.
        // Fragments and chunks download in parallel, which is much faster than one stream.
        cmd.args(["--newline", "--no-warnings", "--no-playlist", "--progress", "--progress-delta", "0.4"])
            .args(["--progress-template", "download:[dl] %(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s"])
            .args(["-N", "8", "--http-chunk-size", "10M", "-f", fmt, "-o"])
            .arg(dir.join("%(title).150B [%(id)s].%(ext)s"))
            .args(["--print", "after_move:filepath"])
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if s.sponsorblock && ffmpeg_path().is_some() {
            cmd.args(["--sponsorblock-mark", "all"]);
        }
        hide_window(&mut cmd);
        let mut child = cmd.spawn()?;
        progress(0.0, "Getting video info…");
        let stdout = child.stdout.take().unwrap();
        // drain stderr on its own thread so a chatty yt-dlp can't block on a full pipe
        let stderr = child.stderr.take().unwrap();
        let err_reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
            s
        });
        let mut final_path = String::new();
        // video and audio arrive one after the other: weight them 90 / 10
        let (mut phase, mut last_pct) = (0u32, 0.0f32);
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                anyhow::bail!("cancelled");
            }
            if let Some(rest) = line.strip_prefix("[dl]") {
                let mut parts = rest.trim().split('|');
                let pct = parts.next().and_then(|p| p.trim().trim_end_matches('%').parse::<f32>().ok()).unwrap_or(0.0);
                let speed = parts.next().unwrap_or("").trim();
                let eta = parts.next().unwrap_or("").trim();
                if pct + 20.0 < last_pct {
                    phase += 1;
                }
                last_pct = pct;
                let overall = if phase == 0 { pct / 100.0 * 0.9 } else { 0.9 + pct / 100.0 * 0.1 };
                let what = if phase == 0 { "" } else { "Audio · " };
                let detail = if pct >= 100.0 { "Finishing…".to_string() } else { format!("{what}{pct:.0}% · {speed} · {eta} left") };
                progress(overall.min(0.99), &detail);
            } else if !line.starts_with('[') && !line.trim().is_empty() {
                final_path = line.trim().to_string();
            }
        }
        let status = child.wait()?;
        let err = err_reader.join().unwrap_or_default();
        if !status.success() {
            let last = err.lines().rev().find(|l| l.contains("ERROR")).or(err.lines().last()).unwrap_or("yt-dlp failed").to_string();
            if last.contains("confirm you") && last.contains("bot") {
                anyhow::bail!("YouTube asked for a bot check. Turn on \"Use IPv4 only\" in Settings → Network, or update yt-dlp.");
            }
            anyhow::bail!("{}", last.trim_start_matches("ERROR: ").trim());
        }
        Ok(if final_path.is_empty() { dir.to_string_lossy().into_owned() } else { final_path })
    }
}

/// Without yt-dlp: progressive (muxed) stream or audio-only stream.
fn native_download(job: &Job, dir: &std::path::Path, cancel: &AtomicBool, progress: &dyn Fn(f32, &str)) -> anyhow::Result<String> {
    let info = streams::get_streams(&job.video.id)?;
    let (fmt, ext) = if job.format == "audio" {
        let a = info.audio.iter().find(|a| a.mime.contains("mp4")).or(info.audio.first()).ok_or_else(|| anyhow::anyhow!("no audio stream"))?;
        (a.clone(), if a.mime.contains("mp4") { "m4a" } else { "webm" })
    } else {
        let m = info.muxed.iter().max_by_key(|m| m.height).ok_or_else(|| anyhow::anyhow!("no combined stream (install yt-dlp for high quality downloads)"))?;
        (m.clone(), "mp4")
    };
    let path = dir.join(format!("{} [{}].{ext}", sanitize(&info.title), job.video.id));
    let mut f = std::fs::File::create(&path)?;
    crate::api::http::download_to(&fmt.url, &mut f, |done, total| {
        let p = total.map(|t| done as f32 / t as f32).unwrap_or(0.0);
        progress(p, &format!("{:.1} MB", done as f64 / 1e6));
        !cancel.load(Ordering::SeqCst)
    })?;
    Ok(path.to_string_lossy().into_owned())
}

pub fn cancel(id: u64, cx: &mut App) {
    if let Some(j) = cx.default_global::<Downloads>().jobs.lock().iter().find(|j| j.id == id) {
        j.cancel.store(true, Ordering::SeqCst);
    }
}

pub fn open_external(id: &str, cx: &mut App) {
    open_external_at(id, 0.0, cx)
}

pub fn open_external_at(id: &str, time: f64, cx: &mut App) {
    let s = store::settings_snapshot();
    let player = if s.external_player.trim().is_empty() { "mpv".to_string() } else { s.external_player.clone() };
    let url = format!("https://www.youtube.com/watch?v={id}");
    let mut cmd = Command::new(&player);
    let lower = player.to_lowercase();
    if lower.contains("mpv") {
        if time > 1.0 {
            cmd.arg(format!("--start={}", time as u64));
        }
        if let Some(y) = streams::ytdlp_path() {
            cmd.arg(format!("--script-opts=ytdl_hook-ytdl_path={y}"));
        }
    } else if lower.contains("vlc") && time > 1.0 {
        cmd.arg(format!("--start-time={}", time as u64));
    }
    cmd.arg(&url);
    match cmd.spawn() {
        Ok(_) => toast(cx, format!("Opened in {player}")),
        Err(e) => toast(cx, format!("Could not launch {player}: {e}. Set the path in Settings → Player.")),
    }
}

/// Removes a finished/failed job from the list (the file stays).
pub fn remove(id: u64, cx: &mut App) {
    cx.default_global::<Downloads>().jobs.lock().retain(|j| j.id != id || matches!(j.status, JobStatus::Running | JobStatus::Queued));
    save_history(cx);
    cx.refresh_windows();
}

/// Clears every finished/failed/cancelled entry.
pub fn clear_history(cx: &mut App) {
    cx.default_global::<Downloads>().jobs.lock().retain(|j| matches!(j.status, JobStatus::Running | JobStatus::Queued));
    save_history(cx);
    cx.refresh_windows();
}

/// Deletes the downloaded file and removes the entry.
pub fn delete_file(id: u64, cx: &mut App) {
    let path = cx.default_global::<Downloads>().jobs.lock().iter().find(|j| j.id == id).and_then(|j| match &j.status {
        JobStatus::Done(p) => Some(p.clone()),
        _ => None,
    });
    if let Some(p) = path {
        match std::fs::remove_file(&p) {
            Ok(()) => toast(cx, "Deleted from disk"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                toast(cx, format!("Could not delete: {e}"));
                return;
            }
        }
    }
    remove(id, cx);
}

/// The real file for a finished download. yt-dlp reports the merged name even
/// when the merge didn't happen, and older entries may only hold the folder, so
/// fall back to the largest media file in the folder tagged with the video id.
pub fn resolve_output(path: &str, video_id: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    if p.is_file() {
        return Some(path.to_string());
    }
    let dir = if p.is_dir() { p.to_path_buf() } else { p.parent()?.to_path_buf() };
    let tag = format!("[{video_id}]");
    std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.contains(&tag) && !n.ends_with(".part") && !n.ends_with(".ytdl") && !n.ends_with(".tmp")
        })
        .filter_map(|e| Some((e.metadata().ok()?.len(), e.path())))
        .max_by_key(|(len, _)| *len)
        .map(|(_, p)| p.to_string_lossy().into_owned())
}

/// Opens a downloaded file with the system's default player.
pub fn open_file(path: &str, cx: &mut App) {
    let Some(path) = std::path::Path::new(path).is_file().then(|| path.to_string()).or_else(|| {
        // find the id from the "[id]" in the file name
        let name = std::path::Path::new(path).file_name()?.to_string_lossy().into_owned();
        let id = name.rsplit_once('[')?.1.split_once(']')?.0.to_string();
        resolve_output(path, &id)
    }) else {
        toast(cx, "File not found (moved or deleted)");
        return;
    };
    // the default app is launched off the UI thread; Explorer is the most
    // reliable way to hand a file to its default app on Windows
    std::thread::spawn(move || {
        #[cfg(windows)]
        {
            let mut c = Command::new("explorer");
            c.arg(&path);
            if c.spawn().is_ok() {
                return;
            }
        }
        let _ = open::that(&path);
    });
}

/// (active jobs, overall progress 0..1 of active jobs)
pub fn summary(cx: &App) -> (usize, f32) {
    let Some(d) = cx.try_global::<Downloads>() else { return (0, 0.0) };
    let jobs = d.jobs.lock();
    let active: Vec<&Job> = jobs.iter().filter(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running)).collect();
    if active.is_empty() {
        return (0, 0.0);
    }
    let p = active.iter().map(|j| j.progress).sum::<f32>() / active.len() as f32;
    (active.len(), p)
}

// ── history: finished downloads survive restarts ──

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    video: VideoItem,
    format: String,
    path: Option<String>,
    error: Option<String>,
}

const HISTORY_KEY: &str = "download_history";

fn save_history(cx: &mut App) {
    let jobs = cx.default_global::<Downloads>().jobs.lock().clone();
    let saved: Vec<Saved> = jobs
        .iter()
        .filter_map(|j| match &j.status {
            JobStatus::Done(p) => Some(Saved { video: j.video.clone(), format: j.format.clone(), path: Some(p.clone()), error: None }),
            JobStatus::Failed(e) => Some(Saved { video: j.video.clone(), format: j.format.clone(), path: None, error: Some(e.clone()) }),
            _ => None,
        })
        .collect();
    if let Ok(s) = serde_json::to_string(&saved) {
        store::kv_set(HISTORY_KEY, &s);
    }
}

/// Loads the saved download list at startup.
pub fn load_history(cx: &mut App) {
    let Some(raw) = store::kv_get(HISTORY_KEY) else { return };
    let Ok(saved) = serde_json::from_str::<Vec<Saved>>(&raw) else { return };
    let d = cx.default_global::<Downloads>();
    let mut jobs = d.jobs.lock();
    for s in saved {
        d.next += 1;
        jobs.push(Job {
            id: d.next,
            video: s.video,
            format: s.format,
            status: match (s.path, s.error) {
                (Some(p), _) => JobStatus::Done(p),
                (None, e) => JobStatus::Failed(e.unwrap_or_default()),
            },
            progress: 1.0,
            detail: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
    }
}

// ── FFmpeg (joins separate video and audio streams) ──

static FFMPEG_FETCHING: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Vec<(VideoItem, Option<String>)>> = Mutex::new(Vec::new());

const FFMPEG_URL: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";

fn ffmpeg_dir() -> std::path::PathBuf {
    store::data_dir().join("ffmpeg")
}

/// FFmpeg next to the app, in the app's data folder, or on PATH.
pub fn ffmpeg_path() -> Option<String> {
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let mut cands = vec![ffmpeg_dir().join(exe)];
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        cands.push(d.join(exe));
        cands.push(d.join("ffmpeg").join(exe));
    }
    if let Some(p) = cands.into_iter().find(|p| p.is_file()) {
        return Some(p.to_string_lossy().into_owned());
    }
    static ON_PATH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let on_path = *ON_PATH.get_or_init(|| {
        let mut c = Command::new("ffmpeg");
        c.arg("-version").stdout(Stdio::null()).stderr(Stdio::null());
        hide_window(&mut c);
        c.status().map(|s| s.success()).unwrap_or(false)
    });
    on_path.then(|| "ffmpeg".to_string())
}

/// Downloads FFmpeg into the data folder, then starts the downloads waiting for it.
pub fn fetch_ffmpeg(cx: &mut App) {
    if !cfg!(windows) {
        toast(cx, "Install FFmpeg with your package manager (e.g. sudo apt install ffmpeg)");
        return;
    }
    if FFMPEG_FETCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    toast(cx, "Downloading FFmpeg…");
    cx.spawn(async move |cx| {
        let res = crate::ui::blocking(|| -> anyhow::Result<()> {
            let dir = ffmpeg_dir();
            let tmp = store::data_dir().join("ffmpeg-download");
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp)?;
            let zip = tmp.join("ffmpeg.zip");
            {
                let mut f = std::fs::File::create(&zip)?;
                crate::api::http::download_to(FFMPEG_URL, &mut f, |_, _| true)?;
            }
            // Windows 10+ ships bsdtar, which reads zip files
            let mut c = Command::new("tar");
            c.arg("-xf").arg(&zip).arg("-C").arg(&tmp);
            hide_window(&mut c);
            if !c.status()?.success() {
                anyhow::bail!("could not unpack FFmpeg");
            }
            let bin = std::fs::read_dir(&tmp)?
                .filter_map(Result::ok)
                .map(|e| e.path().join("bin"))
                .find(|p| p.join("ffmpeg.exe").is_file())
                .ok_or_else(|| anyhow::anyhow!("ffmpeg.exe not found in the archive"))?;
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            for e in std::fs::read_dir(&bin)?.filter_map(Result::ok) {
                std::fs::copy(e.path(), dir.join(e.file_name()))?;
            }
            let _ = std::fs::remove_dir_all(&tmp);
            Ok(())
        })
        .await;
        FFMPEG_FETCHING.store(false, Ordering::SeqCst);
        let _ = cx.update(|cx| {
            let pending: Vec<_> = std::mem::take(&mut *PENDING.lock());
            match res {
                Ok(()) => {
                    toast(cx, "FFmpeg is ready");
                    for (v, f) in pending {
                        start(v, f, cx);
                    }
                }
                Err(e) => toast(cx, format!("FFmpeg download failed: {e}")),
            }
        });
    })
    .detach();
}
