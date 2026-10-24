//! Screenshots of strangers for the compare strip.
//!
//! Each engine is asked for what a page shows right now, as a PNG:
//! WebKitGTK's snapshot, WebView2's CapturePreview and WKWebView's
//! takeSnapshot. The engines answer on the main thread; this waits for the
//! answer, so call it off the main thread.

use std::sync::mpsc;
use std::time::Duration;

/// What `page` shows now, as PNG bytes.
pub fn capture(page: &tauri::Webview) -> Result<Vec<u8>, String> {
    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
    ask(page, tx)?;
    let png = rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "the page didn't give a screenshot".to_string())??;
    if !png.starts_with(PNG_MAGIC) {
        return Err("the engine's screenshot isn't a PNG".into());
    }
    Ok(png)
}

pub const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

#[cfg(target_os = "linux")]
fn ask(page: &tauri::Webview, tx: mpsc::Sender<Result<Vec<u8>, String>>) -> Result<(), String> {
    page.with_webview(move |pv| {
        use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
        pv.inner().snapshot(
            SnapshotRegion::Visible,
            SnapshotOptions::NONE,
            None::<&gtk::gio::Cancellable>,
            move |r| {
                let out = r.map_err(|e| e.to_string()).and_then(|surface| {
                    let image = cairo::ImageSurface::try_from(surface)
                        .map_err(|_| "the screenshot isn't an image".to_string())?;
                    let mut buf = Vec::new();
                    image.write_to_png(&mut buf).map_err(|e| e.to_string())?;
                    Ok(buf)
                });
                let _ = tx.send(out);
            },
        );
    })
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
fn ask(page: &tauri::Webview, tx: mpsc::Sender<Result<Vec<u8>, String>>) -> Result<(), String> {
    page.with_webview(move |pv| unsafe {
        use webview2_com::CapturePreviewCompletedHandler;
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG;
        use windows::Win32::Foundation::HGLOBAL;
        use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
        let failed = tx.clone();
        let asked = (|| -> windows_core::Result<()> {
            let core = pv.controller().CoreWebView2()?;
            let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
            let written = stream.clone();
            core.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &CapturePreviewCompletedHandler::create(Box::new(move |r| {
                    let out = r
                        .map_err(|e| e.to_string())
                        .and_then(|()| stream_bytes(&written).map_err(|e| e.to_string()));
                    let _ = tx.send(out);
                    Ok(())
                })),
            )
        })();
        if let Err(e) = asked {
            let _ = failed.send(Err(e.to_string()));
        }
    })
    .map_err(|e| e.to_string())
}

/// Everything written to a memory stream so far.
#[cfg(target_os = "windows")]
unsafe fn stream_bytes(
    stream: &windows::Win32::System::Com::IStream,
) -> windows_core::Result<Vec<u8>> {
    use windows::Win32::System::Com::StructuredStorage::GetHGlobalFromStream;
    use windows::Win32::System::Com::STREAM_SEEK_CUR;
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    unsafe {
        // The stream's position is how much was written; the memory behind
        // it can be larger.
        let mut len = 0u64;
        stream.Seek(0, STREAM_SEEK_CUR, Some(&mut len))?;
        let global = GetHGlobalFromStream(stream)?;
        let size = GlobalSize(global).min(len as usize);
        let ptr = GlobalLock(global) as *const u8;
        if ptr.is_null() {
            return Err(windows_core::Error::from_thread());
        }
        let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(global);
        Ok(bytes)
    }
}

#[cfg(target_os = "macos")]
fn ask(page: &tauri::Webview, tx: mpsc::Sender<Result<Vec<u8>, String>>) -> Result<(), String> {
    page.with_webview(move |pv| unsafe {
        let ptr = pv.inner() as *const objc2_web_kit::WKWebView;
        let Some(wv) = ptr.as_ref() else {
            let _ = tx.send(Err("no page".into()));
            return;
        };
        let done = block2::RcBlock::new(
            move |image: *mut objc2_app_kit::NSImage, _error: *mut objc2_foundation::NSError| {
                let out = match image.as_ref() {
                    None => Err("the page didn't give a screenshot".to_string()),
                    Some(image) => png_of(image),
                };
                let _ = tx.send(out);
            },
        );
        wv.takeSnapshotWithConfiguration_completionHandler(None, &done);
    })
    .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn png_of(image: &objc2_app_kit::NSImage) -> Result<Vec<u8>, String> {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey};
    use objc2_foundation::NSDictionary;
    let tiff = image
        .TIFFRepresentation()
        .ok_or("the screenshot has no pixels")?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff).ok_or("the screenshot can't be read")?;
    let props = NSDictionary::<NSBitmapImageRepPropertyKey, objc2::runtime::AnyObject>::new();
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }
        .ok_or("the screenshot can't be made a PNG")?;
    Ok(png.to_vec())
}
