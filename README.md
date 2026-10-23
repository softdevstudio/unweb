# Unweb

A tiny browser where every visit is a first visit.

Every Unweb window is a stranger. It starts with no history, no logins and no cookies, and when you close it, everything it collected is deleted. You can open several strangers at once, and each one is kept apart from the others and from your normal browser.

Unweb uses the web engine your system already has: WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux.

## Status

Early. This is the foundation: each window gets a throwaway store of its own, and a self-test checks on all three systems that nothing carries over. There is no address bar yet and no installer; those come with version 0.1. To try it, build it from source and start it with an address: `unweb example.com`.

## How a stranger is kept apart

On Windows and Linux each stranger gets a data folder of its own, which the engine uses for cookies, storage, its cache, service workers and everything else a site can leave. No two strangers share a folder. On macOS each stranger gets its own WKWebView data store that keeps nothing on disk.

When a stranger closes, Unweb deletes its folder and checks that it stays deleted. On Linux it first tells the engine to clear the stranger's data, then waits until the engine has stopped writing to the folder (WebKitGTK writes a few cache records a few seconds after a page closes), deletes it, and watches it for ten seconds in case anything writes to it again. When the last stranger closes, Unweb waits for these deletions before it quits.

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

This runs the real app against two small local servers: a site at `127.0.0.1` and a third party at `localhost`, which counts as a different site. Both log every request with its cookies and whether it asked to revalidate a cached copy.

1. Stranger A opens a page that leaves everything a site can leave: a server cookie hidden from scripts, a script cookie, localStorage, IndexedDB, a Cache API entry, a service worker, a file in the HTTP cache, and a cookie and storage key from a frame on the other site.
2. A looks for all of it and must find it. This is the control, so the checks after it can't pass by accident.
3. Stranger B, open at the same time, looks for the same things and must find none of them. It must not send A's cookies, and it must fetch the cached file afresh.
4. A closes. Its data folder must be deleted and stay deleted.
5. Stranger C, opened after A closed, must find nothing.
6. Every stranger closes. No folder may be left, and no file may appear anywhere else in Unweb's own folders.
7. On Linux, the GPU shader caches every program shares must not have changed, even though A compiled a WebGL shader no one had compiled before. With Unweb's setting removed, this check fails.

A check only counts something as carried over where A's own control could see it. Every engine tested so far keeps a cookie set by a frame from another site away from that frame on the next page, even in A, so that cookie is reported as not judged.

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
- `scripts/third-party-licenses.py`: regenerates THIRD_PARTY_LICENSES.md.

Unweb is a sister project of [Watchkeeper](https://github.com/softdevstudio/watchkeeper), whose detection code it will reuse for its cookie counter.

## License

Apache License 2.0. See LICENSE, NOTICE and THIRD_PARTY_LICENSES.md.

Contact: info@unweb.org
