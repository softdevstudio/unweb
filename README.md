# Unweb

A tiny browser where every visit is a first visit.

Every stranger in Unweb starts with no history, no logins and no cookies, and when you close it, everything it collected is deleted. You can open several strangers at once, in separate windows or side by side in one, and each one is kept apart from the others and from your normal browser.

Unweb uses the web engine your system already has: WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux.

## Using it

Download the installer for your system from the [latest release](https://github.com/softdevstudio/unweb/releases/latest). The builds aren't signed yet: on Windows, SmartScreen warns you (choose More info, then Run anyway); on macOS, right-click the app and choose Open the first time. Each release lists checksums in `SHA256SUMS`.

Each window has a bar at the top:

- **Back, Forward and Reload.** These use the engine's own calls, so a page can't catch or block them.
- **The address field.** Type an address and press Enter. `example.com` opens `https://example.com/`. Only web addresses open; typing words doesn't send them to a search engine.
- **Side by side.** Adds another stranger to the window, next to the ones already there, up to four (three in a row, four as two by two). With several strangers, the address you type goes to all of them, and Back, Forward and Reload act on all of them. It's the quick way to see whether a site shows different strangers different things.
- **New stranger.** Opens another window with a new stranger in it.

Above each stranger is a thin strip:

- **Its cookie count**, such as "18 cookies from example.com, 3 from 2 other sites". A stranger starts with no cookies, so this is exactly what the sites gave this stranger since it opened. It counts the cookies in the stranger's store, including the ones scripts can't see, and is updated every two seconds and whenever a page loads. Only names and domains are read, never values.
- **Its language** (Windows only), such as Deutsch or Français. The site sees it in the stranger's requests and in `navigator.language`. A stranger gets its language before its first request, so changing it opens a new stranger in its place, at the same address, and closes the old one with everything it collected. On Linux and macOS the menu is off and every stranger uses the system language: WebKitGTK applies its language setting to every stranger at once (the self-test caught this), and WKWebView takes one language for the whole app.
- **×** closes that stranger and deletes what it collected; the others carry on. It shows when a window has more than one stranger.

Close a window and its strangers are gone, with everything they collected. You can also start Unweb with an address: `unweb example.com`.

A link that asks for a new window opens in the same stranger, so it stays in that stranger's store. Downloads are refused for now, because a downloaded file would outlive the stranger.

## How a stranger is kept apart

On Windows and Linux each stranger gets a data folder of its own, which the engine uses for cookies, storage, its cache, service workers and everything else a site can leave. No two strangers share a folder. On macOS each stranger gets its own WKWebView data store that keeps nothing on disk.

Strangers side by side in one window are kept apart exactly as strangers in separate windows: each has its own folder (or in-memory store on macOS). The bar has a small store of its own, deleted with the window, and each strip shares its stranger's. Neither keeps anything a site gave.

The bar and the strips are the only parts of a window that can ask Unweb to do anything, and each only for its own window or stranger; a web page can't call the app at all. Nothing but the bar itself can be loaded in the bar, and nothing but the strip in a strip, so a link dropped on them, or any other trick, can't put a web page in their place. Every window and stranger has a random internal name, which a page can read (it is how Tauri, the framework under Unweb, tells them apart), so a site can't tell which stranger of a run it is in.

When a stranger closes, Unweb deletes its folder and checks that it stays deleted. On Linux it first tells the engine to clear the stranger's data, then waits until the engine has stopped writing to the folder (WebKitGTK writes a few cache records a few seconds after a page closes), deletes it, and watches it for ten seconds in case anything writes to it again. When the last stranger closes, Unweb waits for these deletions before it quits. On macOS, where nothing is on disk, Unweb clears the stranger's store before the window goes, because WKWebView otherwise lets a closed stranger's service worker run on for a few seconds. Even then, a worker that was busy can make a call or two in the first second after the window goes; the self-test counts these and requires silence from two seconds on. By then the stranger's store is already empty, so such a call carries nothing the stranger collected.

Each running copy of Unweb keeps its strangers in a run folder of its own and holds a lock on it. If Unweb crashes or is killed, its lock is let go, and the next copy that starts deletes that run folder. A second copy started while the first is running leaves the first one's strangers alone.

Unweb also turns off what the engine would otherwise keep outside the stranger's folder: WebView2's GPU shader disk cache on Windows, Mesa's and NVIDIA's shared shader caches on Linux (a page could otherwise compile a shader in one stranger and find it again, by timing, in the next), and on Linux the offer to save sign-in passwords in the system keyring.

## What a stranger still gives away

- **Your IP address and rough location.** A site still sees where the connection comes from.
- **Your device.** All strangers on one machine share its screen size, fonts and hardware, so a site that fingerprints devices could link them.
- **It's not a sandbox for dangerous sites.** Your normal browser's data stays untouched, but a stranger runs on the same system engine as everything else.
- **Your graphics driver's own caches on Windows and macOS.** The driver can keep compiled shaders in a cache of its own (DirectX's shader cache, Apple's Metal cache), shared by every program. Unweb can't turn those off, so in theory a page could recognize, by timing, that a shader was compiled on this machine before.
- **A stranger closed by force.** Killing Unweb skips the careful wipe; whatever is left is deleted the next time Unweb starts. A page that closes itself with `window.close()` behaves differently per system: on Linux the page goes and the window stays with its bar, and closing that window wipes the stranger as usual; on Windows and macOS see the next start.
- **Memory on Linux.** Each stranger has its own network process in WebKitGTK, and because of a limitation in Tauri (issue 14626) a closed stranger's process stays in memory, about 55 MB, until Unweb quits. Its data is already cleared and its folder deleted. If you open and close many strangers, restart Unweb now and then.
- **Being in Unweb.** A page can tell it runs inside a Tauri app, because Tauri adds a small script to every page (its calls are refused).

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
7. The bar on C: it shows the page's address; typing an address, Back, Forward and Reload work, even on a page that replaced `history.back`, `history.go` and the popstate event; nothing can load another page in the bar; a page's calls to the app are refused, and the names a page can read are random; `window.open` opens in the same stranger with no new window; the bar and strip keep their heights and the page fills the rest, before and after a resize; New stranger opens a stranger with a store of its own.
8. A window with three strangers side by side, asked for German, French and the system language. Each is given four cookies by the site (two by the server, one of them hidden from scripts, and two by the page), and each strip must count exactly four. One address typed in the bar must reach all three, and each must send back only the ID cookie the site gave it. On Windows each must send its own language and its page must see it; changing the third one's language must put a new stranger in its place, at the same address, with no earlier cookie and the new language, and delete the old one. On Linux and macOS the language menu must be off and no stranger may have been given the languages asked for. Closing one of the three must delete it and leave the others as they were.
9. Every window closes. No folder may be left, and no file may appear anywhere else in Unweb's own folders.
10. On Linux, the GPU shader caches every program shares must not have changed, even though A compiled a new shader. With Unweb's setting removed, this check fails.
11. The normal browser's folders (Edge's profile on Windows, Safari's web data on macOS, Epiphany, Firefox and Chrome profiles on Linux) and the places an engine keeps data when an app names no folder must not change.

The HSTS check needs a certificate the engine trusts. `scripts/test-ca.sh` makes a throwaway one and trusts it system-wide, so it is only for test machines; CI runs it on its throwaway runners. Without it the HSTS check reports "not run" (and fails in CI).

A check only counts something as carried over where A's own control could see it. Every engine tested so far keeps a cookie set by a frame from another site away from that frame on the next page, even in A, so that cookie is reported as not judged. On macOS, WKWebView keeps no HTTP cache from one of a stranger's pages to the next, so the cache and ETag controls can't show anything there and the detail says so.

Results are written to `results-dir/selftest.json`; the exit code is 0 only if every check passed. A check that couldn't run here (HSTS without its certificate) is printed as SKIP and counts as a failure in CI. `scripts/record-results.sh` also checks, after Unweb has exited, that none of the folders it or its engine could use is left, not even empty. The release workflow runs the self-test on what people install: the installed .deb, the AppImage, the app from the .dmg and the program installed by the .msi. CI runs it on Windows, macOS and Linux on every push. Results recorded on the development machine (Linux) are in [test/results/latest](test/results/latest).

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
- `crates/unweb-core`: what is counted for each stranger (sites, cookies), with no window code.
- `scripts/record-results.sh`: runs every check and records the results.
- `scripts/test-ca.sh`: sets up the HSTS check's test certificate (test machines only).
- `.github/workflows/release.yml`: builds, self-tests and publishes the installers for a new version.
- `scripts/third-party-licenses.py`: regenerates THIRD_PARTY_LICENSES.md.

`crates/unweb-core` started as a one-time copy of the core of [Watchkeeper](https://github.com/softdevstudio/watchkeeper), Apache 2.0, trimmed to the site logic; the cookie counter is Unweb's own, and the two projects don't share code from here on.

## License

Apache License 2.0. See LICENSE, NOTICE and THIRD_PARTY_LICENSES.md.

Contact: info@unweb.org
