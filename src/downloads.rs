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
    ("best", "Best quality (video + audio)"),
    ("1080", "Up to 1080p"),
    ("720", "Up to 720p"),
    ("480", "Up to 480p"),
    ("audio", "Audio only"),
];

pub fn start(video: VideoItem, format: Option<String>, cx: &mut App) {
    let fmt = format.unwrap_or_else(|| store::settings_snapshot().download_format.clone());
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
        update(&|j| j.status = JobStatus::Running);
        let res = run_job(&job, &cancel, &|p, detail| {
            update(&|j| {
                j.progress = p;
                j.detail = detail.to_string();
            })
        });
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
            "best" => "bv*+ba/b".to_string(),
            h => format!("bv*[height<={h}]+ba/b[height<={h}]/b"),
        };
        let mut cmd = Command::new(bin);
        cmd.args(["--newline", "--no-warnings", "--no-playlist", "-f", &fmt, "-o"])
            .arg(dir.join("%(title).150B [%(id)s].%(ext)s"))
            .args(["--print", "after_move:filepath"])
            .arg(&url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if store::settings_snapshot().sponsorblock {
            cmd.args(["--sponsorblock-mark", "all"]);
        }
        hide_window(&mut cmd);
        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().unwrap();
        let mut final_path = String::new();
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                anyhow::bail!("cancelled");
            }
            if let Some(rest) = line.strip_prefix("[download]") {
                if let Some(pct) = rest.split('%').next().and_then(|p| p.trim().parse::<f32>().ok()) {
                    progress(pct / 100.0, rest.trim());
                }
            } else if !line.starts_with('[') && !line.trim().is_empty() {
                final_path = line.trim().to_string();
            }
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("yt-dlp failed"));
        }
        return Ok(if final_path.is_empty() { dir.to_string_lossy().into_owned() } else { final_path });
    }
    // Native fallback: progressive (muxed) stream or audio-only stream.
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
