# Third Party Licenses

Unweb is built with the Rust crates listed below, grouped by license. The list is generated from Cargo.lock by scripts/third-party-licenses.py and covers every crate the app can link on Windows, macOS or Linux; a given build links only the ones for its platform. Each crate's full license text ships with its source on crates.io.

On each platform Unweb also uses the system's own web engine, which is not part of the download: WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux.

## MIT OR Apache-2.0

189 crates: android_system_properties 0.1.6, anyhow 1.0.104, base64 0.21.7, base64 0.23.1, bitflags 2.13.2, block-buffer 0.10.4, bumpalo 3.20.3, camino 1.2.6, cargo-platform 0.1.9, cfg-if 1.0.5, chrono 0.4.45, cookie 0.18.2, core-foundation 0.10.1, core-foundation-sys 0.8.7, core-graphics 0.25.0, core-graphics-types 0.2.0, cpufeatures 0.2.17, crc32fast 1.5.3, crossbeam-channel 0.5.17, crossbeam-utils 0.8.23, crypto-common 0.1.7, defmt 1.2.0, defmt-macros 1.2.0, defmt-parser 1.1.0, deranged 0.5.9, digest 0.10.7, dirs 7.0.0, dirs-sys 0.5.0, displaydoc 0.2.7, dtoa 1.0.11, dyn-clone 1.0.20, embed_plist 1.2.2, erased-serde 0.4.10, fdeflate 0.3.7, field-offset 0.3.6, flate2 1.1.10, form_urlencoded 1.2.2, futures-channel 0.3.34, futures-core 0.3.34, futures-executor 0.3.34, futures-io 0.3.34, futures-macro 0.3.34, futures-sink 0.3.34, futures-task 0.3.34, futures-util 0.3.34, getrandom 0.2.17, getrandom 0.3.4, getrandom 0.4.3, glob 0.3.4, hashbrown 0.12.3, hashbrown 0.17.1, heck 0.4.1, heck 0.5.0, hex 0.4.3, html5ever 0.39.0, http 1.5.0, httparse 1.10.1, iana-time-zone 0.1.65, iana-time-zone-haiku 0.1.2, idna 1.1.0, ipnet 2.12.2, itoa 1.0.18, jni-sys 0.3.1, jni-sys 0.4.1, jni-sys-macros 0.4.1, js-sys 0.3.106, jsonptr 0.7.1, keyboard-types 0.8.3, libc 0.2.191, lock_api 0.4.14, log 0.4.34, markup5ever 0.39.0, mime 0.3.17, ndk 0.9.0, ndk-context 0.1.1, ndk-sys 0.6.0+11769913, num-conv 0.2.2, num-traits 0.2.19, once_cell 1.21.4, parking_lot 0.12.5, parking_lot_core 0.9.12, percent-encoding 2.3.2, png 0.18.1, powerfmt 0.2.1, proc-macro-crate 1.3.1, proc-macro-crate 2.0.2, proc-macro-crate 3.5.0, proc-macro-error 1.0.4, proc-macro-error-attr 1.0.4, proc-macro2 1.0.107, quote 1.0.47, ref-cast 1.0.27, ref-cast-impl 1.0.27, regex 1.13.1, regex-automata 0.4.18, regex-syntax 0.8.11, reqwest 0.13.5, rustls-pki-types 1.15.1, rustversion 1.0.23, scopeguard 1.2.0, semver 1.0.28, serde 1.0.229, serde-untagged 0.1.9, serde_core 1.0.229, serde_derive 1.0.229, serde_derive_internals 0.29.1, serde_json 1.0.154, serde_repr 0.1.21, serde_spanned 0.6.9, serde_spanned 1.1.2, serde_with 3.24.0, serde_with_macros 3.24.0, serialize-to-javascript 0.1.2, serialize-to-javascript-impl 0.1.2, servo_arc 0.4.3, sha2 0.10.9, siphasher 1.0.4, smallvec 1.16.3, socket2 0.6.5, softbuffer 0.4.8, stable_deref_trait 1.2.1, string_cache 0.9.0, swift-rs 1.0.9, syn 1.0.109, syn 2.0.119, syn 3.0.9, tao-macros 0.1.4, tendril 0.5.1, thiserror 1.0.69, thiserror 2.0.21, thiserror-impl 1.0.69, thiserror-impl 2.0.21, time 0.3.55, time-core 0.1.9, time-macros 0.2.32, toml 1.1.8+spec-1.1.0, toml_datetime 0.6.3, toml_datetime 1.1.2+spec-1.1.0, toml_edit 0.19.15, toml_edit 0.20.2, toml_edit 0.25.17+spec-1.1.0, toml_parser 1.1.5+spec-1.1.0, toml_writer 1.1.3+spec-1.1.0, tray-icon 0.25.1, typeid 1.0.3, typenum 1.20.1, unicode-segmentation 1.13.3, url 2.5.8, wasm-bindgen 0.2.129, wasm-bindgen-futures 0.4.79, wasm-bindgen-macro 0.2.129, wasm-bindgen-macro-support 0.2.129, wasm-bindgen-shared 0.2.129, wasm-streams 0.5.0, web-sys 0.3.106, web-time 1.1.0, web_atoms 0.2.6, windows 0.62.2, windows-collections 0.3.2, windows-core 0.62.2, windows-future 0.3.2, windows-implement 0.60.2, windows-interface 0.59.3, windows-link 0.2.1, windows-numerics 0.3.1, windows-result 0.4.1, windows-strings 0.5.1, windows-sys 0.45.0, windows-sys 0.52.0, windows-sys 0.61.2, windows-targets 0.42.2, windows-targets 0.52.6, windows-threading 0.2.1, windows-version 0.1.7, windows_aarch64_gnullvm 0.42.2, windows_aarch64_gnullvm 0.52.6, windows_aarch64_msvc 0.42.2, windows_aarch64_msvc 0.52.6, windows_i686_gnu 0.42.2, windows_i686_gnu 0.52.6, windows_i686_gnullvm 0.52.6, windows_i686_msvc 0.42.2, windows_i686_msvc 0.52.6, windows_x86_64_gnu 0.42.2, windows_x86_64_gnu 0.52.6, windows_x86_64_gnullvm 0.42.2, windows_x86_64_gnullvm 0.52.6, windows_x86_64_msvc 0.42.2, windows_x86_64_msvc 0.52.6.

## MIT

91 crates: atk 0.18.2, atk-sys 0.18.2, block2 0.6.2, bytes 1.12.1, cairo-rs 0.18.5, cairo-sys-rs 0.18.2, cargo_metadata 0.19.2, cfb 0.14.0, combine 4.6.9, darling 0.24.1, darling_core 0.24.1, darling_macro 0.24.1, derive_more 2.1.1, derive_more-impl 2.1.1, dlopen2 0.8.2, dlopen2_derive 0.4.3, dom_query 0.28.0, gdk 0.18.2, gdk-pixbuf 0.18.5, gdk-pixbuf-sys 0.18.0, gdk-sys 0.18.2, gdkwayland-sys 0.18.2, gdkx11 0.18.2, gdkx11-sys 0.18.2, generic-array 0.14.7, gio 0.18.4, gio-sys 0.18.1, glib 0.18.5, glib-macros 0.18.5, glib-sys 0.18.1, gobject-sys 0.18.0, gtk 0.18.2, gtk-sys 0.18.2, gtk3-macros 0.18.2, http-body 1.1.0, http-body-util 0.1.5, hyper 1.12.0, hyper-util 0.1.21, ico 0.6.0, infer 0.22.0, javascriptcore-rs 1.1.2, javascriptcore-rs-sys 1.1.1, libredox 0.1.25, memoffset 0.9.1, mio 1.2.4, new_debug_unreachable 1.0.6, objc2 0.6.5, objc2-encode 4.1.0, objc2-foundation 0.3.2, pango 0.18.3, pango-sys 0.18.0, phf 0.13.1, phf_generator 0.13.1, phf_macros 0.13.1, phf_shared 0.13.1, plist 1.10.3, precomputed-hash 0.1.1, quick-xml 0.42.0, redox_syscall 0.5.18, redox_users 0.5.3, schemars 0.8.22, schemars 0.9.0, schemars 1.2.3, schemars_derive 0.8.22, simd-adler32 0.3.10, slab 0.4.12, soup3 0.5.0, soup3-sys 0.5.0, strsim 0.11.1, synstructure 0.14.0, tokio 1.54.1, tokio-util 0.7.20, tower 0.5.3, tower-http 0.6.11, tower-layer 0.3.3, tower-service 0.3.3, tracing 0.1.44, tracing-core 0.1.36, try-lock 0.2.5, urlpattern 0.6.0, want 0.3.2, webkit2gtk 2.0.2, webkit2gtk-sys 2.0.2, webview2-com 0.39.1, webview2-com-macros 0.8.1, webview2-com-sys 0.39.1, winnow 0.5.40, winnow 1.0.4, x11 2.21.0, x11-dl 2.21.0, zmij 1.0.23.

## Apache-2.0 OR MIT

27 crates: atomic-waker 1.1.2, bit-set 0.8.0, bit-vec 0.8.0, ctor 1.0.13, equivalent 1.0.3, fastrand 2.5.0, idna_adapter 1.2.2, indexmap 1.9.3, indexmap 2.14.2, libappindicator 0.9.0, libappindicator-sys 0.9.0, muda 0.20.0, pin-project-lite 0.2.17, portable-atomic 1.15.0, portable-atomic-util 0.2.8, rustc-hash 2.1.3, tauri 2.12.5, tauri-codegen 2.7.4, tauri-macros 2.7.4, tauri-runtime 2.12.2, tauri-runtime-wry 2.12.3, tauri-utils 2.10.2, utf8_iter 1.0.4, uuid 1.29.0, window-vibrancy 0.8.1, wry 0.57.1, zeroize 1.9.1.

## Zlib OR Apache-2.0 OR MIT

20 crates: bytemuck 1.25.2, dispatch2 0.3.1, objc2-app-kit 0.3.2, objc2-cloud-kit 0.3.2, objc2-core-data 0.3.2, objc2-core-foundation 0.3.2, objc2-core-graphics 0.3.2, objc2-core-image 0.3.2, objc2-core-location 0.3.2, objc2-core-text 0.3.2, objc2-core-video 0.3.2, objc2-exception-helper 0.1.1, objc2-io-surface 0.3.2, objc2-javascript-core 0.3.2, objc2-quartz-core 0.3.2, objc2-security 0.3.2, objc2-ui-kit 0.3.2, objc2-user-notifications 0.3.2, objc2-web-kit 0.3.2, tinyvec 1.13.3.

## Unicode-3.0

18 crates: icu_collections 2.4.0, icu_locale_core 2.4.0, icu_normalizer 2.4.0, icu_normalizer_data 2.4.0, icu_properties 2.4.0, icu_properties_data 2.4.0, icu_provider 2.4.0, litemap 0.8.3, potential_utf 0.1.6, tinystr 0.8.5, writeable 0.6.4, yoke 0.8.4, yoke-derive 0.8.4, zerofrom 0.1.8, zerofrom-derive 0.1.8, zerotrie 0.2.5, zerovec 0.11.9, zerovec-derive 0.11.7.

## MIT/Apache-2.0

13 crates: bitflags 1.3.2, bs58 0.5.1, foreign-types 0.5.0, foreign-types-macros 0.2.4, foreign-types-shared 0.3.1, ident_case 1.0.1, jni 0.21.1, json-patch 4.2.0, psl 2.1.245, psl-types 2.0.11, winapi 0.3.9, winapi-i686-pc-windows-gnu 0.4.0, winapi-x86_64-pc-windows-gnu 0.4.0.

## Unlicense OR MIT

9 crates: aho-corasick 1.1.5, byteorder 1.5.0, jiff 0.2.38, jiff-core 0.1.1, jiff-static 0.2.38, jiff-tzdb 0.1.9, jiff-tzdb-platform 0.1.3, memchr 2.8.3, winapi-util 0.1.11.

## MPL-2.0

5 crates: cssparser 0.37.0, cssparser-macros 0.7.1, dtoa-short 0.3.5, option-ext 0.2.0, selectors 0.38.0.

## Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT

3 crates: wasi 0.11.1+wasi-snapshot-preview1, wasip2 1.0.4+wasi-0.2.12, wit-bindgen 0.57.1.

## Apache-2.0/MIT

3 crates: cesu8 1.1.0, dbus 0.9.12, libdbus-sys 0.2.7.

## BSD-3-Clause

3 crates: alloc-no-stdlib 3.0.0, alloc-stdlib 0.3.0, subtle 2.6.1.

## ISC

3 crates: libloading 0.7.4, rustls-webpki 0.103.16, untrusted 0.9.0.

## Apache-2.0

2 crates: sync_wrapper 1.0.2, tao 0.37.2.

## BSD-3-Clause OR MIT OR Apache-2.0

2 crates: num_enum 0.7.6, num_enum_derive 0.7.6.

## MIT OR Apache-2.0 OR LGPL-2.1-or-later

2 crates: r-efi 5.3.0, r-efi 6.0.0.

## MIT OR Zlib OR Apache-2.0

2 crates: miniz_oxide 0.8.9, miniz_oxide 0.9.1.

## Unlicense/MIT

2 crates: same-file 1.0.6, walkdir 2.5.0.

## Zlib

2 crates: foldhash 0.2.0, zlib-rs 0.6.8.

## (MIT OR Apache-2.0) AND Unicode-3.0

1 crates: unicode-ident 1.0.27.

## 0BSD OR MIT OR Apache-2.0

1 crates: adler2 2.0.1.

## Apache-2.0 / MIT

1 crates: fnv 1.0.7.

## Apache-2.0 AND ISC

1 crates: ring 0.17.14.

## Apache-2.0 AND MIT

1 crates: dpi 0.1.2.

## Apache-2.0 OR ISC OR MIT

1 crates: rustls 0.23.46.

## BSD-3-Clause AND MIT

1 crates: brotli 9.0.0.

## BSD-3-Clause/MIT

1 crates: brotli-decompressor 6.0.1.

## CC0-1.0 OR MIT-0 OR Apache-2.0

1 crates: dunce 1.0.5.

## MIT OR Apache-2.0 OR Zlib

1 crates: raw-window-handle 0.6.2.
