# Unweb

A tiny browser where every visit is a first visit.

Every Unweb window is a stranger. It starts with no history, no logins and no cookies, and when you close it, everything it collected is deleted. You can open several strangers at once, and each one is kept apart from the others and from your normal browser.

Unweb uses the web engine your system already has: WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux.

## Using it

Download the installer for your system from the [latest release](https://github.com/softdevstudio/unweb/releases/latest). The builds aren't signed yet: on Windows, SmartScreen warns you (choose More info, then Run anyway); on macOS, right-click the app and choose Open the first time. Each release lists checksums in `SHA256SUMS`.

Each stranger window has a bar at the top:

- **Back, Forward and Reload.** These use the engine's own calls, so a page can't catch or block them.
- **The address field.** Type an address and press Enter. `example.com` opens `https://example.com/`. Only web addresses open; typing words doesn't send them to a search engine.
- **New stranger.** Opens another window with a store of its own. Open as many as you like; they share nothing.

Close a window and that stranger is gone, with everything it collected. You can also start Unweb with an address: `unweb example.com`.

A link that asks for a new window opens in the same stranger, so it stays in that stranger's store. Downloads are refused for now, because a downloaded file would outlive the stranger.

## How a stranger is kept apart

On Windows and Linux each stranger gets a data folder of its own, which the engine uses for cookies, storage, its cache, service workers and everything else a site can leave. No two strangers share a folder. On macOS each stranger gets its own WKWebView data store that keeps nothing on disk.

The bar is part of the same window and uses the same store, so it leaves nothing behind either. It is the only part of the window that can ask Unweb to do anything; a web page can't call the app at all.

When a stranger closes, Unweb deletes its folder and checks that it stays deleted. On Linux it first tells the engine to clear the stranger's data, then waits until the engine has stopped writing to the folder (WebKitGTK writes a few cache records a few seconds after a page closes), deletes it, and watches it for ten seconds in case anything writes to it again. When the last stranger closes, Unweb waits for these deletions before it quits. On macOS, where nothing is on disk, Unweb clears the stranger's store before the window goes, because WKWebView otherwise lets a closed stranger's service worker run on for a few seconds. Even then, a worker that was busy can make a call or two in the first second after the window goes; the self-test counts these and requires silence from two seconds on. By then the stranger's store is already empty, so such a call carries nothing the stranger collected.

Each running copy of Unweb keeps its strangers in a run folder of its own and holds a lock on it. If Unweb crashes or is killed, its lock is let go, and the next copy that starts deletes that run folder. A second copy started while the first is running leaves the first one's strangers alone.

Unweb also turns off what the engine would otherwise keep outside the stranger's folder: WebView2's GPU shader disk cache on Windows, Mesa's and NVIDIA's shared shader caches on Linux (a page could otherwise compile a shader in one stranger and find it again, by timing, in the next), and on Linux the offer to save sign-in passwords in the system keyring.

## What a stranger still gives away

- **Your IP address and rough location.** A site still sees where the connection comes from.
- **Your device.** All strangers on one machine share its screen size, fonts and hardware, so a site that fingerprints devices could link them.
- **It's not a sandbox for dangerous sites.** Your normal browser's data stays untouched, but a stranger runs on the same system engine as everything else.
- **Your graphics driver's own caches on Windows and macOS.** The driver can keep compiled shaders in a cache of its own (DirectX's shader cache, Apple's Metal cache), shared by every program. Unweb can't turn those off, so in theory a page could recognize, by timing, that a shader was compiled on this machine before.
- **A stranger closed by force.** A page that closes its own window, or killing Unweb, skips the careful wipe. Whatever is left is deleted the next time Unweb starts.

## Self-test

```
unweb --selftest results-dir
```

This runs the real app against small local servers: a site at `127.0.0.1`, a third party at `localhost` (which counts as a different site) and, when a test certificate is set up, an HTTPS site at `hsts.unweb.test`. The servers log every request with its cookies, whether it asked to revalidate a cached copy and any ETag it sent back.

1. Stranger A opens a page that leaves everything a site can leave: a server cookie hidden from scripts, a script cookie, localStorage, IndexedDB, a Cache API entry, a service worker, a file in the HTTP cache, a file whose ETag works as an ID, a cookie and storage key from a frame on the other site, and a WebGL shader no one has compiled before.
2. A looks for all of it and must find it, and sends its ETag back. This is the control, so the checks after it can't pass by accident.
3. A visits the HTTPS site, which tells it to use HTTPS from now on (HSTS). A's next plain `http://` visit to that site must arrive as HTTPS: the control for the HSTS check.
4. Stranger B, open at the same time, looks for the same things and must find none of them. It must not send A's cookies or ETag, it must fetch the cached file afresh, and its plain visit to the HTTPS site must stay plain.
5. A closes. Its data folder must be deleted and stay deleted, and its service worker, which calls home twice a second, must be silent from two seconds after the window is gone (calls in those first two seconds are counted in the result).
6. Stranger C, opened after A closed, must find nothing, and its plain visit must stay plain too.
7. The bar on C: it shows the page's address; typing an address, Back, Forward and Reload work, even on a page that replaced `history.back`, `history.go` and the popstate event; a page's calls to the app are refused; `window.open` opens in the same stranger with no new window; the bar keeps its height and the page fills the rest, before and after a resize; New stranger opens a stranger with a store of its own.
8. Every stranger closes. No folder may be left, and no file may appear anywhere else in Unweb's own folders.
9. On Linux, the GPU shader caches every program shares must not have changed, even though A compiled a new shader. With Unweb's setting removed, this check fails.
10. The normal browser's folders (Edge's profile on Windows, Safari's web data on macOS, Epiphany, Firefox and Chrome profiles on Linux) and the places an engine keeps data when an app names no folder must not change.

The HSTS check needs a certificate the engine trusts. `scripts/test-ca.sh` makes a throwaway one and trusts it system-wide, so it is only for test machines; CI runs it on its throwaway runners. Without it the HSTS check reports "not run" (and fails in CI).

A check only counts something as carried over where A's own control could see it. Every engine tested so far keeps a cookie set by a frame from another site away from that frame on the next page, even in A, so that cookie is reported as not judged. On macOS, WKWebView keeps no HTTP cache from one of a stranger's pages to the next, so the cache and ETag controls can't show anything there and the detail says so.

Results are written to `results-dir/selftest.json`; the exit code is 0 only if every check passed. CI runs it on Windows, macOS and Linux on every push. Results recorded on the development machine (Linux) are in [test/results/latest](test/results/latest).

## Build from source

You need Rust 1.90 or newer.

- **Windows:** the WebView2 runtime (built into Windows 10 and 11).
- **macOS:** Xcode command line tools.
- **Linux:** WebKitGTK 4.1 and GTK 3 development packages. On Debian or Ubuntu:

  ```
  sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
  ```

Then:

```
cargo build --release
./target/release/unweb example.com
```

## Repository

- `crates/unweb`: the app, built with Tauri 2.
- `scripts/record-results.sh`: runs every check and records the results.
- `scripts/test-ca.sh`: sets up the HSTS check's test certificate (test machines only).
- `.github/workflows/release.yml`: builds, self-tests and publishes the installers for a new version.
- `scripts/third-party-licenses.py`: regenerates THIRD_PARTY_LICENSES.md.

Unweb's cookie counter (planned for 0.2) starts from a copy of the detection code in [Watchkeeper](https://github.com/softdevstudio/watchkeeper), Apache 2.0; from there it is Unweb's own code.

## License

Apache License 2.0. See LICENSE, NOTICE and THIRD_PARTY_LICENSES.md.

Contact: info@unweb.org
