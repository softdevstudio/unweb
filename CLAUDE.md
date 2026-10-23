# Unweb

Unweb is a tiny browser where every visit is a first visit. Every window is a stranger: it starts with no history, no logins and no cookies, and when it closes everything it collected is deleted. Several strangers can run side by side, each kept apart from the others and from the normal browser. It runs on the system web engines (WebView2, WKWebView, WebKitGTK) through Tauri 2.

Domain: unweb.org
Code repository: softdevstudio/unweb
Site repository: photozeitgeist/unweb.org
Language: Rust, Tauri 2
Code license: Apache 2.0
Contact: info@unweb.org

Origin: YK decided on October 23, 2026 that Unweb copies Watchkeeper's core crate (softdevstudio/watchkeeper, crates/watchkeeper-core) into this repository once, as a starter for the cookie counter in 0.2, and edits and builds on it here. After that copy the two are separate, unrelated projects: no shared code, no syncing of fixes, no dependency on the Watchkeeper repository.

## Low maintenance workflow

YK supplies the go-ahead, language choice, repositories and access. The agent handles code, tests, documentation, CI, automated releases, site work and upkeep. Answer questions without treating them as instructions to implement. Update Next only when YK approves a new step; record actual progress without inventing approval.

Work on main only; main is the default branch. This CLAUDE.md is the sole plan and session handoff file. Do not create ROADMAP.md, RESUME.md or another status file. Session notes go in Where we are, never as files in docs/; docs/ holds reference material only. Pause and report after each session and wait for YK's go-ahead before the next.

README.md is the manual and plain-language explainer. Reference documentation belongs in docs/ as Markdown. No Word or PDF. Keep LICENSE, NOTICE and THIRD_PARTY_LICENSES.md current (regenerate the last with scripts/third-party-licenses.py when dependencies change). Unweb is a separate project: never mix in code, names or content from unrelated projects (the one-time starter copy of Watchkeeper's core is the exception, and is Unweb's own code from then on). Do not discuss domain sale or domain value anywhere in project materials.

Write in plain words with varied sentence length. Avoid em dashes, formulaic contrasts, decorative vocabulary and forced lists of three.

## Screening record

Agreed with YK on October 23, 2026.

Money: free open-source app. Paid features later for professionals: saved session setups ("5 strangers in 5 countries"), side-by-side screenshot comparison and team sharing. SEO and ad-verification people already pay for tools that show clean, unpersonalized results. No revenue model is approved.

Real problem: your normal browser is soaked in your history and logins, so you never see what a stranger sees. Incognito windows share one session with each other and are awkward to run in parallel.

Plain-language technology: "A browser where every visit is your first visit. Open as many strangers as you like, side by side. Close them, and they never existed."

Differentiation: multi-profile browsers exist, but they're mostly heavy Chromium-based tools aimed at managing many accounts. Unweb is tiny, open source and built around the opposite idea: no accounts, no memory, a clean look every time.

What a stranger still gives away, to be said plainly on the site: the IP address and rough location; the device (all strangers on one machine share screen size, fonts and hardware, so fingerprinting could link them); and it is not a malware sandbox (same system engine).

## Design

Strangers (crates/unweb/src/strangers.rs): each stranger is a window (WebviewWindowBuilder, label "stranger-N") with a throwaway store of its own. On Windows and Linux that is a data folder `<app data>/strangers/run-<random>/stranger-N-<random>` passed as data_directory, which gives each stranger its own WebView2 environment or WebKitGTK WebContext, and so its own cookies, storage, HTTP cache, service workers and everything else. On macOS it is incognito (a new non-persistent WKWebsiteDataStore per webview), which keeps nothing on disk.

Closing: a stranger's CloseRequested is prevented and the close goes through Strangers::retire, which on Linux first clears the engine's website data for the stranger (WebsiteDataManager::clear, ALL) and then destroys the window. On Destroyed, Strangers::closed wipes the folder: it waits until nothing in the folder has changed for 4 s (at most 20 s), deletes it (retrying while files are held, up to 30 s), then watches it for 10 s and deletes it again if anything writes it back. This is needed because Tauri 2.12 keeps a WebKitGTK WebContext alive for the app's lifetime (tauri issue 14626), and its network process writes cache records (SubResources, dump.json) 2 to 7 s after a page closes, recreating the folder. When the last stranger closes, the app waits (up to 90 s) for running wipes before exiting. Each running copy has its own run folder and holds a lock on `run-<random>.lock` beside it (std File::try_lock; the file is created and locked as run-<random>.lock.new and then renamed, so a copy starting at the same moment never sees it unlocked, and purges skip lock files under 10 s old). At start, run folders whose lock is free (a crash, a kill, a page's window.close) and anything that isn't a run folder are deleted; a live copy's run is left alone. At exit the run folder is removed, then the lock.

Outside the folder: Linux sets MESA_SHADER_CACHE_DISABLE, MESA_GLSL_CACHE_DISABLE and __GL_SHADER_DISK_CACHE=0 at the top of main (engine processes inherit them) and turns off WebKitGTK's persistent credential storage per stranger (webkit2gtk v2_30). Windows passes --disable-gpu-shader-disk-cache along with wry's default --disable-features list (additional_browser_args replaces it). Driver shader caches on Windows and macOS are a stated gap.

No app commands exist yet and no capabilities are granted, so a page can't call anything in the app.

Known gaps to cover in 0.1 tests: HSTS records and cache-based tracking across strangers (needs a local HTTPS server), separation from the normal browser, and Windows folder deletion timing with WebView2's browser process.

## Code and completion standard

Tests for every operation, plus failure tests aimed at the main promise: nothing carries over between strangers, from a closed stranger to a new one, or into the normal browser, and nothing is left on disk after closing. The end-to-end self-test (unweb --selftest, src/selftest.rs) runs the real app on the OS's own engine against a local site (127.0.0.1) and a third party (localhost); the servers' request logs (Cookie headers, revalidation headers) and the pages' own reports are the ground truth. Every check that something is absent has a control that shows the same thing present in the stranger that left it. Record results with scripts/record-results.sh, which replaces test/results/latest/<os>; git history keeps earlier runs. Every number on the site comes from recorded results with its conditions.

CI runs on every push to main on Linux, macOS and Windows: rustfmt, clippy with warnings denied, all tests, and the self-test (under Xvfb on Linux), uploading the results and posting the self-test lines as annotations. Toolchain pinned to Rust 1.97.0. Before calling a release complete, get an independent review from a separate agent that has not seen the implementation, fix findings, rerun checks and record again.

Releases go out through an automated workflow after CI succeeds on the exact commit, building installers for Windows (.msi), macOS (.dmg) and Linux (AppImage and .deb). Early versions ship unsigned, with install instructions on the site; signing certificates are YK's later decision. Never ask YK to publish a release. Version 0.0.x is never released.

## Site standard

photozeitgeist/unweb.org uses the unchanged Hugo Launch template: keep themes/ and archetypes/ byte for byte, content and configuration values only, no new layouts, CSS, _index.md files or public/ folder. Pages: about, contact (info@unweb.org), quick start, download and reference. Posts: launch, a plain-language explainer of what browsers remember about you, how it was tested and the name. Post front matter: quoted title in normal title case with no special characters, date only, draft: false, tags; no description or categories; never repeat the title in the body; featured_image plus the same image inline after the second or third paragraph when a post has one. Images drawn for the project live in static/images. YK connects the domain later.

## Development plan

Agreed with YK on October 23, 2026 (his plan text, sent the same day); YK approved sessions 1 and 2. Each later session needs YK's go-ahead. Pause and report after each session.

1. Foundation and isolation prototype: repository, license files, CI on three OSes, one throwaway store per stranger, wiped on close and checked, and a self-test proving nothing carries over on all three engines.
2. Version 0.1: strangers in separate windows, each fully isolated and wiped on close; address bar, back, forward and reload; the harder carry-over cases (HSTS, cache-based tracking, service workers left behind, the normal browser, leftover files on Windows); release workflow and installers.
3. Version 0.2: a side-by-side grid of 2 to 4 strangers with one shared address bar that sends the same address to all; a language setting per stranger where the engine allows it; a cookie counter, built on the starter copy of Watchkeeper's core ("this stranger was given 18 cookies by this site").
4. Version 0.3: a different location per stranger through a proxy the user supplies; saved session setups; a screenshot strip comparing all strangers.
5. Site: unweb.org on the Launch template, numbers from recorded results.

## Where we are

October 23, 2026, session 1 (foundation and isolation prototype) done, at YK's word "Unweb.org: go ahead" and "Go":

- Repository: Cargo workspace (version 0.0.1, never released), crates/unweb (Tauri 2.12, no unstable feature), LICENSE, NOTICE, THIRD_PARTY_LICENSES.md, CI on three OSes (copied from Watchkeeper's), scripts/record-results.sh and scripts/third-party-licenses.py. No address bar yet: a stranger opens at a start page or the address given on the command line.
- Strangers and wiping as described in Design. Linux needed the engine clear plus settle, delete and watch, because WebKitGTK writes cache records into a closed stranger's folder seconds later.
- Self-test, 7 checks: A leaves every kind of trace (server and script cookies, localStorage, IndexedDB, Cache API, a service worker, an HTTP-cached file, a frame on another site, a new WebGL shader); A's control finds them; B open at the same time and C opened after A closed find nothing and send no cookie; A's folder is deleted and stays deleted; nothing is left in Unweb's folders; on Linux the shared GPU shader cache is unchanged. On macOS WKWebView keeps no HTTP cache between a stranger's pages, so the cache control is waived there and the detail says so. No engine lets a cross-site frame's cookie be seen again even by A, so it is reported as not judged.
- Passes on Linux (WebKitGTK 2.52.6, local, recorded in test/results/latest/linux) and in CI on ubuntu-latest, macos-latest and windows-latest (WebView2 155) at afedd24.
- Independent review in two rounds by a reviewer agent that had not seen the implementation, with probe pages against a scratch copy. Round 1: isolation of every storage type and the wipes held; found the shared Mesa shader cache readable across strangers by timing, a second copy of Unweb deleting the first one's strangers, a frame cookie check that could never fail, HTTP sign-in passwords able to reach the keyring, and forced closes skipping the wipe. Round 2 confirmed all fixed (the shader check fails with the fix removed) and found a start-up race in the run lock, fixed by locking before the file is visible. Stated gaps: driver shader caches on Windows and macOS, forced closes (cleaned at next start).
- YK's decision on the core recorded under Origin.

## Next

Session 2 (version 0.1) approved by YK ("Go", October 23, 2026) and in progress. Nothing beyond it approved yet.
