<p align="center">
  <img src="assets/logo.png" alt="Stream Tube" width="120">
</p>

<h1 align="center">Stream Tube</h1>

<p align="center">
  A native YouTube client for Windows and Linux, written in Rust.<br>
  No browser, no ads, no tracking. SponsorBlock, DeArrow and Return YouTube Dislike built in.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20Linux-0078D6" alt="Windows and Linux">
  <img src="https://img.shields.io/github/v/release/Avaneesh-Inamdar/Stream-Tube-Desktop?include_prereleases&label=release" alt="Latest release">
  <img src="https://img.shields.io/badge/built%20with-Rust-B7410E" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT-green" alt="MIT license">
</p>

<p align="center">
  <img src="docs/screenshots/home.png" alt="Home feed" width="900">
</p>

## Why

I wanted YouTube on my desktop without keeping a browser tab open, and without the
ads, autoplaying junk and sponsor reads. The existing options were either Electron
apps that eat a gigabyte of RAM or web frontends that break every other week.
Stream Tube is a single native app: the UI is drawn on the GPU with
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), and video
goes through libmpv, so it starts fast and scrolls smoothly even with big feeds.

## What it does

**Watching**
- Home, Shorts, Subscriptions, Explore categories, channels, playlists and search
- libmpv player with hardware decoding, codec preference, speed control (remembered per channel if you want), captions and seek-bar preview thumbnails
- Resumes where you left off, a miniplayer while you browse, and a picture-in-picture window
- Keeps playing when minimized, has an audio-only mode, or can hand a video off to an external player
- Live streams with live chat (Top chat or every message), and chatting when signed in
- Hold Space or press and hold on the video to play at 2x
- Hover previews: rest the pointer on a thumbnail and it plays inside it, muted, with a seek bar to skip around
- Most replayed graph with "jump ahead", ambient glow around the player, force original audio instead of auto-dubbing

**Less noise**
- SponsorBlock with per-category behaviour: skip automatically, ask, only mark it on the seek bar, or ignore
- Use your own SponsorBlock private ID and see your contribution stats (segments, skips by others, time saved)
- DeArrow titles and thumbnails, so you see what a video is actually about
- Return YouTube Dislike counts
- Hide Shorts, live streams, premieres, comments, related videos, view counts and more
- Block channels or keywords from every feed

**Your data stays yours**
- Subscriptions can live locally. The feed is built by fetching each channel directly, no account needed
- Or sign in with your Google account to get your own Home feed, subscriptions, playlists, likes and subscribe button
- Watch history from your other devices syncs in with resume positions, and single entries can be removed
- History, watch later, favourites and playlists are stored in a local SQLite database
- Import from YouTube Takeout, NewPipe, FreeTube or OPML. Export to NewPipe JSON, YouTube CSV, OPML or a full backup

**Downloads**
- Download video or audio in any quality through yt-dlp, with SponsorBlock chapters
- Without yt-dlp it falls back to the built-in downloader (360p/720p or audio)

**Looks the way you like**
- Dark, light or follow Windows
- Seven accent colours used across the app, interface scaling, density, thumbnail corner style and grid columns

**Opens YouTube links**
- `streamtube://` links open in the app, and a browser bookmark (*Settings → General*) sends the YouTube page you're on to Stream Tube
- Links opened while the app is running reuse the open window

## Performance

Stream Tube versus youtube.com in Microsoft Edge (clean profile, no extensions) on
the same Windows 11 PC, playing the same video. Numbers are averaged over ten
seconds and add up every process involved.

| | youtube.com in Edge | Stream Tube 0.2.0-alpha |
|---|---|---|
| Processes | 11–12 | 1 |
| RAM, idle on Home (working set) | 1,073 MB | **302 MB** |
| RAM, idle on Home (private) | 670 MB | **333 MB** |
| RAM, playing a video (working set) | 1,203 MB | **507 MB** |
| RAM, playing a video (private) | 769 MB | **545 MB** |
| CPU while playing | ~0.7% | ~1.6% |

Idle, Stream Tube uses about a third of the RAM the browser needs for the same page.
While playing it uses well under half. Much of that is a read-ahead buffer (up to
200 MB) that keeps seeking instant. The browser uses less CPU in this test because
it decodes video on the GPU by default. Turn on *Settings → Player → Hardware
decoding* to do the same in Stream Tube. Results depend on the video, resolution
and hardware.

## Screenshots

<table>
  <tr>
    <td><img src="docs/screenshots/live.png" alt="Live stream with live chat"></td>
    <td><img src="docs/screenshots/search.png" alt="Search results"></td>
  </tr>
  <tr>
    <td align="center">Live streams with chat</td>
    <td align="center">Search</td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/settings.png" alt="General settings"></td>
    <td><img src="docs/screenshots/sponsorblock.png" alt="SponsorBlock settings"></td>
  </tr>
  <tr>
    <td align="center">Settings</td>
    <td align="center">SponsorBlock and DeArrow</td>
  </tr>
</table>

## Install

Get the latest build from the [Releases page](https://github.com/Avaneesh-Inamdar/Stream-Tube-Desktop/releases).
This is an early alpha, so expect rough edges.

**Windows 10/11 (64-bit)**: run `StreamTube-<version>-setup.exe`. It installs for
your user only, so no administrator prompt, and the app updates itself when a new
release comes out (*Settings → About*).

**Debian, Ubuntu, Mint**: install the `.deb`. This also installs libmpv:

```sh
sudo apt install ./streamtube_0.2.0-alpha_amd64.deb
```

**Fedora, openSUSE**: install the `.rpm`:

```sh
sudo dnf install ./streamtube-0.2.0-alpha.x86_64.rpm
```

**Other Linux (x86_64)**: unpack `StreamTube-<version>-linux-x86_64.tar.gz` and run
`./install.sh`. It puts the app in `~/.local/bin` and adds a menu entry.
Video plays through libmpv, so install it if you don't have it:

```sh
sudo apt install libmpv2      # Debian 12+, Ubuntu 24.04+ (Ubuntu 22.04: libmpv1)
sudo dnf install mpv-libs     # Fedora
sudo pacman -S mpv            # Arch
```

Downloads above 360p need yt-dlp and FFmpeg. On Windows the app offers to fetch
both the first time you need them. On Linux, install them with your package manager.

## Building

You need:

- **Rust 1.85 or newer** (the crate uses the 2024 edition). Get it from [rustup.rs](https://rustup.rs).
- **libmpv** — download a `mpv-dev-x86_64` archive from
  [shinchiro's mpv builds](https://github.com/shinchiro/mpv-winbuild-cmake/releases)
  and put `libmpv-2.dll` in `third_party/mpv/`. It isn't in the repo because it's over 100 MB.
- **yt-dlp** (optional, recommended) — on your `PATH`, or point to it in
  *Settings → Backend & network*. It's used for downloads and as a fallback when
  the built-in extractor can't get a stream.

Then:

```powershell
git clone https://github.com/Avaneesh-Inamdar/Stream-Tube-Desktop.git
cd Stream-Tube-Desktop
cargo build --release
```

The executable ends up at `target\release\streamtube.exe`. To run it somewhere
else, copy `libmpv-2.dll` next to it.

To build the installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run:

```powershell
ISCC.exe /DAppVersion=0.2.0-alpha installer\streamtube.iss
```

It lands in `dist\`. On Linux, install the packages listed in
[`.github/workflows/linux.yml`](.github/workflows/linux.yml) and run `cargo build --release`.

## Where things are stored

Settings, history and subscriptions go in `%APPDATA%\StreamTube\streamtube.db` on
Windows and `~/.local/share/StreamTube/streamtube.db` on Linux. Set `STREAMTUBE_DATA_DIR`
to keep them somewhere else (for example a portable copy).
If you used the app back when it was called Ferrotube, your old data is moved
over automatically on first launch.

## Project layout

```
src/
  api/       Innertube client, stream extraction, sign-in, SponsorBlock/DeArrow/RYD
  player/    libmpv binding, playback controller, player UI, local stream proxy
  ui/        app shell, pages (home, watch, shorts, settings, ...), cards, PiP window
  feed.rs    local subscription feed
  import.rs  Takeout / NewPipe / FreeTube / OPML import
  store.rs   SQLite storage
```

## Known issues

- YouTube sometimes answers with "Sign in to confirm you're not a bot". When that
  happens, signing in inside the app or updating yt-dlp usually fixes it.
- Windows is the main platform. The Linux build is new and less tested; macOS isn't built yet.

## Credits

Stream Tube leans on a lot of other people's work:
[mpv](https://mpv.io), [yt-dlp](https://github.com/yt-dlp/yt-dlp),
[SponsorBlock](https://sponsor.ajay.app), [DeArrow](https://dearrow.ajay.app),
[Return YouTube Dislike](https://returnyoutubedislike.com) and
[GPUI](https://www.gpui.rs).

It's an independent project and isn't affiliated with or endorsed by YouTube or Google.

## License

[MIT](LICENSE)
